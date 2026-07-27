//! IMS Viewer — Tauri Rust 后端
//!
//! 暴露给前端 (app.js) 的命令：
//!   open_file(path)    — 打开文件，扫描文件夹，解析 IMS，返回完整载荷
//!   pick_file()        — 弹出原生文件选择对话框
//!   get_sub_image()    — 获取当前 IMS 文件的指定子图数据
//!   extract_image()    — 导出单张图片
//!   extract_all()      — 导出全部子图
//!   start_batch_extract() — 目录级批量解压 IMS

use base64::Engine;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::Emitter;
use tauri::Manager;

mod dedup;
mod ims;
mod ims_pack;

use ims::{
    file_type_label, is_ims_file, parse_ims, parse_ims_and_read_cover, read_sub_image, FilePayload,
    FolderEntry, SaveImagePayload, SubImagePayload,
};
use ims_pack::{
    compute_output_path, load_image_as_entry, pack_ims, resize_ims_file, save_ims_entries_atomic,
    scan_ims_files, scan_multiple_folders, scan_multiple_folders_with_options, upgrade_ims_file,
    BatchConfig, BatchExtractConfig, BatchResizeConfig, ImageGroup, ProgressPayload,
    ResizeOutcome, UpEntry,
};

/// IMS 解析缓存：避免对同一文件反复调用 parse_ims（读取索引区）。
/// 键为文件路径，值为最近一次解析结果。
pub struct ImsCache(pub Mutex<HashMap<String, ims::ImsFileInfo>>);

/// 文件夹扫描缓存：目录 → (扫描时的目录 mtime, 文件列表)
/// 每次命中时校验 mtime，目录有变更则自动重新扫描。
pub struct FolderScanState {
    cache: Mutex<HashMap<String, (SystemTime, Vec<FolderEntry>)>>,
}

impl FolderScanState {
    fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }
}

/// 缓存最大条目数。IMS 元数据很小（每条 ~KB 级），64 条约几十 KB。
const MAX_CACHE_ENTRIES: usize = 64;
static EXPORT_WRITE_LOCK: Mutex<()> = Mutex::new(());
static IMS_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 带缓存的 parse_ims：命中缓存则直接返回，否则解析并写入缓存。
/// 超过 MAX_CACHE_ENTRIES 时全部清空，防止无界增长。
fn cached_parse_ims(cache: &ImsCache, filepath: &str) -> Result<ims::ImsFileInfo, String> {
    {
        let map = cache.0.lock().map_err(|e| format!("锁缓存失败: {}", e))?;
        if let Some(info) = map.get(filepath) {
            return Ok(info.clone());
        }
    }
    let info = parse_ims(filepath)?;
    {
        let mut map = cache.0.lock().map_err(|e| format!("锁缓存失败: {}", e))?;
        if map.len() >= MAX_CACHE_ENTRIES {
            map.clear();
        }
        map.insert(filepath.to_string(), info.clone());
    }
    Ok(info)
}

// ============================================================
// 文件夹扫描管理
// ============================================================

/// 纯扫描：列文件名 + 排序，不调 metadata()
fn do_scan_dir(dir_path: &str) -> Vec<FolderEntry> {
    let mut entries: Vec<FolderEntry> = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(dir_path) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if filename.starts_with('.') {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            let path_str = path.to_string_lossy().to_string();
            let is_ims = is_ims_file(&path_str);
            if !is_ims && !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif") {
                continue;
            }
            entries.push(FolderEntry {
                filepath: path_str,
                filename,
                is_ims,
                file_size: 0,
            });
        }
    }
    entries.sort_by(|a, b| natord_compare(&a.filename, &b.filename));
    entries
}

fn natord_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let a_chars: Vec<char> = a.to_lowercase().chars().collect();
    let b_chars: Vec<char> = b.to_lowercase().chars().collect();
    let mut i = 0;
    let mut j = 0;
    while i < a_chars.len() && j < b_chars.len() {
        if a_chars[i].is_ascii_digit() && b_chars[j].is_ascii_digit() {
            let mut an = String::new();
            let mut bn = String::new();
            while i < a_chars.len() && a_chars[i].is_ascii_digit() {
                an.push(a_chars[i]);
                i += 1;
            }
            while j < b_chars.len() && b_chars[j].is_ascii_digit() {
                bn.push(b_chars[j]);
                j += 1;
            }
            match an
                .parse::<u64>()
                .unwrap_or(0)
                .cmp(&bn.parse::<u64>().unwrap_or(0))
            {
                std::cmp::Ordering::Equal => continue,
                o => return o,
            }
        } else {
            match a_chars[i].cmp(&b_chars[j]) {
                std::cmp::Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
                o => return o,
            }
        }
    }
    a_chars.len().cmp(&b_chars.len())
}

/// 获取文件夹列表。缓存命中直接返回，首次扫描并缓存。
fn get_or_scan_folder(parent_dir: &str, state: &FolderScanState) -> Vec<FolderEntry> {
    {
        let cache = state.cache.lock().unwrap();
        if let Some((_mtime, list)) = cache.get(parent_dir) {
            return list.clone();
        }
    }
    let list = do_scan_dir(parent_dir);
    let mtime = std::fs::metadata(parent_dir)
        .and_then(|m| m.modified())
        .ok();
    let _ = mtime.map(|mt| {
        state
            .cache
            .lock()
            .unwrap()
            .insert(parent_dir.to_string(), (mt, list.clone()))
    });
    list
}

/// 检查目录是否变更，如有变更则重新扫描并更新缓存。
/// 前端在切换文件前调用，确保导航列表始终反映最新的目录内容。
#[tauri::command]
fn refresh_folder(
    dir: String,
    state: tauri::State<'_, FolderScanState>,
) -> Result<Vec<FolderEntry>, String> {
    let current_mtime = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();

    // 检查 mtime 是否变更
    if let Some(mtime) = current_mtime {
        let cache = state.cache.lock().unwrap();
        if let Some((cached_mtime, list)) = cache.get(&dir) {
            if *cached_mtime == mtime {
                return Ok(list.clone()); // 未变更，直接返回
            }
        }
    }

    // 有变更或首次 → 重新扫描
    let list = do_scan_dir(&dir);
    if let Some(mtime) = std::fs::metadata(&dir).and_then(|m| m.modified()).ok() {
        state
            .cache
            .lock()
            .unwrap()
            .insert(dir, (mtime, list.clone()));
    }
    Ok(list)
}

// ============================================================
// Tauri Commands
// ============================================================

#[tauri::command]
fn open_file(
    path: String,
    cache: tauri::State<'_, ImsCache>,
    scan_state: tauri::State<'_, FolderScanState>,
) -> Result<FilePayload, String> {
    let filepath = Path::new(&path);

    if !filepath.exists() {
        return Err(format!("文件不存在: {}", path));
    }

    let filename = filepath
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path)
        .to_string();

    let file_size = std::fs::metadata(&path)
        .map_err(|e| format!("读取文件信息失败: {}", e))?
        .len();

    let is_ims = is_ims_file(&path);
    let file_type = file_type_label(&path);

    // 同步获取文件夹列表（只列文件名无 metadata），缓存自动复用
    let parent_dir = filepath
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".")
        .to_string();
    let folder_list = get_or_scan_folder(&parent_dir, &scan_state);
    let file_index = folder_list
        .iter()
        .position(|e| e.filepath == path)
        .unwrap_or(0);

    if is_ims {
        // ---- IMS 文件：一次打开完成解析 + 封面读取 ----
        let (ims_info, cover_bytes) = parse_ims_and_read_cover(&path)?;
        let cover_b64 = base64::engine::general_purpose::STANDARD.encode(&cover_bytes);

        // 写入缓存，后续 get_sub_image / extract 可复用
        {
            let mut map = cache.0.lock().map_err(|e| format!("锁缓存失败: {}", e))?;
            map.insert(path.clone(), ims_info.clone());
        }

        // 按需加载：仅封面或首张图在打开时编码，其余延迟到切换时加载
        let sub_images: Vec<SubImagePayload> = ims_info
            .images
            .iter()
            .map(|img| {
                let bytes_base64 = if img.is_cover || img.index == 0 {
                    cover_b64.clone()
                } else {
                    String::new() // 延迟加载
                };
                SubImagePayload {
                    index: img.index,
                    filename: img.filename.clone(),
                    format: img.format,
                    data_offset: img.data_offset,
                    data_length: img.data_length,
                    is_cover: img.is_cover,
                    bytes_base64,
                    hash: img.hash.clone(),
                }
            })
            .collect();

        Ok(FilePayload {
            folder_list,
            file_index,
            filename,
            is_ims: true,
            file_size,
            file_type,
            ims_info: Some(ims_info),
            sub_images,
        })
    } else {
        // ---- 普通图片文件 ----
        let raw_bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败: {}", e))?;

        // 推断格式
        let format: u8 = if path.to_lowercase().ends_with(".png") {
            1
        } else {
            0
        };
        let bytes_base64 = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

        let sub_images = vec![SubImagePayload {
            index: 0,
            filename: filename.clone(),
            format,
            data_offset: 0,
            data_length: raw_bytes.len() as u32,
            is_cover: true,
            bytes_base64,
            hash: None,
        }];

        Ok(FilePayload {
            folder_list,
            file_index,
            filename,
            is_ims: false,
            file_size,
            file_type,
            ims_info: None,
            sub_images,
        })
    }
}

/// 保存编辑后的 IMS 文件（原子写入：先写临时文件，成功后再替换）
///
/// 接收前端传回的 subImages 数组，写入 IMS v2 文件。
/// - 仅剩 1 张封面 → 存为普通图片
/// - 多张图片 → 构建完整 IMS v2（含 SHA-256 哈希）
/// - WebP/AVIF 自动转 PNG
#[tauri::command]
fn save_ims(
    path: String,
    images: Vec<SaveImagePayload>,
    cache: tauri::State<'_, ImsCache>,
) -> Result<(), String> {
    if images.is_empty() {
        return Err("图片列表为空".into());
    }

    let mut entries: Vec<UpEntry> = Vec::with_capacity(images.len());

    for img in &images {
        // base64 解码
        let raw_bytes = base64::engine::general_purpose::STANDARD
            .decode(&img.bytes_base64)
            .map_err(|e| format!("base64 解码失败: {}", e))?;

        // WebP/AVIF → 自动转 PNG
        let (final_bytes, final_format, final_filename) = if img.format == 2 || img.format == 3 {
            let decoded = image::load_from_memory(&raw_bytes).map_err(|e| {
                format!(
                    "解码图片失败: {} (格式: {})",
                    e,
                    if img.format == 2 { "WebP" } else { "AVIF" }
                )
            })?;
            let mut png_bytes = Vec::new();
            decoded
                .write_to(
                    &mut std::io::Cursor::new(&mut png_bytes),
                    image::ImageFormat::Png,
                )
                .map_err(|e| format!("PNG 编码失败: {}", e))?;
            let new_name = format!(
                "{}.png",
                Path::new(&img.filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("image")
            );
            (png_bytes, 1u8, new_name)
        } else {
            (raw_bytes, img.format, img.filename.clone())
        };

        entries.push(UpEntry {
            filename: final_filename,
            format: final_format,
            bytes: final_bytes,
            is_cover: img.is_cover,
        });
    }

    // 封面排第一位
    entries.sort_by_key(|e| !e.is_cover);

    let result = save_ims_entries_atomic(&path, &entries);

    // 保存成功后清除缓存，后续 open_file 会重新解析
    if result.is_ok() {
        if let Ok(mut map) = cache.0.lock() {
            map.remove(&path);
        }
    }

    result
}

/// 弹出原生文件选择对话框，返回选中的文件路径
#[tauri::command]
async fn pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let result = app
        .dialog()
        .file()
        .add_filter(
            "图片文件",
            &[
                "jpg", "jpeg", "png", "webp", "avif", "ims", "ims.jpg", "ims.jpeg", "ims.png",
                "ims.webp", "ims.avif",
            ],
        )
        .add_filter(
            "IMS 文件",
            &[
                "ims", "ims.jpg", "ims.jpeg", "ims.png", "ims.webp", "ims.avif",
            ],
        )
        .add_filter("普通图片", &["jpg", "jpeg", "png", "webp", "avif"])
        .add_filter("所有文件", &["*"])
        .blocking_pick_file();

    Ok(result.map(|p| p.to_string()))
}

/// 弹出另存为对话框
#[tauri::command]
async fn pick_save_path(
    app: tauri::AppHandle,
    default_name: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let result = app
        .dialog()
        .file()
        .add_filter("IMS 文件", &["ims.jpg", "ims.png"])
        .add_filter("所有文件", &["*"])
        .set_file_name(default_name)
        .blocking_save_file();

    Ok(result.map(|p| p.to_string()))
}

/// 弹出文件夹选择对话框
#[tauri::command]
async fn pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let result = app.dialog().file().blocking_pick_folder();

    Ok(result.map(|p| p.to_string()))
}

/// 按需获取 IMS 文件中指定索引的子图数据（base64）
/// 使用缓存避免对同一文件反复解析 IMS 索引。
#[tauri::command]
fn get_sub_image(
    path: String,
    index: usize,
    cache: tauri::State<'_, ImsCache>,
) -> Result<SubImagePayload, String> {
    if !is_ims_file(&path) {
        return Err("非 IMS 文件，无法获取子图".into());
    }

    let ims_info = cached_parse_ims(&cache, &path)?;

    let img = ims_info
        .images
        .get(index)
        .ok_or_else(|| format!("子图索引越界: {} (共 {} 张)", index, ims_info.image_count))?;

    let bytes = read_sub_image(&path, img.data_offset, img.data_length)?;
    let bytes_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(SubImagePayload {
        index: img.index,
        filename: img.filename.clone(),
        format: img.format,
        data_offset: img.data_offset,
        data_length: img.data_length,
        is_cover: img.is_cover,
        bytes_base64,
        hash: img.hash.clone(),
    })
}

/// 导出单张图片——弹出保存对话框
#[tauri::command]
async fn extract_image(
    app: tauri::AppHandle,
    path: String,
    index: usize,
    cache: tauri::State<'_, ImsCache>,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let (bytes, default_name) = if is_ims_file(&path) {
        let ims_info = cached_parse_ims(&cache, &path)?;
        let img = ims_info
            .images
            .get(index)
            .ok_or_else(|| "子图索引越界".to_string())?;
        let data = read_sub_image(&path, img.data_offset, img.data_length)?;

        // 使用原始文件名，保留扩展名
        let ext = image_extension(img.format);
        let stem = Path::new(&img.filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");
        (data, format!("{}.{}", stem, ext))
    } else {
        let data = std::fs::read(&path).map_err(|e| format!("读取文件失败: {}", e))?;
        let name = Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("export.jpg");
        (data, name.to_string())
    };

    // 弹出保存对话框
    let save_path = app
        .dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter("JPEG 图片", &["jpg", "jpeg"])
        .add_filter("PNG 图片", &["png"])
        .blocking_save_file();

    if let Some(save_path) = save_path {
        let path_str = save_path.to_string();
        std::fs::write(Path::new(&path_str), &bytes).map_err(|e| format!("写入文件失败: {}", e))?;
        Ok(path_str)
    } else {
        Err("用户取消了保存".into())
    }
}

/// 导出 IMS 文件中的全部子图到指定目录
#[tauri::command]
async fn extract_all(
    app: tauri::AppHandle,
    path: String,
    cache: tauri::State<'_, ImsCache>,
) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    if !is_ims_file(&path) {
        return Err("非 IMS 文件，无法导出子图".into());
    }

    // 选择输出目录
    let dir = app.dialog().file().blocking_pick_folder();

    if dir.is_none() {
        return Err("用户取消了选择".into());
    }

    let out_dir = dir.unwrap().to_string();
    let ims_info = cached_parse_ims(&cache, &path)?;
    let mut saved_paths = Vec::new();

    for img in &ims_info.images {
        let bytes = read_sub_image(&path, img.data_offset, img.data_length)?;
        let out_name = output_image_filename(&img.filename, img.format, img.index);
        let out_path = write_export_image(&Path::new(&out_dir).join(out_name), &bytes)?;
        saved_paths.push(out_path.to_string_lossy().to_string());
    }

    Ok(saved_paths)
}

fn image_extension(format: u8) -> &'static str {
    match format {
        1 => "png",
        2 => "webp",
        3 => "avif",
        _ => "jpg",
    }
}

fn sanitize_path_component(raw: &str, fallback: &str) -> String {
    let mut name = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_string();

    if name.is_empty() {
        name = fallback.to_string();
    }

    let upper = name.to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if reserved.contains(&upper.as_str()) {
        name.push('_');
    }

    name
}

fn output_image_filename(filename: &str, format: u8, index: usize) -> String {
    let fallback = format!("image_{:05}", index + 1);
    let file_name = Path::new(filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&fallback);
    let safe_stem = sanitize_path_component(stem, &fallback);
    format!("{}.{}", safe_stem, image_extension(format))
}

fn md5_hex(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}

fn existing_file_md5(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err(format!("输出路径已存在且不是文件: {}", path.display()));
    }
    let bytes =
        std::fs::read(path).map_err(|e| format!("读取已有文件失败 {}: {}", path.display(), e))?;
    Ok(Some(md5_hex(&bytes)))
}

fn numbered_base_stem(stem: &str) -> &str {
    let Some(without_tail) = stem.strip_suffix('_') else {
        return stem;
    };
    let Some(pos) = without_tail.rfind('_') else {
        return stem;
    };
    let number = &without_tail[pos + 1..];
    if number.len() == 5 && number.chars().all(|c| c.is_ascii_digit()) {
        &without_tail[..pos]
    } else {
        stem
    }
}

fn numbered_export_path(base: &Path, number: u32) -> PathBuf {
    let parent = base.parent().unwrap_or_else(|| Path::new(""));
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = base.extension().and_then(|s| s.to_str()).unwrap_or("");
    let base_stem = numbered_base_stem(stem);
    let filename = if ext.is_empty() {
        format!("{}_{:05}_", base_stem, number)
    } else {
        format!("{}_{:05}_.{}", base_stem, number, ext)
    };
    parent.join(filename)
}

fn resolve_export_file_path(base: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let current_md5 = md5_hex(bytes);
    match existing_file_md5(base)? {
        None => return Ok(base.to_path_buf()),
        Some(existing_md5) if existing_md5 == current_md5 => return Ok(base.to_path_buf()),
        Some(_) => {}
    }

    for number in 1..=99_999 {
        let candidate = numbered_export_path(base, number);
        match existing_file_md5(&candidate)? {
            None => return Ok(candidate),
            Some(existing_md5) if existing_md5 == current_md5 => return Ok(candidate),
            Some(_) => continue,
        }
    }

    Err(format!(
        "没有可用编号: {} 已占用 _00001_ 到 _99999_",
        base.display()
    ))
}

fn write_export_image(base_path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let _guard = EXPORT_WRITE_LOCK
        .lock()
        .map_err(|e| format!("锁导出写入失败: {}", e))?;
    let out_path = resolve_export_file_path(base_path, bytes)?;
    if out_path.exists() {
        return Ok(out_path);
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建输出目录失败 {}: {}", parent.display(), e))?;
    }
    std::fs::write(&out_path, bytes)
        .map_err(|e| format!("写入失败 {}: {}", out_path.display(), e))?;
    Ok(out_path)
}

fn extract_ims_file_to_dir(filepath: &str, out_dir: &Path) -> Result<usize, String> {
    let ims_info = parse_ims(filepath)?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("创建输出目录失败 {}: {}", out_dir.display(), e))?;

    let mut saved = 0usize;
    for img in &ims_info.images {
        let bytes = read_sub_image(filepath, img.data_offset, img.data_length)?;
        let out_name = output_image_filename(&img.filename, img.format, img.index);
        write_export_image(&out_dir.join(out_name), &bytes)?;
        saved += 1;
    }

    Ok(saved)
}

fn source_path_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

fn ims_filename_key(filename: &str) -> String {
    filename.to_lowercase()
}

fn next_available_ims_filename(
    original_filename: &str,
    used_names: &HashSet<String>,
) -> Result<String, String> {
    if !used_names.contains(&ims_filename_key(original_filename)) {
        return Ok(original_filename.to_string());
    }

    let file_name = Path::new(original_filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(original_filename);
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ext = Path::new(file_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let base_stem = numbered_base_stem(stem);

    for number in 1..=99_999 {
        let candidate = if ext.is_empty() {
            format!("{}_{:05}_", base_stem, number)
        } else {
            format!("{}_{:05}_.{}", base_stem, number, ext)
        };
        if !used_names.contains(&ims_filename_key(&candidate)) {
            return Ok(candidate);
        }
    }

    Err(format!(
        "No available numbered filename for {} (_00001_ to _99999_ are occupied)",
        original_filename
    ))
}

fn push_unique_path(paths: &mut Vec<String>, used: &mut HashSet<String>, path: &str) {
    if used.insert(source_path_key(path)) {
        paths.push(path.to_string());
    }
}

fn existing_ims_candidates(base_name: &str) -> Vec<String> {
    vec![
        format!("{}.ims.jpg", base_name),
        format!("{}.ims.jpeg", base_name),
        format!("{}.ims.png", base_name),
        format!("{}.ims.webp", base_name),
        format!("{}.ims.avif", base_name),
        format!("{}.ims", base_name),
    ]
}

fn find_existing_ims_output_path(computed_path: &Path, base_name: &str) -> Option<PathBuf> {
    if computed_path.is_file() {
        return Some(computed_path.to_path_buf());
    }

    let parent = computed_path.parent()?;
    for filename in existing_ims_candidates(base_name) {
        let candidate = parent.join(filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn resolve_batch_convert_output_path(
    group: &ImageGroup,
    output_dir: &Option<String>,
    preserve_structure: bool,
    merge_existing_ims: bool,
) -> String {
    let computed = compute_output_path(group, output_dir, preserve_structure);
    if merge_existing_ims {
        if let Some(existing) =
            find_existing_ims_output_path(Path::new(&computed), &group.base_name)
        {
            return existing.to_string_lossy().to_string();
        }
    }
    computed
}

struct MergeSourceEntry {
    path: String,
    entry: UpEntry,
    md5: String,
    insert_candidate: bool,
}

struct MergeExistingResult {
    inserted: usize,
    skipped_existing: usize,
    matched_existing: usize,
    delete_files: Vec<String>,
}

fn merge_into_existing_ims(
    output_path: &str,
    match_paths: &[String],
    insert_paths: &[String],
    sub_images_to_jpeg: bool,
) -> Result<MergeExistingResult, String> {
    let _guard = IMS_WRITE_LOCK
        .lock()
        .map_err(|e| format!("Failed to lock IMS writer: {}", e))?;

    let ims_info = parse_ims(output_path)?;
    let mut entries: Vec<UpEntry> = Vec::with_capacity(ims_info.images.len() + insert_paths.len());
    let mut existing_md5 = HashSet::new();
    let mut used_names = HashSet::new();

    for img in &ims_info.images {
        let bytes = read_sub_image(output_path, img.data_offset, img.data_length)?;
        existing_md5.insert(md5_hex(&bytes));
        used_names.insert(ims_filename_key(&img.filename));
        entries.push(UpEntry {
            filename: img.filename.clone(),
            format: img.format,
            bytes,
            is_cover: img.is_cover,
        });
    }

    let insert_keys: HashSet<String> = insert_paths.iter().map(|p| source_path_key(p)).collect();
    let mut seen_sources = HashSet::new();
    let mut sources = Vec::new();

    for path in match_paths {
        if !seen_sources.insert(source_path_key(path)) {
            continue;
        }
        let is_insert = insert_keys.contains(&source_path_key(path));
        // 仅真正要插入的子图转 JPEG；用于 MD5 匹配的封面保持原字节（保证去重正确）
        let entry = load_image_as_entry(Path::new(path), false, is_insert && sub_images_to_jpeg)?;
        let md5 = md5_hex(&entry.bytes);
        sources.push(MergeSourceEntry {
            path: path.clone(),
            entry,
            md5,
            insert_candidate: is_insert,
        });
    }

    let matched_existing = sources
        .iter()
        .filter(|source| existing_md5.contains(&source.md5))
        .count();

    let mut inserted = 0usize;
    let mut skipped_existing = 0usize;
    let mut delete_files = Vec::new();
    let mut delete_keys = HashSet::new();

    for source in &sources {
        if existing_md5.contains(&source.md5) {
            push_unique_path(&mut delete_files, &mut delete_keys, &source.path);
        }
    }

    for source in sources {
        if !source.insert_candidate {
            continue;
        }
        if existing_md5.contains(&source.md5) {
            skipped_existing += 1;
            continue;
        }

        let mut entry = source.entry;
        entry.is_cover = false;
        entry.filename = next_available_ims_filename(&entry.filename, &used_names)?;

        existing_md5.insert(source.md5);
        used_names.insert(ims_filename_key(&entry.filename));
        push_unique_path(&mut delete_files, &mut delete_keys, &source.path);
        entries.push(entry);
        inserted += 1;
    }

    if inserted > 0 {
        save_ims_entries_atomic(output_path, &entries)?;
    }

    Ok(MergeExistingResult {
        inserted,
        skipped_existing,
        matched_existing,
        delete_files,
    })
}

/// 注册 imsViewer 到右键菜单"使用 imsViewer 打开"
#[tauri::command]
fn register_file_associations() -> Result<String, String> {
    let exe_path = std::env::current_exe().map_err(|e| format!("获取 exe 路径失败: {}", e))?;
    let exe_str = exe_path.to_string_lossy().to_string();
    let exe_cmd = format!("\"{}\" \"%1\"", exe_str);
    let ico = format!("\"{}\",0", exe_str);

    let ext_groups: &[(&str, &[&str])] = &[
        ("image", &["jpg", "jpeg", "png", "webp", "avif"]),
        ("imscontainer", &["ims", "ims.jpg", "ims.png"]),
    ];

    for (_group, exts) in ext_groups {
        for ext in *exts {
            // 为每个扩展名添加右键菜单项
            let shell_path = format!(
                "HKCU\\Software\\Classes\\SystemFileAssociations\\.{}\\shell\\OpenWithImsViewer",
                ext
            );
            // 菜单显示名
            let _ = std::process::Command::new("reg")
                .args(["add", &shell_path, "/ve", "/d", "使用 imsViewer 打开", "/f"])
                .output();
            // 图标
            let _ = std::process::Command::new("reg")
                .args(["add", &shell_path, "/v", "Icon", "/d", &ico, "/f"])
                .output();
            // 命令
            let cmd_path = format!("{}\\command", shell_path);
            let _ = std::process::Command::new("reg")
                .args(["add", &cmd_path, "/ve", "/d", &exe_cmd, "/f"])
                .output();
        }
    }

    // 刷新 shell
    let _ = std::process::Command::new("ie4uinit")
        .args(["-show"])
        .output();

    Ok("已添加到右键菜单".into())
}

struct BatchExtractTask {
    input_path: String,
    output_dir: PathBuf,
}

/// 批量解压：扫描目录中的 IMS 文件，逐个导出全部子图
#[tauri::command]
async fn start_batch_extract(
    app: tauri::AppHandle,
    config: BatchExtractConfig,
) -> Result<String, String> {
    let input_dir = std::fs::canonicalize(config.input_dir.trim())
        .map_err(|e| format!("输入目录无效: {}", e))?;
    if !input_dir.is_dir() {
        return Err(format!("输入目录不存在: {}", input_dir.display()));
    }

    if config.output_dir.trim().is_empty() {
        return Err("请选择输出目录".into());
    }
    let output_root = PathBuf::from(config.output_dir.trim());
    let depth = config.depth.min(100);
    let files = scan_ims_files(&input_dir.to_string_lossy(), depth);
    let total = files.len() as u32;

    if total == 0 {
        return Err("未发现 IMS 文件".into());
    }

    let input_base = input_dir.as_path();
    let mut tasks = Vec::with_capacity(files.len());
    for fp in files {
        let source_parent = Path::new(&fp).parent().unwrap_or(input_dir.as_path());
        let target_parent = if config.preserve_structure {
            let rel = source_parent
                .strip_prefix(input_base)
                .unwrap_or_else(|_| Path::new(""));
            output_root.join(rel)
        } else {
            output_root.clone()
        };
        tasks.push(BatchExtractTask {
            input_path: fp,
            output_dir: target_parent,
        });
    }

    let total_msg = format!("共发现 {} 个 IMS 文件", total);
    let _ = app.emit(
        "batch-progress",
        ProgressPayload {
            current: 0,
            total,
            message: total_msg.clone(),
            status: "running".into(),
        },
    );

    let app_handle = app.clone();
    let multi = config.multi_threaded;

    std::thread::spawn(move || {
        let result: Result<String, String> = if multi {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicU32, Ordering};

            let completed = AtomicU32::new(0);
            let failed = AtomicU32::new(0);

            tasks.par_iter().for_each(|task| {
                let fname = Path::new(&task.input_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&task.input_path);
                match extract_ims_file_to_dir(&task.input_path, &task.output_dir) {
                    Ok(count) => {
                        let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current,
                                total,
                                message: format!("已解压 {} ({} 张)", fname, count),
                                status: "running".into(),
                            },
                        );
                    }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::SeqCst);
                        let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current,
                                total,
                                message: format!("失败 {}: {}", fname, e),
                                status: "running".into(),
                            },
                        );
                    }
                }
            });

            let failures = failed.load(Ordering::SeqCst);
            if failures == 0 {
                Ok(format!("全部完成，共 {} 个 IMS 文件", total))
            } else {
                Err(format!(
                    "解压完成，成功 {} 个，失败 {} 个",
                    total - failures,
                    failures
                ))
            }
        } else {
            let mut completed = 0u32;
            let mut failed = 0u32;

            for task in &tasks {
                let fname = Path::new(&task.input_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&task.input_path);
                match extract_ims_file_to_dir(&task.input_path, &task.output_dir) {
                    Ok(count) => {
                        completed += 1;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current: completed,
                                total,
                                message: format!("已解压 {} ({} 张)", fname, count),
                                status: "running".into(),
                            },
                        );
                    }
                    Err(e) => {
                        completed += 1;
                        failed += 1;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current: completed,
                                total,
                                message: format!("失败 {}: {}", fname, e),
                                status: "running".into(),
                            },
                        );
                    }
                }
            }

            if failed == 0 {
                Ok(format!("全部完成，共 {} 个 IMS 文件", total))
            } else {
                Err(format!(
                    "解压完成，成功 {} 个，失败 {} 个",
                    total - failed,
                    failed
                ))
            }
        };

        match result {
            Ok(message) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message,
                        status: "completed".into(),
                    },
                );
            }
            Err(message) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message,
                        status: "error".into(),
                    },
                );
            }
        }
    });

    Ok(total_msg)
}

/// 批量升级：扫描 IMS 文件，升级到最新版本（v2 + SHA-256 hash）
#[tauri::command]
async fn start_batch_upgrade(
    app: tauri::AppHandle,
    path: String,
    depth: u32,
    multi_threaded: bool,
) -> Result<String, String> {
    let files = scan_ims_files(&path, depth);
    let total = files.len() as u32;

    if total == 0 {
        return Err("未发现 IMS 文件".into());
    }

    let total_msg = format!("共发现 {} 个 IMS 文件", total);
    let _ = app.emit(
        "batch-progress",
        ProgressPayload {
            current: 0,
            total,
            message: total_msg.clone(),
            status: "running".into(),
        },
    );

    let app_handle = app.clone();

    std::thread::spawn(move || {
        let result = if multi_threaded {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            files.par_iter().for_each(|fp| {
                let fname = Path::new(fp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(fp);
                match upgrade_ims_file(fp) {
                    Ok(()) => {
                        let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current,
                                total,
                                message: format!("已升级 {}", fname),
                                status: "running".into(),
                            },
                        );
                    }
                    Err(e) => {
                        has_error.store(true, Ordering::SeqCst);
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current: completed.load(Ordering::SeqCst),
                                total,
                                message: format!("失败 {}: {}", fname, e),
                                status: "running".into(),
                            },
                        );
                    }
                }
            });

            if has_error.load(Ordering::SeqCst) {
                Err("有文件升级失败".into())
            } else {
                Ok(())
            }
        } else {
            let mut completed: u32 = 0;
            for fp in &files {
                let fname = Path::new(fp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(fp);
                match upgrade_ims_file(fp) {
                    Ok(()) => {
                        completed += 1;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current: completed,
                                total,
                                message: format!("已升级 {}", fname),
                                status: "running".into(),
                            },
                        );
                    }
                    Err(e) => {
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current: completed,
                                total,
                                message: format!("失败 {}: {}", fname, e),
                                status: "running".into(),
                            },
                        );
                    }
                }
            }
            Ok(())
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: format!("全部完成，共 {} 个文件", total),
                        status: "completed".into(),
                    },
                );
            }
            Err(e) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: e,
                        status: "error".into(),
                    },
                );
            }
        }
    });

    Ok(total_msg)
}

/// 生成单文件调整结果的进度摘要（缩小 N 张 / 转 JPEG M 张）。
fn resize_summary(resized: usize, converted: usize) -> String {
    match (resized, converted) {
        (r, 0) if r > 0 => format!("缩小 {} 张子图", r),
        (0, c) if c > 0 => format!("转 JPEG {} 张子图", c),
        (r, c) if r > 0 && c > 0 => format!("缩小 {} 张, 转 JPEG {} 张", r, c),
        _ => "无变化".to_string(),
    }
}

#[tauri::command]
async fn start_batch_resize(
    app: tauri::AppHandle,
    config: BatchResizeConfig,
) -> Result<String, String> {
    if !config.enable_resize && !config.convert_to_jpeg {
        return Err("请至少勾选一个操作：调整分辨率或转为 JPEG".into());
    }
    if config.enable_resize && config.max_edge < 64 {
        return Err("最长边分辨率不能小于 64".into());
    }

    // 扫描所有文件夹的 IMS 文件并按路径去重（保留顺序）
    let mut files: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for folder in &config.folders {
        for fp in scan_ims_files(&folder.path, folder.depth) {
            if seen.insert(fp.clone()) {
                files.push(fp);
            }
        }
    }
    let total = files.len() as u32;

    if total == 0 {
        return Err("未发现 IMS 文件".into());
    }

    let total_msg = format!("共发现 {} 个 IMS 文件", total);
    let _ = app.emit(
        "batch-progress",
        ProgressPayload {
            current: 0,
            total,
            message: total_msg.clone(),
            status: "running".into(),
        },
    );

    let app_handle = app.clone();
    let max_edge = config.max_edge;
    let convert_to_jpeg = config.convert_to_jpeg;
    let enable_resize = config.enable_resize;

    std::thread::spawn(move || {
        let result = if config.multi_threaded {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            files.par_iter().for_each(|fp| {
                let fname = Path::new(fp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(fp);
                // 扫描阶段已按路径去重，每个文件是独立任务，无跨文件冲突，无需全局写锁
                let outcome = resize_ims_file(fp, max_edge, convert_to_jpeg, enable_resize);
                let message = match &outcome {
                    Ok(ResizeOutcome::Resized {
                        sub_images_resized,
                        sub_images_converted,
                    }) => format!(
                        "已调整 {} ({})",
                        fname,
                        resize_summary(*sub_images_resized, *sub_images_converted)
                    ),
                    Ok(ResizeOutcome::Skipped) => format!("跳过 {} (无需处理)", fname),
                    Err(e) => format!("失败 {}: {}", fname, e),
                };
                if outcome.is_err() {
                    has_error.store(true, Ordering::SeqCst);
                } else {
                    // 成功重写后失效 ImsCache
                    if let Some(cache) = app_handle.try_state::<ImsCache>() {
                        if let Ok(mut map) = cache.0.lock() {
                            map.remove(fp);
                        }
                    }
                }
                let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current,
                        total,
                        message,
                        status: "running".into(),
                    },
                );
            });

            if has_error.load(Ordering::SeqCst) {
                Err("有文件处理失败".into())
            } else {
                Ok(())
            }
        } else {
            let mut completed: u32 = 0;
            let mut has_error = false;
            for fp in &files {
                let fname = Path::new(fp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(fp);
                // 扫描阶段已按路径去重，每个文件是独立任务，无跨文件冲突，无需全局写锁
                let outcome = resize_ims_file(fp, max_edge, convert_to_jpeg, enable_resize);
                let message = match &outcome {
                    Ok(ResizeOutcome::Resized {
                        sub_images_resized,
                        sub_images_converted,
                    }) => format!(
                        "已调整 {} ({})",
                        fname,
                        resize_summary(*sub_images_resized, *sub_images_converted)
                    ),
                    Ok(ResizeOutcome::Skipped) => format!("跳过 {} (无需处理)", fname),
                    Err(e) => format!("失败 {}: {}", fname, e),
                };
                if outcome.is_err() {
                    has_error = true;
                } else if let Ok(ResizeOutcome::Resized { .. }) = &outcome {
                    if let Some(cache) = app_handle.try_state::<ImsCache>() {
                        if let Ok(mut map) = cache.0.lock() {
                            map.remove(fp);
                        }
                    }
                }
                completed += 1;
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: completed,
                        total,
                        message,
                        status: "running".into(),
                    },
                );
            }
            if has_error {
                Err("有文件处理失败".into())
            } else {
                Ok(())
            }
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: format!("全部完成，共 {} 个文件", total),
                        status: "completed".into(),
                    },
                );
            }
            Err(e) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: e,
                        status: "error".into(),
                    },
                );
            }
        }
    });

    Ok(total_msg)
}

/// 批量转换：扫描文件夹，按命名规范分组并打包为 IMS 文件
#[tauri::command]
async fn start_batch_convert(app: tauri::AppHandle, config: BatchConfig) -> Result<String, String> {
    // 1. 扫描所有文件夹，分组
    let groups = if config.merge_existing_ims {
        scan_multiple_folders_with_options(&config.folders, true)?
    } else {
        scan_multiple_folders(&config.folders)?
    };
    let total = groups.len() as u32;

    if total == 0 {
        return Err("没有找到可转换的图片组（需有封面 + 至少一张子图）".into());
    }

    let total_msg = format!("共发现 {} 组可转换", total);
    let _ = app.emit(
        "batch-progress",
        ProgressPayload {
            current: 0,
            total,
            message: total_msg.clone(),
            status: "running".into(),
        },
    );

    // 2. 构建任务列表
    let output_dir = config.output_dir.clone();
    struct Task {
        cover_path: Option<String>,
        sub_image_paths: Vec<String>,
        match_paths: Vec<String>,
        output_path: String,
        source_files: Vec<String>,
        message: String,
    }

    let tasks: Vec<Task> = groups
        .iter()
        .map(|g| {
            let output_path = resolve_batch_convert_output_path(
                g,
                &output_dir,
                config.preserve_structure,
                config.merge_existing_ims,
            );
            let output_filename = Path::new(&output_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&output_path)
                .to_string();

            let cover_path = g.cover.as_ref().map(|p| p.to_string_lossy().to_string());
            let mut sub_image_paths = Vec::new();
            let mut match_paths = Vec::new();
            let mut source_files = Vec::new();

            if let Some(cover) = &cover_path {
                match_paths.push(cover.clone());
                source_files.push(cover.clone());
            }
            for (_, sub_path) in &g.sub_images {
                let source = sub_path.to_string_lossy().to_string();
                sub_image_paths.push(source.clone());
                match_paths.push(source.clone());
                source_files.push(source);
            }

            Task {
                cover_path,
                sub_image_paths,
                match_paths,
                output_path,
                source_files,
                message: format!("已转换 {}", output_filename),
            }
        })
        .collect();

    // 3. 执行转换
    let delete_source = config.delete_source;
    let multi = config.multi_threaded;
    let merge_existing_ims = config.merge_existing_ims;
    let sub_images_to_jpeg = config.sub_images_to_jpeg;
    let app_handle = app.clone();

    std::thread::spawn(move || {
        let convert = |task: &Task| -> Result<String, String> {
            if merge_existing_ims && Path::new(&task.output_path).is_file() {
                let merge = merge_into_existing_ims(
                    &task.output_path,
                    &task.match_paths,
                    &task.sub_image_paths,
                    sub_images_to_jpeg,
                )?;
                if delete_source {
                    for src in &merge.delete_files {
                        let _ = std::fs::remove_file(src);
                    }
                }
                return Ok(format!(
                    "已合并 {}（新增 {}，已存在 {}，MD5 命中 {}）",
                    Path::new(&task.output_path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&task.output_path),
                    merge.inserted,
                    merge.skipped_existing,
                    merge.matched_existing
                ));
            }

            let cover_path = task.cover_path.as_ref().ok_or_else(|| {
                format!("目标 IMS 不存在且没有封面，无法创建: {}", task.output_path)
            })?;
            let mut image_paths = Vec::with_capacity(1 + task.sub_image_paths.len());
            image_paths.push(cover_path.clone());
            image_paths.extend(task.sub_image_paths.clone());

            pack_ims(&image_paths, 0, &task.output_path, sub_images_to_jpeg)?;
            if delete_source {
                for src in &task.source_files {
                    let _ = std::fs::remove_file(src);
                }
            }
            Ok(task.message.clone())
        };

        let result = if multi {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            tasks.par_iter().for_each(|task| match convert(task) {
                Ok(message) => {
                    let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                    let _ = app_handle.emit(
                        "batch-progress",
                        ProgressPayload {
                            current,
                            total,
                            message,
                            status: "running".into(),
                        },
                    );
                }
                Err(e) => {
                    has_error.store(true, Ordering::SeqCst);
                    let _ = app_handle.emit(
                        "batch-progress",
                        ProgressPayload {
                            current: completed.load(Ordering::SeqCst),
                            total,
                            message: format!("失败: {}", e),
                            status: "running".into(),
                        },
                    );
                }
            });

            if has_error.load(Ordering::SeqCst) {
                Err("有转换失败".into())
            } else {
                Ok(())
            }
        } else {
            let mut completed: u32 = 0;
            let mut has_error = false;
            for task in &tasks {
                match convert(task) {
                    Ok(message) => {
                        completed += 1;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current: completed,
                                total,
                                message,
                                status: "running".into(),
                            },
                        );
                    }
                    Err(e) => {
                        has_error = true;
                        let _ = app_handle.emit(
                            "batch-progress",
                            ProgressPayload {
                                current: completed,
                                total,
                                message: format!("失败: {}", e),
                                status: "running".into(),
                            },
                        );
                    }
                }
            }
            if has_error {
                Err("存在转换失败".into())
            } else {
                Ok(())
            }
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: format!("全部完成，共 {} 组", total),
                        status: "completed".into(),
                    },
                );
            }
            Err(e) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: e,
                        status: "error".into(),
                    },
                );
            }
        }
    });

    Ok(total_msg)
}

// ============================================================
// Tauri 应用入口
// ============================================================

/// 返回通过命令行传入的启动文件路径（右键"打开方式"传入）
#[tauri::command]
fn get_launch_file(state: tauri::State<'_, LaunchFile>) -> Result<Option<String>, String> {
    Ok(state.0.clone())
}

pub struct LaunchFile(pub Option<String>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// 清理 Tauri Updater 残留的临时安装包。
/// Updater 下载的安装包写入 %TEMP%\imsViewer-<ver>-updater-*\ 目录，
/// 因 install_inner 调用 std::process::exit(0) 跳过 Drop，临时文件不会被自动删除。
/// 此命令在应用启动时调用，删除所有旧的 updater 临时目录。
#[tauri::command]
fn cleanup_updater_temp() -> Result<u64, String> {
    let temp_dir = std::env::temp_dir();
    let mut removed: u64 = 0;

    let read_dir = std::fs::read_dir(&temp_dir)
        .map_err(|e| format!("读取临时目录失败: {}", e))?;

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // 匹配 Tauri Updater 的临时目录命名: imsViewer-<version>-updater-<random>
        if !name.starts_with("imsViewer-") || !name.contains("-updater-") {
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(_) => removed += 1,
            Err(e) => {
                // 目录可能被正在运行的安装器锁定，忽略错误
                eprintln!("清理 updater 临时目录失败 {:?}: {}", path, e);
            }
        }
    }

    Ok(removed)
}

pub fn run(launch_file: Option<String>) {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(LaunchFile(launch_file))
        .manage(ImsCache(Mutex::new(HashMap::new())))
        .manage(FolderScanState::new())
        .manage(dedup::DedupService::new())
        .invoke_handler(tauri::generate_handler![
            open_file,
            save_ims,
            pick_file,
            pick_save_path,
            pick_folder,
            get_sub_image,
            extract_image,
            extract_all,
            start_batch_extract,
            start_batch_convert,
            start_batch_upgrade,
            start_batch_resize,
            register_file_associations,
            get_launch_file,
            refresh_folder,
            dedup::dedup_get_model_status,
            dedup::dedup_get_index_status,
            dedup::dedup_list_indexes,
            dedup::dedup_update_index,
            dedup::dedup_rebuild_index,
            dedup::dedup_search,
            dedup::dedup_start_batch_scan,
            dedup::dedup_resume_batch_scan,
            dedup::dedup_list_batch_runs,
            dedup::dedup_get_batch_run,
            dedup::dedup_get_batch_groups,
            dedup::dedup_get_batch_group_members,
            dedup::dedup_preview_batch_auto_delete,
            dedup::dedup_auto_delete_batch_files,
            dedup::dedup_delete_batch_run,
            dedup::dedup_cancel_job,
            dedup::dedup_get_result_thumbnail,
            dedup::dedup_delete_indexed_file,
            dedup::dedup_verify_result,
            dedup::dedup_delete_index,
            dedup::dedup_delete_all_indexes,
            dedup::dedup_reveal_file,
            cleanup_updater_temp,
        ])
        .run(tauri::generate_context!())
        .expect("启动 imsViewer 失败");
}

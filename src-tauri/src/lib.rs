//! IMS Viewer — Tauri Rust 后端
//!
//! 暴露给前端 (app.js) 的命令：
//!   open_file(path)    — 打开文件，扫描文件夹，解析 IMS，返回完整载荷
//!   pick_file()        — 弹出原生文件选择对话框
//!   get_sub_image()    — 获取当前 IMS 文件的指定子图数据
//!   extract_image()    — 导出单张图片
//!   extract_all()      — 导出全部子图

use base64::Engine;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::Emitter;

mod ims;
mod ims_pack;

use ims::{parse_ims, parse_ims_and_read_cover, read_sub_image, is_ims_file, file_type_label, FilePayload, SubImagePayload, SaveImagePayload, FolderEntry};
use ims_pack::{pack_ims, scan_multiple_folders, compute_output_path, upgrade_ims_file, scan_ims_files, save_ims_entries_atomic, UpEntry, BatchConfig, ProgressPayload};

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
        Self { cache: Mutex::new(HashMap::new()) }
    }
}

/// 缓存最大条目数。IMS 元数据很小（每条 ~KB 级），64 条约几十 KB。
const MAX_CACHE_ENTRIES: usize = 64;

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
            if path.is_dir() { continue; }
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if filename.starts_with('.') { continue; }
            let ext = path.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            let stem = path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let is_ims = stem.contains(".ims");
            if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif") { continue; }
            entries.push(FolderEntry {
                filepath: path.to_str().unwrap_or(&filename).to_string(),
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
    let mut i = 0; let mut j = 0;
    while i < a_chars.len() && j < b_chars.len() {
        if a_chars[i].is_ascii_digit() && b_chars[j].is_ascii_digit() {
            let mut an = String::new(); let mut bn = String::new();
            while i < a_chars.len() && a_chars[i].is_ascii_digit() { an.push(a_chars[i]); i += 1; }
            while j < b_chars.len() && b_chars[j].is_ascii_digit() { bn.push(b_chars[j]); j += 1; }
            match an.parse::<u64>().unwrap_or(0).cmp(&bn.parse::<u64>().unwrap_or(0)) {
                std::cmp::Ordering::Equal => continue,
                o => return o,
            }
        } else {
            match a_chars[i].cmp(&b_chars[j]) {
                std::cmp::Ordering::Equal => { i += 1; j += 1; }
                o => return o,
            }
        }
    }
    a_chars.len().cmp(&b_chars.len())
}

/// 获取文件夹列表。通过 mtime 自动检测目录变更，过期则重新扫描。
fn get_or_scan_folder(
    parent_dir: &str,
    state: &FolderScanState,
) -> Vec<FolderEntry> {
    let current_mtime = std::fs::metadata(parent_dir)
        .and_then(|m| m.modified())
        .ok();

    // 缓存命中 + mtime 未变 → 直接返回
    if let Some(mtime) = current_mtime {
        let cache = state.cache.lock().unwrap();
        if let Some((cached_mtime, list)) = cache.get(parent_dir) {
            if *cached_mtime == mtime {
                return list.clone();
            }
        }
    }

    // 缓存缺失、mtime 变更或无权限获取 mtime → 重新扫描
    let list = do_scan_dir(parent_dir);
    if let Some(mtime) = current_mtime {
        state.cache.lock().unwrap().insert(parent_dir.to_string(), (mtime, list.clone()));
    }
    list
}

// ============================================================
// Tauri Commands
// ============================================================

#[tauri::command]
fn open_file(path: String, cache: tauri::State<'_, ImsCache>, scan_state: tauri::State<'_, FolderScanState>) -> Result<FilePayload, String> {
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
    let file_index = folder_list.iter().position(|e| e.filepath == path).unwrap_or(0);

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
        let raw_bytes = std::fs::read(&path)
            .map_err(|e| format!("读取文件失败: {}", e))?;

        // 推断格式
        let format: u8 = if path.to_lowercase().ends_with(".png") { 1 } else { 0 };
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
fn save_ims(path: String, images: Vec<SaveImagePayload>, cache: tauri::State<'_, ImsCache>) -> Result<(), String> {
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
            let decoded = image::load_from_memory(&raw_bytes)
                .map_err(|e| format!("解码图片失败: {} (格式: {})", e, if img.format == 2 { "WebP" } else { "AVIF" }))?;
            let mut png_bytes = Vec::new();
            decoded
                .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
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
        .add_filter("图片文件", &["jpg", "jpeg", "png", "ims.jpg", "ims.png"])
        .add_filter("IMS 文件", &["ims.jpg", "ims.png"])
        .add_filter("普通图片", &["jpg", "jpeg", "png"])
        .add_filter("所有文件", &["*"])
        .blocking_pick_file();

    Ok(result.map(|p| p.to_string()))
}

/// 弹出另存为对话框
#[tauri::command]
async fn pick_save_path(app: tauri::AppHandle, default_name: String) -> Result<Option<String>, String> {
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

    let result = app
        .dialog()
        .file()
        .blocking_pick_folder();

    Ok(result.map(|p| p.to_string()))
}

/// 按需获取 IMS 文件中指定索引的子图数据（base64）
/// 使用缓存避免对同一文件反复解析 IMS 索引。
#[tauri::command]
fn get_sub_image(path: String, index: usize, cache: tauri::State<'_, ImsCache>) -> Result<SubImagePayload, String> {
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
async fn extract_image(app: tauri::AppHandle, path: String, index: usize, cache: tauri::State<'_, ImsCache>) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let (bytes, default_name) = if is_ims_file(&path) {
        let ims_info = cached_parse_ims(&cache, &path)?;
        let img = ims_info
            .images
            .get(index)
            .ok_or_else(|| "子图索引越界".to_string())?;
        let data = read_sub_image(&path, img.data_offset, img.data_length)?;

        // 使用原始文件名，保留扩展名
        let ext = if img.format == 1 { "png" } else { "jpg" };
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
        std::fs::write(Path::new(&path_str), &bytes)
            .map_err(|e| format!("写入文件失败: {}", e))?;
        Ok(path_str)
    } else {
        Err("用户取消了保存".into())
    }
}

/// 导出 IMS 文件中的全部子图到指定目录
#[tauri::command]
async fn extract_all(app: tauri::AppHandle, path: String, cache: tauri::State<'_, ImsCache>) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    if !is_ims_file(&path) {
        return Err("非 IMS 文件，无法导出子图".into());
    }

    // 选择输出目录
    let dir = app
        .dialog()
        .file()
        .blocking_pick_folder();

    if dir.is_none() {
        return Err("用户取消了选择".into());
    }

    let out_dir = dir.unwrap().to_string();
    let ims_info = cached_parse_ims(&cache, &path)?;
    let mut saved_paths = Vec::new();

    for img in &ims_info.images {
        let bytes = read_sub_image(&path, img.data_offset, img.data_length)?;
        let ext = if img.format == 1 { "png" } else { "jpg" };
        let out_path = Path::new(&out_dir).join(format!("{}.{}",
            Path::new(&img.filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&img.filename),
            ext
        ));
        std::fs::write(&out_path, &bytes)
            .map_err(|e| format!("写入失败 {}: {}", out_path.display(), e))?;
        saved_paths.push(out_path.to_string_lossy().to_string());
    }

    Ok(saved_paths)
}

/// 注册 imsViewer 到右键菜单"使用 imsViewer 打开"
#[tauri::command]
fn register_file_associations() -> Result<String, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("获取 exe 路径失败: {}", e))?;
    let exe_str = exe_path.to_string_lossy().to_string();
    let exe_cmd = format!("\"{}\" \"%1\"", exe_str);
    let ico = format!("\"{}\",0", exe_str);

    let ext_groups: &[(&str, &[&str])] = &[
        ("image", &["jpg", "jpeg", "png", "webp", "avif"]),
        ("imscontainer", &["ims.jpg", "ims.png"]),
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
    let _ = std::process::Command::new("ie4uinit").args(["-show"]).output();

    Ok("已添加到右键菜单".into())
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
    let _ = app.emit("batch-progress", ProgressPayload {
        current: 0,
        total,
        message: total_msg.clone(),
        status: "running".into(),
    });

    let app_handle = app.clone();

    std::thread::spawn(move || {
        let result = if multi_threaded {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            files.par_iter().for_each(|fp| {
                let fname = Path::new(fp).file_name().and_then(|n| n.to_str()).unwrap_or(fp);
                match upgrade_ims_file(fp) {
                    Ok(()) => {
                        let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current,
                            total,
                            message: format!("已升级 {}", fname),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        has_error.store(true, Ordering::SeqCst);
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed.load(Ordering::SeqCst),
                            total,
                            message: format!("失败 {}: {}", fname, e),
                            status: "running".into(),
                        });
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
                let fname = Path::new(fp).file_name().and_then(|n| n.to_str()).unwrap_or(fp);
                match upgrade_ims_file(fp) {
                    Ok(()) => {
                        completed += 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: format!("已升级 {}", fname),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: format!("失败 {}: {}", fname, e),
                            status: "running".into(),
                        });
                    }
                }
            }
            Ok(())
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: format!("全部完成，共 {} 个文件", total),
                    status: "completed".into(),
                });
            }
            Err(e) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: e,
                    status: "error".into(),
                });
            }
        }
    });

    Ok(total_msg)
}

/// 批量转换：扫描文件夹，按命名规范分组并打包为 IMS 文件
#[tauri::command]
async fn start_batch_convert(
    app: tauri::AppHandle,
    config: BatchConfig,
) -> Result<String, String> {
    // 1. 扫描所有文件夹，分组
    let groups = scan_multiple_folders(&config.folders)?;
    let total = groups.len() as u32;

    if total == 0 {
        return Err("没有找到可转换的图片组（需有封面 + 至少一张子图）".into());
    }

    let total_msg = format!("共发现 {} 组可转换", total);
    let _ = app.emit("batch-progress", ProgressPayload {
        current: 0,
        total,
        message: total_msg.clone(),
        status: "running".into(),
    });

    // 2. 构建任务列表
    let output_dir = config.output_dir.clone();
    struct Task {
        image_paths: Vec<String>,
        output_path: String,
        source_files: Vec<String>,
        message: String,
    }

    let tasks: Vec<Task> = groups.iter().map(|g| {
        let output_filename = format!("{}.ims.{}", g.base_name, g.cover_ext);
        let output_path = compute_output_path(g, &output_dir, config.preserve_structure);

        let mut image_paths = vec![g.cover.to_string_lossy().to_string()];
        let mut source_files = vec![g.cover.to_string_lossy().to_string()];
        for (_, sub_path) in &g.sub_images {
            image_paths.push(sub_path.to_string_lossy().to_string());
            source_files.push(sub_path.to_string_lossy().to_string());
        }

        Task {
            image_paths,
            output_path,
            source_files,
            message: format!("已转换 {}", output_filename),
        }
    }).collect();

    // 3. 执行转换
    let delete_source = config.delete_source;
    let multi = config.multi_threaded;
    let app_handle = app.clone();

    std::thread::spawn(move || {
        let convert = |task: &Task| -> Result<(), String> {
            pack_ims(&task.image_paths, 0, &task.output_path)?;
            if delete_source {
                for src in &task.source_files {
                    let _ = std::fs::remove_file(src);
                }
            }
            Ok(())
        };

        let result = if multi {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            tasks.par_iter().for_each(|task| {
                match convert(task) {
                    Ok(()) => {
                        let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current,
                            total,
                            message: task.message.clone(),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        has_error.store(true, Ordering::SeqCst);
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed.load(Ordering::SeqCst),
                            total,
                            message: format!("失败: {}", e),
                            status: "running".into(),
                        });
                    }
                }
            });

            if has_error.load(Ordering::SeqCst) {
                Err("有转换失败".into())
            } else {
                Ok(())
            }
        } else {
            let mut completed: u32 = 0;
            for task in &tasks {
                match convert(task) {
                    Ok(()) => {
                        completed += 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: task.message.clone(),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: format!("失败: {}", e),
                            status: "running".into(),
                        });
                    }
                }
            }
            Ok(())
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: format!("全部完成，共 {} 组", total),
                    status: "completed".into(),
                });
            }
            Err(e) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: e,
                    status: "error".into(),
                });
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
pub fn run(launch_file: Option<String>) {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(LaunchFile(launch_file))
        .manage(ImsCache(Mutex::new(HashMap::new())))
        .manage(FolderScanState::new())
        .invoke_handler(tauri::generate_handler![
            open_file,
            save_ims,
            pick_file,
            pick_save_path,
            pick_folder,
            get_sub_image,
            extract_image,
            extract_all,
            start_batch_convert,
            start_batch_upgrade,
            register_file_associations,
            get_launch_file,
        ])
        .run(tauri::generate_context!())
        .expect("启动 imsViewer 失败");
}

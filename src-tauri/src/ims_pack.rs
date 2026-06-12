//! IMS 图片打包与批量转换模块
//!
//! 与 Python 版 ims_pack.py + ims_batch.py 等价的 Rust 实现。

use byteorder::{LittleEndian, WriteBytesExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// IMS 文件魔数
const MAGIC: &[u8; 8] = b"IMSIMAGE";
const VERSION: u16 = 2;
const HASH_ALGO_SHA256: u8 = 1;

/// 图片组：一个封面 + 若干子图，记录来源文件夹
#[derive(Debug)]
pub struct ImageGroup {
    pub base_name: String,
    pub cover: PathBuf,
    pub sub_images: Vec<(u32, PathBuf)>, // (seq_number, path)
    pub cover_ext: String,               // "jpg" or "png"
    pub source_dir: PathBuf,             // 文件所在的目录
    pub input_folder: String,            // 所属的输入文件夹路径
}

/// 批量转换配置（从前端传入）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchConfig {
    pub folders: Vec<FolderConfig>,
    pub delete_source: bool,
    pub multi_threaded: bool,
    pub output_dir: Option<String>,
    pub preserve_structure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderConfig {
    pub path: String,
    pub depth: u32,
}

/// 进度事件载荷（发送给前端）
#[derive(Debug, Clone, Serialize)]
pub struct ProgressPayload {
    pub current: u32,
    pub total: u32,
    pub message: String,
    pub status: String, // "running" | "completed" | "error"
}

// ---- IMS 打包 ----

/// 将多张图片打包为单个 IMS 文件
pub fn pack_ims(image_paths: &[String], cover_index: usize, output_path: &str) -> Result<(), String> {
    let count = image_paths.len();
    if count == 0 {
        return Err("图片列表为空".into());
    }
    if cover_index >= count {
        return Err(format!("封面索引 {} 超出范围 (共 {} 张)", cover_index, count));
    }

    // 验证并读取所有图片
    struct Entry {
        filename: String,
        format: u8,
        bytes: Vec<u8>,
        is_cover: bool,
    }

    let mut entries: Vec<Entry> = Vec::with_capacity(count);
    for (i, path) in image_paths.iter().enumerate() {
        let p = Path::new(path);
        if !p.exists() {
            return Err(format!("文件不存在: {}", path));
        }
        let ext = p.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format: u8 = match ext.as_str() {
            "jpg" | "jpeg" => 0,
            "png" => 1,
            "webp" => 2,
            "avif" => 3,
            _ => return Err(format!("不支持的图片格式: {} (仅支持 jpg/jpeg/png/webp/avif)", path)),
        };
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            if stem.to_lowercase().contains(".ims") {
                return Err(format!("不能将已是 IMS 的文件作为源: {}", path));
            }
        }
        let filename = p.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let raw_bytes = std::fs::read(path)
            .map_err(|e| format!("读取文件失败 {}: {}", path, e))?;

        // WebP/AVIF → 自动转 PNG
        let (final_bytes, final_format, final_filename) = if format == 2 || format == 3 {
            let img = image::load_from_memory(&raw_bytes)
                .map_err(|e| format!("解码 {} 失败: {} (格式: {})", path, e, if format == 2 { "WebP" } else { "AVIF" }))?;
            let mut png_bytes = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
                .map_err(|e| format!("PNG 编码失败 {}: {}", path, e))?;
            let new_name = format!(
                "{}.png",
                Path::new(&filename).file_stem().and_then(|s| s.to_str()).unwrap_or("image")
            );
            (png_bytes, 1u8, new_name)
        } else {
            (raw_bytes, format, filename)
        };

        entries.push(Entry { filename: final_filename, format: final_format, bytes: final_bytes, is_cover: i == cover_index });
    }

    // 封面排在第一位
    entries.sort_by_key(|e| !e.is_cover);

    // 计算累计偏移量
    let mut offsets: Vec<u32> = Vec::with_capacity(count);
    let mut cumulative: u32 = 0;
    for entry in &entries {
        offsets.push(cumulative);
        cumulative += entry.bytes.len() as u32;
    }
    let index_offset = cumulative;

    // 确保输出目录存在
    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建输出目录失败 {}: {}", parent.display(), e))?;
    }

    // 写入输出文件
    let mut out = std::fs::File::create(output_path)
        .map_err(|e| format!("创建输出文件失败 {}: {}", output_path, e))?;

    // 1. 写入所有图片数据
    for entry in &entries {
        out.write_all(&entry.bytes)
            .map_err(|e| format!("写入图片数据失败: {}", e))?;
    }

    // 2. 写入 index footer
    out.write_all(MAGIC).map_err(|e| format!("写入 magic 失败: {}", e))?;
    out.write_u16::<LittleEndian>(VERSION).map_err(|e| format!("写入 version 失败: {}", e))?;
    out.write_u32::<LittleEndian>(count as u32).map_err(|e| format!("写入 image_count 失败: {}", e))?;
    out.write_u32::<LittleEndian>(0).map_err(|e| format!("写入 data_start 失败: {}", e))?;

    // 3. 写入每个 entry (v2: 带 SHA-256 hash)
    for (i, entry) in entries.iter().enumerate() {
        let name_bytes = entry.filename.as_bytes();
        out.write_u16::<LittleEndian>(name_bytes.len() as u16).map_err(|e| format!("写入 name_len 失败: {}", e))?;
        out.write_all(name_bytes).map_err(|e| format!("写入 name 失败: {}", e))?;
        out.write_u8(entry.format).map_err(|e| format!("写入 format 失败: {}", e))?;
        out.write_u32::<LittleEndian>(offsets[i]).map_err(|e| format!("写入 offset 失败: {}", e))?;
        out.write_u32::<LittleEndian>(entry.bytes.len() as u32).map_err(|e| format!("写入 length 失败: {}", e))?;
        out.write_u8(if entry.is_cover { 1 } else { 0 }).map_err(|e| format!("写入 is_cover 失败: {}", e))?;

        // v2: SHA-256 hash
        let mut hasher = Sha256::new();
        hasher.update(&entry.bytes);
        let hash = hasher.finalize();
        out.write_u8(HASH_ALGO_SHA256).map_err(|e| format!("写入 hash_algo 失败: {}", e))?;
        out.write_all(&hash).map_err(|e| format!("写入 hash 失败: {}", e))?;
    }

    // 4. 写入 index_offset
    out.write_u32::<LittleEndian>(index_offset).map_err(|e| format!("写入 index_offset 失败: {}", e))?;

    Ok(())
}

// ---- 文件夹扫描与分组 ----

/// 扫描文件夹，将图片按 base_name 分组
pub fn scan_and_group(input_folder: &str, max_depth: u32) -> Result<Vec<ImageGroup>, String> {
    let dir_path = Path::new(input_folder);
    if !dir_path.is_dir() {
        return Err(format!("目录不存在: {}", input_folder));
    }

    let re = Regex::new(r"^(.+)_(\d{5})_$").map_err(|e| format!("正则编译失败: {}", e))?;

    // 收集所有图片文件
    let mut all_files: Vec<(PathBuf, PathBuf)> = Vec::new(); // (file_path, parent_dir)

    let mut walker = walkdir::WalkDir::new(input_folder);
    if max_depth > 0 {
        walker = walker.max_depth(max_depth as usize);
    }
    for entry in walker.into_iter().filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let lower = ext.to_lowercase();
            if lower == "jpg" || lower == "jpeg" || lower == "png" || lower == "webp" || lower == "avif" {
                // 跳过已经是 IMS 的文件
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem.to_lowercase().contains(".ims") {
                        continue;
                    }
                }
                let parent = path.parent().unwrap_or(dir_path).to_path_buf();
                all_files.push((path.to_path_buf(), parent));
            }
        }
    }

    // 分组: base_name -> (cover, sub_images, source_dir, input_folder)
    let mut groups: BTreeMap<String, (Option<PathBuf>, Vec<(u32, PathBuf)>, Option<PathBuf>)> =
        BTreeMap::new();
    // value: (cover, sub_images, source_dir)

    for (path, parent_dir) in &all_files {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        if let Some(caps) = re.captures(stem) {
            // 子图: {base}_XXXXX_
            let base_name = caps.get(1).unwrap().as_str().to_string();
            let seq: u32 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
            let entry = groups.entry(base_name).or_insert_with(|| (None, Vec::new(), None));
            entry.1.push((seq, path.clone()));
            if entry.2.is_none() {
                entry.2 = Some(parent_dir.clone());
            }
        } else {
            // 封面: {base}.{ext}
            let base_name = stem.to_string();
            let entry = groups.entry(base_name).or_insert_with(|| (None, Vec::new(), None));
            entry.0 = Some(path.clone());
            if entry.2.is_none() {
                entry.2 = Some(parent_dir.clone());
            }
        }
    }

    // 筛选有效组（有封面 + 有子图）
    let mut result: Vec<ImageGroup> = Vec::new();
    for (base_name, (cover_opt, mut sub_images, source_dir_opt)) in groups {
        let cover = match cover_opt {
            Some(c) => c,
            None => continue,
        };
        if sub_images.is_empty() {
            continue;
        }
        sub_images.sort_by_key(|(seq, _)| *seq);

        let raw_ext = cover
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();
        // WebP/AVIF 会转为 PNG，输出扩展名改为 png
        let cover_ext = match raw_ext.as_str() {
            "webp" | "avif" => "png".to_string(),
            other => other.to_string(),
        };

        let source_dir = source_dir_opt.unwrap_or_else(|| dir_path.to_path_buf());

        result.push(ImageGroup {
            base_name,
            cover,
            sub_images,
            cover_ext,
            source_dir,
            input_folder: input_folder.to_string(),
        });
    }

    result.sort_by(|a, b| a.base_name.cmp(&b.base_name));
    Ok(result)
}

// ---- IMS 版本升级 ----

/// 将旧版 IMS 文件升级到 v2（添加 SHA-256 hash）
pub fn upgrade_ims_file(filepath: &str) -> Result<(), String> {
    use crate::ims::{parse_ims, read_sub_image};

    let ims_info = parse_ims(filepath)?;

    // 已经是 v2+，跳过
    if ims_info.version >= 2 {
        return Ok(());
    }

    // 读取所有子图数据
    let mut entries: Vec<UpEntry> = Vec::new();
    for img in &ims_info.images {
        let bytes = read_sub_image(filepath, img.data_offset, img.data_length)?;
        entries.push(UpEntry {
            filename: img.filename.clone(),
            format: img.format,
            bytes,
            is_cover: img.is_cover,
        });
    }

    // 封面排第一
    entries.sort_by_key(|e| !e.is_cover);

    // 写入临时文件，然后替换
    let tmp_path = format!("{}.tmp_upgrade", filepath);
    write_ims_entries(&tmp_path, &entries)?;
    std::fs::remove_file(filepath)
        .map_err(|e| format!("删除旧文件失败: {}", e))?;
    std::fs::rename(&tmp_path, filepath)
        .map_err(|e| format!("替换文件失败: {}", e))?;

    Ok(())
}

/// 扫描目录中所有 IMS 文件
pub fn scan_ims_files(dir: &str, max_depth: u32) -> Vec<String> {
    let mut walker = walkdir::WalkDir::new(dir);
    if max_depth > 0 {
        walker = walker.max_depth(max_depth as usize);
    }
    walker
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            if let Some(stem) = e.path().file_stem().and_then(|s| s.to_str()) {
                stem.to_lowercase().contains(".ims")
            } else {
                false
            }
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .collect()
}

/// 直接写入 IMS v2 文件（内部用，接受已读取的字节数据）
fn write_ims_entries(output_path: &str, entries: &[UpEntry]) -> Result<(), String> {
    let count = entries.len();
    let mut offsets: Vec<u32> = Vec::with_capacity(count);
    let mut cumulative: u32 = 0;
    for e in entries {
        offsets.push(cumulative);
        cumulative += e.bytes.len() as u32;
    }
    let index_offset = cumulative;

    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let mut out = std::fs::File::create(output_path)
        .map_err(|e| format!("创建文件失败: {}", e))?;

    for e in entries {
        out.write_all(&e.bytes).map_err(|e| format!("写入数据失败: {}", e))?;
    }

    out.write_all(MAGIC).map_err(|e| format!("写入 magic 失败: {}", e))?;
    out.write_u16::<LittleEndian>(VERSION).map_err(|e| format!("写入 version 失败: {}", e))?;
    out.write_u32::<LittleEndian>(count as u32).map_err(|e| format!("写入 count 失败: {}", e))?;
    out.write_u32::<LittleEndian>(0).map_err(|e| format!("写入 data_start 失败: {}", e))?;

    for (i, e) in entries.iter().enumerate() {
        let name_bytes = e.filename.as_bytes();
        out.write_u16::<LittleEndian>(name_bytes.len() as u16).map_err(|e| format!("写入 name_len 失败: {}", e))?;
        out.write_all(name_bytes).map_err(|e| format!("写入 name 失败: {}", e))?;
        out.write_u8(e.format).map_err(|e| format!("写入 format 失败: {}", e))?;
        out.write_u32::<LittleEndian>(offsets[i]).map_err(|e| format!("写入 offset 失败: {}", e))?;
        out.write_u32::<LittleEndian>(e.bytes.len() as u32).map_err(|e| format!("写入 length 失败: {}", e))?;
        out.write_u8(if e.is_cover { 1 } else { 0 }).map_err(|e| format!("写入 is_cover 失败: {}", e))?;

        let mut hasher = Sha256::new();
        hasher.update(&e.bytes);
        let hash = hasher.finalize();
        out.write_u8(HASH_ALGO_SHA256).map_err(|e| format!("写入 hash_algo 失败: {}", e))?;
        out.write_all(&hash).map_err(|e| format!("写入 hash 失败: {}", e))?;
    }

    out.write_u32::<LittleEndian>(index_offset).map_err(|e| format!("写入 index_offset 失败: {}", e))?;

    Ok(())
}

struct UpEntry {
    filename: String,
    format: u8,
    bytes: Vec<u8>,
    is_cover: bool,
}

/// 扫描多个文件夹，汇总所有 ImageGroup
pub fn scan_multiple_folders(folders: &[FolderConfig]) -> Result<Vec<ImageGroup>, String> {
    let mut all_groups = Vec::new();
    for folder in folders {
        let groups = scan_and_group(&folder.path, folder.depth)?;
        all_groups.extend(groups);
    }
    Ok(all_groups)
}

/// 计算输出路径
///
/// 若指定了 output_dir，则按输入目录结构镜像输出：
/// 例如 input="Input1/tex0", output_dir="Output" → "Output/Input1/tex0/file.ims.jpg"
///
/// 若未指定 output_dir，则直接输出到源文件所在目录（原地转换）
pub fn compute_output_path(
    group: &ImageGroup,
    output_dir: &Option<String>,
    preserve_structure: bool,
) -> String {
    let output_filename = format!("{}.ims.{}", group.base_name, group.cover_ext);
    if let Some(out_dir) = output_dir {
        if preserve_structure {
            // 用 input_folder 的父目录作为基准，保留输入文件夹名本身
            let input_base = Path::new(&group.input_folder)
                .parent()
                .unwrap_or(Path::new(&group.input_folder));
            let rel = group.source_dir
                .strip_prefix(input_base)
                .unwrap_or(&group.source_dir);
            let out_path = Path::new(out_dir).join(rel).join(&output_filename);
            out_path.to_string_lossy().to_string()
        } else {
            Path::new(out_dir).join(&output_filename).to_string_lossy().to_string()
        }
    } else {
        group.source_dir.join(&output_filename).to_string_lossy().to_string()
    }
}

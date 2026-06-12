//! IMS 图片打包与批量转换模块
//!
//! 与 Python 版 ims_pack.py + ims_batch.py 等价的 Rust 实现。

use byteorder::{LittleEndian, WriteBytesExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// IMS 文件魔数
const MAGIC: &[u8; 8] = b"IMSIMAGE";
const VERSION: u16 = 1;

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
            _ => return Err(format!("不支持的图片格式: {} (仅支持 jpg/jpeg/png)", path)),
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
        let bytes = std::fs::read(path)
            .map_err(|e| format!("读取文件失败 {}: {}", path, e))?;
        entries.push(Entry { filename, format, bytes, is_cover: i == cover_index });
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

    // 3. 写入每个 entry
    for (i, entry) in entries.iter().enumerate() {
        let name_bytes = entry.filename.as_bytes();
        out.write_u16::<LittleEndian>(name_bytes.len() as u16).map_err(|e| format!("写入 name_len 失败: {}", e))?;
        out.write_all(name_bytes).map_err(|e| format!("写入 name 失败: {}", e))?;
        out.write_u8(entry.format).map_err(|e| format!("写入 format 失败: {}", e))?;
        out.write_u32::<LittleEndian>(offsets[i]).map_err(|e| format!("写入 offset 失败: {}", e))?;
        out.write_u32::<LittleEndian>(entry.bytes.len() as u32).map_err(|e| format!("写入 length 失败: {}", e))?;
        out.write_u8(if entry.is_cover { 1 } else { 0 }).map_err(|e| format!("写入 is_cover 失败: {}", e))?;
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

    for entry in walkdir::WalkDir::new(input_folder)
        .max_depth(max_depth as usize)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let lower = ext.to_lowercase();
            if lower == "jpg" || lower == "jpeg" || lower == "png" {
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

        let cover_ext = cover
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();

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
) -> String {
    let output_filename = format!("{}.ims.{}", group.base_name, group.cover_ext);
    if let Some(out_dir) = output_dir {
        // 计算 source_dir 相对于 input_folder 的路径
        let input_base = Path::new(&group.input_folder);
        let rel = group.source_dir
            .strip_prefix(input_base)
            .unwrap_or(&group.source_dir);
        let out_path = Path::new(out_dir).join(rel).join(&output_filename);
        out_path.to_string_lossy().to_string()
    } else {
        group.source_dir.join(&output_filename).to_string_lossy().to_string()
    }
}

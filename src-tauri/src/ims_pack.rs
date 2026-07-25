//! IMS 图片打包与批量转换模块
//!
//! 与 Python 版 ims_pack.py + ims_batch.py 等价的 Rust 实现。

use byteorder::{LittleEndian, WriteBytesExt};
use image::ImageEncoder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// IMS 文件魔数
const MAGIC: &[u8; 8] = b"IMSIMAGE";
const VERSION: u16 = 2;
const HASH_ALGO_SHA256: u8 = 1;
const THUMBNAIL_MAGIC: &[u8; 8] = b"IMSTHMB\0";
const THUMBNAIL_VERSION: u16 = 1;
const THUMBNAIL_FORMAT_JPEG: u8 = 0;
const THUMBNAIL_MAX_SIZE: u32 = 160;
const THUMBNAIL_JPEG_QUALITY: u8 = 60;
const RESIZE_JPEG_QUALITY: u8 = 88;
const CONVERT_JPEG_QUALITY: u8 = 90;

struct EmbeddedThumbnail {
    bytes: Vec<u8>,
    width: u16,
    height: u16,
    crc32: u32,
}

/// 将带 alpha 的 RGBA 图像合成到白底并返回 RGB 图像（用于 JPEG 编码）。
///
/// 合成公式与 `create_embedded_thumbnail` 原有逻辑一致：
/// `out = (src * alpha + 255 * (255 - alpha) + 127) / 255`
fn flatten_alpha_to_rgb(rgba: image::RgbaImage) -> image::RgbImage {
    let width = rgba.width();
    let height = rgba.height();
    let mut rgb = image::RgbImage::new(width, height);
    for (column, row, source_pixel) in rgba.enumerate_pixels() {
        let alpha = source_pixel[3] as u16;
        let target_pixel = rgb.get_pixel_mut(column, row);
        for channel_index in 0..3 {
            let source_channel = source_pixel[channel_index] as u16;
            target_pixel[channel_index] =
                ((source_channel * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    rgb
}

/// 解码 AVIF 字节为 `DynamicImage`（`image` crate 未启用 avif 特性时的专用路径）。
fn decode_avif(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    avif::decode(bytes).map_err(|error| error.to_string())
}

fn create_embedded_thumbnail(source_bytes: &[u8]) -> Result<EmbeddedThumbnail, String> {
    let decoded = image::load_from_memory(source_bytes)
        .map_err(|error| format!("生成缩略图时解码封面失败: {}", error))?;
    let rgba = decoded
        .thumbnail(THUMBNAIL_MAX_SIZE, THUMBNAIL_MAX_SIZE)
        .to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let rgb = flatten_alpha_to_rgb(rgba);

    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, THUMBNAIL_JPEG_QUALITY)
        .encode(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .map_err(|error| format!("编码 IMS 缩略图失败: {}", error))?;

    Ok(EmbeddedThumbnail {
        crc32: crc32fast::hash(&bytes),
        bytes,
        width: u16::try_from(width).map_err(|_| "IMS 缩略图宽度超出范围")?,
        height: u16::try_from(height).map_err(|_| "IMS 缩略图高度超出范围")?,
    })
}

fn write_thumbnail_trailer<W: Write>(
    writer: &mut W,
    thumbnail: &EmbeddedThumbnail,
    thumbnail_offset: u32,
) -> Result<(), String> {
    writer
        .write_all(THUMBNAIL_MAGIC)
        .map_err(|error| format!("写入缩略图 magic 失败: {}", error))?;
    writer
        .write_u16::<LittleEndian>(THUMBNAIL_VERSION)
        .map_err(|error| format!("写入缩略图版本失败: {}", error))?;
    writer
        .write_u8(THUMBNAIL_FORMAT_JPEG)
        .map_err(|error| format!("写入缩略图格式失败: {}", error))?;
    writer
        .write_u8(0)
        .map_err(|error| format!("写入缩略图 flags 失败: {}", error))?;
    writer
        .write_u32::<LittleEndian>(thumbnail_offset)
        .map_err(|error| format!("写入缩略图偏移失败: {}", error))?;
    writer
        .write_u32::<LittleEndian>(thumbnail.bytes.len() as u32)
        .map_err(|error| format!("写入缩略图长度失败: {}", error))?;
    writer
        .write_u16::<LittleEndian>(thumbnail.width)
        .map_err(|error| format!("写入缩略图宽度失败: {}", error))?;
    writer
        .write_u16::<LittleEndian>(thumbnail.height)
        .map_err(|error| format!("写入缩略图高度失败: {}", error))?;
    writer
        .write_u32::<LittleEndian>(thumbnail.crc32)
        .map_err(|error| format!("写入缩略图 CRC32 失败: {}", error))?;
    writer
        .write_u32::<LittleEndian>(0)
        .map_err(|error| format!("写入缩略图 reserved 失败: {}", error))?;
    Ok(())
}

fn has_embedded_thumbnail(filepath: &str) -> Result<bool, String> {
    let mut file = std::fs::File::open(filepath)
        .map_err(|error| format!("打开 IMS 文件失败: {}", error))?;
    let file_size = file
        .metadata()
        .map_err(|error| format!("读取 IMS 文件信息失败: {}", error))?
        .len();
    if file_size < 36 {
        return Ok(false);
    }

    file.seek(SeekFrom::End(-36))
        .map_err(|error| format!("定位 IMS 缩略图 Trailer 失败: {}", error))?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)
        .map_err(|error| format!("读取 IMS 缩略图 Trailer 失败: {}", error))?;
    Ok(&magic == THUMBNAIL_MAGIC)
}

/// 图片组：一个封面 + 若干子图，记录来源文件夹
#[derive(Debug)]
pub struct ImageGroup {
    pub base_name: String,
    pub cover: Option<PathBuf>,
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
    #[serde(default)]
    pub merge_existing_ims: bool,
    /// 是否将所有非 JPEG 子图转为 JPEG（q90）
    #[serde(default)]
    pub sub_images_to_jpeg: bool,
}

/// 批量解压配置（从前端传入）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchExtractConfig {
    pub input_dir: String,
    pub depth: u32,
    pub output_dir: String,
    pub preserve_structure: bool,
    pub multi_threaded: bool,
}

/// 批量调整分辨率配置（从前端传入）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResizeConfig {
    pub folders: Vec<FolderConfig>,
    /// 最长边像素下限 64
    pub max_edge: u32,
    pub multi_threaded: bool,
    /// 是否将所有非 JPEG 子图转为 JPEG（q90）
    #[serde(default)]
    pub convert_to_jpeg: bool,
}

/// 单个 IMS 文件的调整结果
#[derive(Debug, Clone)]
pub enum ResizeOutcome {
    /// 文件已重写；`sub_images_resized` 为被缩小的子图数，
    /// `sub_images_converted` 为被转为 JPEG 的非 JPEG 子图数（未勾选转 JPEG 时恒为 0）
    Resized {
        sub_images_resized: usize,
        sub_images_converted: usize,
    },
    /// 全部子图未超标且无需转码，文件未改动
    Skipped,
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
pub fn pack_ims(
    image_paths: &[String],
    cover_index: usize,
    output_path: &str,
    sub_images_to_jpeg: bool,
) -> Result<(), String> {
    let count = image_paths.len();
    if count == 0 {
        return Err("图片列表为空".into());
    }
    if cover_index >= count {
        return Err(format!(
            "封面索引 {} 超出范围 (共 {} 张)",
            cover_index, count
        ));
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
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format: u8 = match ext.as_str() {
            "jpg" | "jpeg" => 0,
            "png" => 1,
            "webp" => 2,
            "avif" => 3,
            _ => {
                return Err(format!(
                    "不支持的图片格式: {} (仅支持 jpg/jpeg/png/webp/avif)",
                    path
                ))
            }
        };
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            if stem.to_lowercase().contains(".ims") {
                return Err(format!("不能将已是 IMS 的文件作为源: {}", path));
            }
        }
        let filename = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let raw_bytes = std::fs::read(path).map_err(|e| format!("读取文件失败 {}: {}", path, e))?;

        let is_cover = i == cover_index;
        // 子图转 JPEG（勾选且非 JPEG）；否则 WebP/AVIF → 自动转 PNG
        let (final_bytes, final_format, final_filename) = if !is_cover
            && sub_images_to_jpeg
            && format != 0
        {
            // 子图转 JPEG：解码（AVIF 用 decode_avif）→ encode_resized 转 JPEG q90
            let decoded = if format == 3 {
                decode_avif(&raw_bytes)
                    .map_err(|e| format!("解码 AVIF {} 失败: {}", path, e))?
            } else {
                image::load_from_memory(&raw_bytes)
                    .map_err(|e| format!("解码 {} 失败: {}", path, e))?
            };
            encode_resized(&decoded, 0, &filename, CONVERT_JPEG_QUALITY)?
        } else if format == 2 || format == 3 {
            let img = image::load_from_memory(&raw_bytes).map_err(|e| {
                format!(
                    "解码 {} 失败: {} (格式: {})",
                    path,
                    e,
                    if format == 2 { "WebP" } else { "AVIF" }
                )
            })?;
            let mut png_bytes = Vec::new();
            img.write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .map_err(|e| format!("PNG 编码失败 {}: {}", path, e))?;
            let new_name = format!(
                "{}.png",
                Path::new(&filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("image")
            );
            (png_bytes, 1u8, new_name)
        } else {
            (raw_bytes, format, filename)
        };

        entries.push(Entry {
            filename: final_filename,
            format: final_format,
            bytes: final_bytes,
            is_cover,
        });
    }

    // 封面排在第一位
    entries.sort_by_key(|e| !e.is_cover);
    let thumbnail = create_embedded_thumbnail(&entries[0].bytes)?;

    // 计算累计偏移量
    let mut offsets: Vec<u32> = Vec::with_capacity(count);
    let mut cumulative: u32 = 0;
    for entry in &entries {
        offsets.push(cumulative);
        cumulative += entry.bytes.len() as u32;
    }
    let thumbnail_offset = cumulative;
    let index_offset = thumbnail_offset
        .checked_add(thumbnail.bytes.len() as u32)
        .ok_or("IMS 文件大小超出格式限制")?;

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
    out.write_all(&thumbnail.bytes)
        .map_err(|e| format!("写入缩略图数据失败: {}", e))?;

    // 2. 写入 index footer
    out.write_all(MAGIC)
        .map_err(|e| format!("写入 magic 失败: {}", e))?;
    out.write_u16::<LittleEndian>(VERSION)
        .map_err(|e| format!("写入 version 失败: {}", e))?;
    out.write_u32::<LittleEndian>(count as u32)
        .map_err(|e| format!("写入 image_count 失败: {}", e))?;
    out.write_u32::<LittleEndian>(0)
        .map_err(|e| format!("写入 data_start 失败: {}", e))?;

    // 3. 写入每个 entry (v2: 带 SHA-256 hash)
    for (i, entry) in entries.iter().enumerate() {
        let name_bytes = entry.filename.as_bytes();
        out.write_u16::<LittleEndian>(name_bytes.len() as u16)
            .map_err(|e| format!("写入 name_len 失败: {}", e))?;
        out.write_all(name_bytes)
            .map_err(|e| format!("写入 name 失败: {}", e))?;
        out.write_u8(entry.format)
            .map_err(|e| format!("写入 format 失败: {}", e))?;
        out.write_u32::<LittleEndian>(offsets[i])
            .map_err(|e| format!("写入 offset 失败: {}", e))?;
        out.write_u32::<LittleEndian>(entry.bytes.len() as u32)
            .map_err(|e| format!("写入 length 失败: {}", e))?;
        out.write_u8(if entry.is_cover { 1 } else { 0 })
            .map_err(|e| format!("写入 is_cover 失败: {}", e))?;

        // v2: SHA-256 hash
        let mut hasher = Sha256::new();
        hasher.update(&entry.bytes);
        let hash = hasher.finalize();
        out.write_u8(HASH_ALGO_SHA256)
            .map_err(|e| format!("写入 hash_algo 失败: {}", e))?;
        out.write_all(&hash)
            .map_err(|e| format!("写入 hash 失败: {}", e))?;
    }

    write_thumbnail_trailer(&mut out, &thumbnail, thumbnail_offset)?;

    // 4. 写入 index_offset
    out.write_u32::<LittleEndian>(index_offset)
        .map_err(|e| format!("写入 index_offset 失败: {}", e))?;

    Ok(())
}

// ---- 文件夹扫描与分组 ----

/// 扫描文件夹，将图片按 base_name 分组
pub fn scan_and_group(input_folder: &str, max_depth: u32) -> Result<Vec<ImageGroup>, String> {
    scan_and_group_with_options(input_folder, max_depth, false)
}

pub fn scan_and_group_with_options(
    input_folder: &str,
    max_depth: u32,
    allow_subimage_only: bool,
) -> Result<Vec<ImageGroup>, String> {
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
    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let lower = ext.to_lowercase();
            if lower == "jpg"
                || lower == "jpeg"
                || lower == "png"
                || lower == "webp"
                || lower == "avif"
            {
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
            let entry = groups
                .entry(base_name)
                .or_insert_with(|| (None, Vec::new(), None));
            entry.1.push((seq, path.clone()));
            if entry.2.is_none() {
                entry.2 = Some(parent_dir.clone());
            }
        } else {
            // 封面: {base}.{ext}
            let base_name = stem.to_string();
            let entry = groups
                .entry(base_name)
                .or_insert_with(|| (None, Vec::new(), None));
            entry.0 = Some(path.clone());
            if entry.2.is_none() {
                entry.2 = Some(parent_dir.clone());
            }
        }
    }

    // 筛选有效组（有封面 + 有子图）
    let mut result: Vec<ImageGroup> = Vec::new();
    for (base_name, (cover_opt, mut sub_images, source_dir_opt)) in groups {
        if sub_images.is_empty() {
            continue;
        }
        let cover = match cover_opt {
            Some(c) => Some(c),
            None if !allow_subimage_only => {
                // 无封面时，尝试用 00001 号子图替代
                if let Some(pos) = sub_images.iter().position(|(seq, _)| *seq == 1) {
                    let (_, cover_path) = sub_images.remove(pos);
                    Some(cover_path)
                } else {
                    continue;
                }
            }
            None => None,
        };
        sub_images.sort_by_key(|(seq, _)| *seq);

        let ext_source = cover
            .as_ref()
            .or_else(|| sub_images.first().map(|(_, path)| path));
        let Some(ext_source) = ext_source else {
            continue;
        };

        let raw_ext = ext_source
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

/// 将旧版 IMS 文件升级到 v2，并为缺少缩略图的 v2 文件补写 Trailer。
pub fn upgrade_ims_file(filepath: &str) -> Result<(), String> {
    use crate::ims::{parse_ims, read_sub_image};

    let ims_info = parse_ims(filepath)?;

    // 已经是 v2 且包含内嵌缩略图，跳过
    if ims_info.version >= 2 && has_embedded_thumbnail(filepath)? {
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
    std::fs::remove_file(filepath).map_err(|e| format!("删除旧文件失败: {}", e))?;
    std::fs::rename(&tmp_path, filepath).map_err(|e| format!("替换文件失败: {}", e))?;

    Ok(())
}

/// 就地调整 IMS 文件中子图的分辨率，可选将非 JPEG 子图转为 JPEG。
///
/// 逐子图判断：若子图原始最长边 `> max_edge`，按保持长宽比缩小到最长边等于
/// `max_edge`（`Lanczos3`）并重新编码；未超标子图默认保留原始字节。
/// 当 `convert_to_jpeg` 为 true 时，所有非 JPEG 子图（PNG/WebP/AVIF）无论是否
/// 超标都重新编码为 JPEG（q90）；已是 JPEG 且未超标的子图保留原字节不动。
/// AVIF 子图用 `avif` 解码；未勾选转 JPEG 时 AVIF 转 PNG，勾选时转 JPEG。
/// 只要任一子图被缩小或转码，整个 IMS 文件会通过临时文件 + rename 原子重写；
/// 若全部子图未超标且无需转码返回 `Skipped`。
/// 任一子图解码/编码失败则整文件跳过（返回 `Err`，原文件保持完好）。
pub fn resize_ims_file(
    filepath: &str,
    max_edge: u32,
    convert_to_jpeg: bool,
) -> Result<ResizeOutcome, String> {
    use crate::ims::{parse_ims, read_sub_image};

    let ims_info = parse_ims(filepath)?;

    let mut entries: Vec<UpEntry> = Vec::with_capacity(ims_info.images.len());
    let mut resized_count: usize = 0;
    let mut converted_count: usize = 0;

    for img in &ims_info.images {
        let bytes = read_sub_image(filepath, img.data_offset, img.data_length)?;
        let is_jpeg = img.format == 0;

        // 解码：AVIF 走 avif crate，其余走 image crate
        let decoded = if img.format == 3 {
            decode_avif(&bytes)
                .map_err(|e| format!("解码 AVIF 子图 {} 失败: {}", img.filename, e))?
        } else {
            image::load_from_memory(&bytes)
                .map_err(|e| format!("解码子图 {} 失败: {}", img.filename, e))?
        };

        let longest = decoded.width().max(decoded.height());
        let oversized = longest > max_edge;

        // 未超标且（已是 JPEG 或未勾选转码）→ 保留原字节
        if !oversized && (is_jpeg || !convert_to_jpeg) {
            entries.push(UpEntry {
                filename: img.filename.clone(),
                format: img.format,
                bytes,
                is_cover: img.is_cover,
            });
            continue;
        }

        // 需要重编码：确定目标格式
        // 勾选转 JPEG → 统一 JPEG(0)；未勾选 → AVIF 转 PNG(1)，其余保持原格式
        let target_format: u8 = if convert_to_jpeg {
            0
        } else if img.format == 3 {
            1
        } else {
            img.format
        };

        // 确定待编码的图（超标则先缩小，未超标转码则图不变尺寸）
        let image_to_encode = if oversized {
            let scale = max_edge as f64 / longest as f64;
            let new_w = std::cmp::max(1, (decoded.width() as f64 * scale).round() as u32);
            let new_h = std::cmp::max(1, (decoded.height() as f64 * scale).round() as u32);
            decoded.resize_exact(new_w, new_h, image::imageops::FilterType::CatmullRom)
        } else {
            decoded
        };

        // JPEG 质量：原 JPEG 子图用 88，非 JPEG 转 JPEG 用 90
        let jpeg_quality = if is_jpeg {
            RESIZE_JPEG_QUALITY
        } else {
            CONVERT_JPEG_QUALITY
        };

        let (final_bytes, final_format, final_filename) =
            encode_resized(&image_to_encode, target_format, &img.filename, jpeg_quality)?;

        entries.push(UpEntry {
            filename: final_filename,
            format: final_format,
            bytes: final_bytes,
            is_cover: img.is_cover,
        });
        if oversized {
            resized_count += 1;
        }
        if convert_to_jpeg && !is_jpeg {
            converted_count += 1;
        }
    }

    if resized_count == 0 && converted_count == 0 {
        return Ok(ResizeOutcome::Skipped);
    }

    // 封面排第一
    entries.sort_by_key(|e| !e.is_cover);

    let tmp_path = format!("{}.tmp_resize", filepath);
    write_ims_entries(&tmp_path, &entries)
        .map_err(|e| {
            // 清理失败的临时文件
            let _ = std::fs::remove_file(&tmp_path);
            e
        })?;
    std::fs::remove_file(filepath).map_err(|e| format!("删除旧文件失败: {}", e))?;
    std::fs::rename(&tmp_path, filepath).map_err(|e| format!("替换文件失败: {}", e))?;

    Ok(ResizeOutcome::Resized {
        sub_images_resized: resized_count,
        sub_images_converted: converted_count,
    })
}

/// 将文件名的扩展名替换为 `.png`（用于 AVIF 子图转 PNG 后的命名）。
fn with_png_extension(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    format!("{}.png", stem)
}

/// 将文件名的扩展名替换为 `.jpg`（用于子图转 JPEG 后的命名）。
fn with_jpg_extension(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    format!("{}.jpg", stem)
}

/// 将 `DynamicImage` 按目标格式编码。
///
/// - JPEG(0)：若带 alpha 先合成到白底，再用 `JpegEncoder` 以 `jpeg_quality` 编码；
///   扩展名改为 `.jpg`
/// - PNG(1)：直接 `write_to` PNG（保留 alpha）；扩展名改为 `.png`
/// - 其余 `out_format` 值统一按 PNG 编码（与现有 WebP/AVIF→PNG 惯例一致）
fn encode_resized(
    image: &image::DynamicImage,
    out_format: u8,
    out_filename: &str,
    jpeg_quality: u8,
) -> Result<(Vec<u8>, u8, String), String> {
    match out_format {
        0 => {
            // JPEG：无 alpha 支持，先 flatten
            let rgb = if matches!(image.color(), image::ColorType::Rgba8) {
                flatten_alpha_to_rgb(image.to_rgba8())
            } else {
                image.to_rgb8()
            };
            let (w, h) = (rgb.width(), rgb.height());
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, jpeg_quality)
                .encode(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG 编码 {} 失败: {}", out_filename, e))?;
            Ok((bytes, 0u8, with_jpg_extension(out_filename)))
        }
        _ => {
            // PNG（含原 PNG 与 AVIF 转码）：快速压缩，体积略增换数倍编码速度
            let mut bytes = Vec::new();
            let rgba = image.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            image::codecs::png::PngEncoder::new_with_quality(
                &mut bytes,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            )
            .write_image(rgba.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .map_err(|e| format!("PNG 编码 {} 失败: {}", out_filename, e))?;
            Ok((bytes, 1u8, with_png_extension(out_filename)))
        }
    }
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
            let name = e
                .path()
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            [
                ".ims",
                ".ims.jpg",
                ".ims.jpeg",
                ".ims.png",
                ".ims.webp",
                ".ims.avif",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .collect()
}

/// 直接写入 IMS v2 文件（内部用，接受已读取的字节数据）
fn write_ims_entries(output_path: &str, entries: &[UpEntry]) -> Result<(), String> {
    if entries.is_empty() {
        return Err("图片列表为空".into());
    }
    let count = entries.len();
    let mut offsets: Vec<u32> = Vec::with_capacity(count);
    let mut cumulative: u32 = 0;
    for e in entries {
        offsets.push(cumulative);
        cumulative += e.bytes.len() as u32;
    }
    let cover = entries
        .iter()
        .find(|entry| entry.is_cover)
        .unwrap_or(&entries[0]);
    let thumbnail = create_embedded_thumbnail(&cover.bytes)?;
    let thumbnail_offset = cumulative;
    let index_offset = thumbnail_offset
        .checked_add(thumbnail.bytes.len() as u32)
        .ok_or("IMS 文件大小超出格式限制")?;

    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let mut out = std::fs::File::create(output_path).map_err(|e| format!("创建文件失败: {}", e))?;

    for e in entries {
        out.write_all(&e.bytes)
            .map_err(|e| format!("写入数据失败: {}", e))?;
    }
    out.write_all(&thumbnail.bytes)
        .map_err(|e| format!("写入缩略图数据失败: {}", e))?;

    out.write_all(MAGIC)
        .map_err(|e| format!("写入 magic 失败: {}", e))?;
    out.write_u16::<LittleEndian>(VERSION)
        .map_err(|e| format!("写入 version 失败: {}", e))?;
    out.write_u32::<LittleEndian>(count as u32)
        .map_err(|e| format!("写入 count 失败: {}", e))?;
    out.write_u32::<LittleEndian>(0)
        .map_err(|e| format!("写入 data_start 失败: {}", e))?;

    for (i, e) in entries.iter().enumerate() {
        let name_bytes = e.filename.as_bytes();
        out.write_u16::<LittleEndian>(name_bytes.len() as u16)
            .map_err(|e| format!("写入 name_len 失败: {}", e))?;
        out.write_all(name_bytes)
            .map_err(|e| format!("写入 name 失败: {}", e))?;
        out.write_u8(e.format)
            .map_err(|e| format!("写入 format 失败: {}", e))?;
        out.write_u32::<LittleEndian>(offsets[i])
            .map_err(|e| format!("写入 offset 失败: {}", e))?;
        out.write_u32::<LittleEndian>(e.bytes.len() as u32)
            .map_err(|e| format!("写入 length 失败: {}", e))?;
        out.write_u8(if e.is_cover { 1 } else { 0 })
            .map_err(|e| format!("写入 is_cover 失败: {}", e))?;

        let mut hasher = Sha256::new();
        hasher.update(&e.bytes);
        let hash = hasher.finalize();
        out.write_u8(HASH_ALGO_SHA256)
            .map_err(|e| format!("写入 hash_algo 失败: {}", e))?;
        out.write_all(&hash)
            .map_err(|e| format!("写入 hash 失败: {}", e))?;
    }

    write_thumbnail_trailer(&mut out, &thumbnail, thumbnail_offset)?;

    out.write_u32::<LittleEndian>(index_offset)
        .map_err(|e| format!("写入 index_offset 失败: {}", e))?;

    Ok(())
}

#[derive(Debug, Clone)]
pub struct UpEntry {
    pub filename: String,
    pub format: u8,
    pub bytes: Vec<u8>,
    pub is_cover: bool,
}

pub fn load_image_as_entry(path: &Path, is_cover: bool, to_jpeg: bool) -> Result<UpEntry, String> {
    if !path.exists() {
        return Err(format!("Source file does not exist: {}", path.display()));
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let format: u8 = match ext.as_str() {
        "jpg" | "jpeg" => 0,
        "png" => 1,
        "webp" => 2,
        "avif" => 3,
        _ => {
            return Err(format!(
                "Unsupported image format: {} (jpg/jpeg/png/webp/avif only)",
                path.display()
            ))
        }
    };

    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        if stem.to_lowercase().contains(".ims") {
            return Err(format!(
                "Cannot use an existing IMS file as source: {}",
                path.display()
            ));
        }
    }

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let raw_bytes = std::fs::read(path)
        .map_err(|e| format!("Failed to read source file {}: {}", path.display(), e))?;

    let (bytes, final_format, final_filename) = if to_jpeg && format != 0 {
        // 转 JPEG：解码（AVIF 用 decode_avif）→ encode_resized 转 JPEG q90
        let decoded = if format == 3 {
            decode_avif(&raw_bytes)
                .map_err(|e| format!("Failed to decode AVIF {}: {}", path.display(), e))?
        } else {
            image::load_from_memory(&raw_bytes)
                .map_err(|e| format!("Failed to decode {}: {}", path.display(), e))?
        };
        encode_resized(&decoded, 0, &filename, CONVERT_JPEG_QUALITY)?
    } else if format == 2 || format == 3 {
        let img = image::load_from_memory(&raw_bytes).map_err(|e| {
            format!(
                "Failed to decode {}: {} ({})",
                path.display(),
                e,
                if format == 2 { "WebP" } else { "AVIF" }
            )
        })?;
        let mut png_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("Failed to encode PNG {}: {}", path.display(), e))?;
        let new_name = format!(
            "{}.png",
            Path::new(&filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("image")
        );
        (png_bytes, 1u8, new_name)
    } else {
        (raw_bytes, format, filename)
    };

    Ok(UpEntry {
        filename: final_filename,
        format: final_format,
        bytes,
        is_cover,
    })
}

/// 原子写入 IMS 文件：先写临时文件，成功后再替换目标文件
pub fn save_ims_entries_atomic(output_path: &str, entries: &[UpEntry]) -> Result<(), String> {
    if entries.is_empty() {
        return Err("图片列表为空".into());
    }

    // 仅剩封面 → 存为普通图片（不加 IMS 索引）
    if entries.len() == 1 {
        if let Some(parent) = Path::new(output_path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let tmp = format!("{}.tmp", output_path);
        std::fs::write(&tmp, &entries[0].bytes).map_err(|e| format!("写入临时文件失败: {}", e))?;
        // 原子替换
        if Path::new(output_path).exists() {
            std::fs::remove_file(output_path).map_err(|e| format!("删除旧文件失败: {}", e))?;
        }
        std::fs::rename(&tmp, output_path).map_err(|e| format!("重命名临时文件失败: {}", e))?;
        return Ok(());
    }

    let tmp = format!("{}.tmp", output_path);
    write_ims_entries(&tmp, entries)?;

    // 原子替换：先删后改名
    if Path::new(output_path).exists() {
        std::fs::remove_file(output_path).map_err(|e| format!("删除旧文件失败: {}", e))?;
    }
    std::fs::rename(&tmp, output_path).map_err(|e| format!("重命名临时文件失败: {}", e))?;

    Ok(())
}

/// 扫描多个文件夹，汇总所有 ImageGroup
pub fn scan_multiple_folders(folders: &[FolderConfig]) -> Result<Vec<ImageGroup>, String> {
    scan_multiple_folders_with_options(folders, false)
}

pub fn scan_multiple_folders_with_options(
    folders: &[FolderConfig],
    allow_subimage_only: bool,
) -> Result<Vec<ImageGroup>, String> {
    let mut all_groups = Vec::new();
    for folder in folders {
        let groups = if allow_subimage_only {
            scan_and_group_with_options(&folder.path, folder.depth, true)?
        } else {
            scan_and_group(&folder.path, folder.depth)?
        };
        all_groups.extend(groups);
    }
    Ok(all_groups)
}

/// 计算输出路径
///
/// 若指定了 output_dir，则按输入目录结构镜像输出：
/// 例如 input="Input1/tex0", output_dir="Output"：
/// - "Input1/tex0/file.jpg" → "Output/file.ims.jpg"
/// - "Input1/tex0/sub/file.jpg" → "Output/sub/file.ims.jpg"
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
            // 所选输入目录本身对应输出根目录，只有其子目录会被保留。
            let input_base = Path::new(&group.input_folder);
            let rel = group
                .source_dir
                .strip_prefix(input_base)
                .unwrap_or(&group.source_dir);
            let out_path = Path::new(out_dir).join(rel).join(&output_filename);
            out_path.to_string_lossy().to_string()
        } else {
            Path::new(out_dir)
                .join(&output_filename)
                .to_string_lossy()
                .to_string()
        }
    } else {
        group
            .source_dir
            .join(&output_filename)
        .to_string_lossy()
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ims::{parse_ims, read_sub_image};
    use image::ImageEncoder;
    use tempfile::tempdir;

    fn group_in(source_dir: &str) -> ImageGroup {
        ImageGroup {
            base_name: "0".into(),
            cover: None,
            sub_images: Vec::new(),
            cover_ext: "jpg".into(),
            source_dir: PathBuf::from(source_dir),
            input_folder: "input".into(),
        }
    }

    #[test]
    fn output_root_matches_selected_input_folder() {
        let output_dir = Some("output".into());
        let output = compute_output_path(&group_in("input"), &output_dir, true);

        assert_eq!(output, Path::new("output").join("0.ims.jpg").to_string_lossy());
    }

    #[test]
    fn output_keeps_only_child_directories_of_selected_input_folder() {
        let output_dir = Some("output".into());
        let output = compute_output_path(&group_in("input/1"), &output_dir, true);

        assert_eq!(
            output,
            Path::new("output")
                .join("1")
                .join("0.ims.jpg")
                .to_string_lossy()
        );
    }

    #[test]
    fn embedded_thumbnail_trailer_contains_valid_metadata() {
        let source = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            480,
            320,
            image::Rgba([40, 80, 120, 128]),
        ));
        let mut source_bytes = Vec::new();
        source
            .write_to(
                &mut std::io::Cursor::new(&mut source_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();

        let thumbnail = create_embedded_thumbnail(&source_bytes).unwrap();
        let mut trailer = Vec::new();
        write_thumbnail_trailer(&mut trailer, &thumbnail, 1234).unwrap();

        assert_eq!(trailer.len(), 32);
        assert_eq!(&trailer[0..8], THUMBNAIL_MAGIC);
        assert_eq!(u16::from_le_bytes(trailer[8..10].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(trailer[12..16].try_into().unwrap()), 1234);
        assert_eq!(
            u32::from_le_bytes(trailer[16..20].try_into().unwrap()),
            thumbnail.bytes.len() as u32
        );
        assert_eq!(u32::from_le_bytes(trailer[24..28].try_into().unwrap()), thumbnail.crc32);
        assert_eq!(thumbnail.crc32, crc32fast::hash(&thumbnail.bytes));
        assert!(thumbnail.width <= THUMBNAIL_MAX_SIZE as u16);
        assert!(thumbnail.height <= THUMBNAIL_MAX_SIZE as u16);
    }

    /// 构造一个含指定子图的 IMS 临时文件，返回 (临时目录, ims 路径)。
    fn make_ims_with_sub_images(sub_images: &[(u32, u32, u8)]) -> (tempfile::TempDir, String) {
        // sub_images: [(width, height, format)]，format 0=JPEG,1=PNG
        let dir = tempdir().unwrap();
        let mut entries: Vec<UpEntry> = Vec::new();
        for (i, &(w, h, fmt)) in sub_images.iter().enumerate() {
            let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                w, h, image::Rgb([100, 120, 140]),
            ));
            let mut bytes = Vec::new();
            let (ext, format_byte) = if fmt == 0 {
                img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
                    .unwrap();
                ("jpg", 0u8)
            } else {
                img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                    .unwrap();
                ("png", 1u8)
            };
            let filename = if i == 0 {
                format!("cover.{}", ext)
            } else {
                format!("cover_{:05}_.{}", i, ext)
            };
            entries.push(UpEntry {
                filename,
                format: format_byte,
                bytes,
                is_cover: i == 0,
            });
        }
        let ims_path = dir.path().join("test.ims.jpg").to_string_lossy().to_string();
        // 用 write_ims_entries 写出 IMS 容器
        write_ims_entries(&ims_path, &entries).unwrap();
        (dir, ims_path)
    }

    #[test]
    fn resize_skips_when_all_sub_images_within_max_edge() {
        let (_dir, ims_path) = make_ims_with_sub_images(&[(100, 100, 0)]);
        let original_bytes = std::fs::read(&ims_path).unwrap();
        let outcome = resize_ims_file(&ims_path, 200, false).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Skipped));
        // 文件字节未变
        assert_eq!(std::fs::read(&ims_path).unwrap(), original_bytes);
    }

    #[test]
    fn resize_resizes_oversized_jpeg_and_preserves_aspect() {
        let (_dir, ims_path) = make_ims_with_sub_images(&[(800, 600, 0)]);
        let outcome = resize_ims_file(&ims_path, 400, false).unwrap();
        match outcome {
            ResizeOutcome::Resized { sub_images_resized, .. } => assert_eq!(sub_images_resized, 1),
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        // 800×600 → longest=800 → scale=0.5 → 400×300
        assert_eq!((decoded.width(), decoded.height()), (400, 300));
    }

    #[test]
    fn resize_preserves_undersized_sub_image_bytes() {
        // 两张子图：800×600 超标 + 100×100 未超标
        let (_dir, ims_path) = make_ims_with_sub_images(&[(800, 600, 0), (100, 100, 1)]);
        // 记录未超标子图的原始字节（index 1）
        let info_before = parse_ims(&ims_path).unwrap();
        let small_before =
            read_sub_image(&ims_path, info_before.images[1].data_offset, info_before.images[1].data_length)
                .unwrap();
        resize_ims_file(&ims_path, 400, false).unwrap();
        // 重写后重新解析，未超标子图字节应与原始一致
        let info_after = parse_ims(&ims_path).unwrap();
        let small_after =
            read_sub_image(&ims_path, info_after.images[1].data_offset, info_after.images[1].data_length)
                .unwrap();
        assert_eq!(small_before, small_after);
    }

    #[test]
    fn resize_returns_err_on_corrupt_sub_image() {
        let (_dir, ims_path) = make_ims_with_sub_images(&[(800, 600, 0)]);
        // 破坏文件：把封面字节区改坏（在数据区写垃圾）
        let mut data = std::fs::read(&ims_path).unwrap();
        // 向数据区前 100 字节写垃圾（保留尾部索引，使 parse_ims 仍成功但解码失败）
        for b in &mut data[0..100] {
            *b = 0xFF;
        }
        std::fs::write(&ims_path, &data).unwrap();
        let result = resize_ims_file(&ims_path, 400, false);
        assert!(result.is_err(), "corrupt sub-image should error whole file");
    }

    #[test]
    fn resize_preserves_alpha_for_png_sub_image() {
        // 构造一张 800×600 的 RGBA PNG（含半透明像素）作为子图，
        // 验证 PNG 分支在缩小后保留 alpha（color 仍为 Rgba8）。
        let dir = tempdir().unwrap();
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            800,
            600,
            image::Rgba([100, 120, 140, 128]),
        ));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let entries = vec![UpEntry {
            filename: "cover.png".into(),
            format: 1u8,
            bytes,
            is_cover: true,
        }];
        let ims_path = dir
            .path()
            .join("test.ims.png")
            .to_string_lossy()
            .to_string();
        write_ims_entries(&ims_path, &entries).unwrap();

        let outcome = resize_ims_file(&ims_path, 400, false).unwrap();
        match outcome {
            ResizeOutcome::Resized { sub_images_resized, .. } => {
                assert_eq!(sub_images_resized, 1);
            }
            _ => panic!("expected Resized, got {:?}", outcome),
        }

        let info = parse_ims(&ims_path).unwrap();
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (400, 300));
        assert_eq!(
            decoded.color(),
            image::ColorType::Rgba8,
            "PNG sub-image should preserve alpha after resize"
        );
    }

    /// 构造一个含单张指定尺寸/格式/是否带 alpha 子图的 IMS，返回 (临时目录, ims 路径)。
    fn make_ims_single(format: u8, w: u32, h: u32, rgba: bool) -> (tempfile::TempDir, String) {
        let dir = tempdir().unwrap();
        let (bytes, ext) = match (format, rgba) {
            (0, _) => {
                let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    w, h, image::Rgb([100, 120, 140]),
                ));
                let mut b = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Jpeg)
                    .unwrap();
                (b, "jpg")
            }
            (1, true) => {
                let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    w, h, image::Rgba([100, 120, 140, 128]),
                ));
                let mut b = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png)
                    .unwrap();
                (b, "png")
            }
            (1, false) => {
                let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    w, h, image::Rgb([100, 120, 140]),
                ));
                let mut b = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png)
                    .unwrap();
                (b, "png")
            }
            _ => panic!("unsupported test format"),
        };
        let entries = vec![UpEntry {
            filename: format!("cover.{}", ext),
            format,
            bytes,
            is_cover: true,
        }];
        let ims_path = dir
            .path()
            .join(format!("test.ims.{}", ext))
            .to_string_lossy()
            .to_string();
        write_ims_entries(&ims_path, &entries).unwrap();
        (dir, ims_path)
    }

    #[test]
    fn convert_to_jpeg_converts_undersized_png() {
        // 未超标 PNG + convert_to_jpeg=true → 转 JPEG（不缩小）
        let (_dir, ims_path) = make_ims_single(1, 100, 100, false);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 0);
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        assert_eq!(info.images[0].format, 0, "应转为 JPEG");
        assert!(info.images[0].filename.ends_with(".jpg"), "扩展名应为 .jpg");
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 100), "未超标不缩小");
    }

    #[test]
    fn convert_to_jpeg_preserves_undersized_jpeg() {
        // 未超标 JPEG + convert_to_jpeg=true → Skipped（已是 JPEG 不二次编码）
        let (_dir, ims_path) = make_ims_single(0, 100, 100, false);
        let original = std::fs::read(&ims_path).unwrap();
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Skipped));
        assert_eq!(std::fs::read(&ims_path).unwrap(), original, "文件不应改动");
    }

    #[test]
    fn convert_to_jpeg_oversized_png_resizes_and_converts() {
        // 超标 PNG + convert_to_jpeg=true → 缩小 + 转 JPEG
        let (_dir, ims_path) = make_ims_single(1, 800, 600, false);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 1);
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        assert_eq!(info.images[0].format, 0);
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (400, 300));
    }

    #[test]
    fn no_convert_keeps_undersized_png() {
        // 未超标 PNG + convert_to_jpeg=false → Skipped（回归：未勾选行为不变）
        let (_dir, ims_path) = make_ims_single(1, 100, 100, false);
        let original = std::fs::read(&ims_path).unwrap();
        let outcome = resize_ims_file(&ims_path, 400, false).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Skipped));
        assert_eq!(std::fs::read(&ims_path).unwrap(), original);
    }

    #[test]
    fn convert_to_jpeg_flattens_alpha() {
        // 带 alpha 的 PNG + convert_to_jpeg=true → 转 JPEG 后无 alpha
        let (_dir, ims_path) = make_ims_single(1, 800, 600, true);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Resized { .. }));
        let info = parse_ims(&ims_path).unwrap();
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_ne!(
            decoded.color(),
            image::ColorType::Rgba8,
            "转 JPEG 后 alpha 应被合成"
        );
    }

    /// 构造一个含 JPEG 封面 + 单张 AVIF 子图的 IMS，返回 (临时目录, ims 路径)。
    /// 封面用 JPEG 是因为 `create_embedded_thumbnail` 走 `image::load_from_memory`，
    /// 当前 `image` crate 未启用 avif 特性，无法从 AVIF 字节生成缩略图。
    fn make_ims_jpeg_cover_with_avif_sub(
        cover_w: u32,
        cover_h: u32,
        avif_w: u32,
        avif_h: u32,
    ) -> (tempfile::TempDir, String) {
        let dir = tempdir().unwrap();

        // JPEG 封面
        let cover_img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            cover_w,
            cover_h,
            image::Rgb([200, 100, 50]),
        ));
        let mut cover_bytes = Vec::new();
        cover_img
            .write_to(
                &mut std::io::Cursor::new(&mut cover_bytes),
                image::ImageFormat::Jpeg,
            )
            .unwrap();

        // AVIF 子图
        let avif_img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            avif_w,
            avif_h,
            image::Rgb([100, 120, 140]),
        ));
        let rgba = avif_img.to_rgba8();
        let mut avif_bytes = Vec::new();
        image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut avif_bytes, 10, 100)
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();

        let entries = vec![
            UpEntry {
                filename: "cover.jpg".into(),
                format: 0u8,
                bytes: cover_bytes,
                is_cover: true,
            },
            UpEntry {
                filename: "sub.avif".into(),
                format: 3u8,
                bytes: avif_bytes,
                is_cover: false,
            },
        ];
        let ims_path = dir
            .path()
            .join("test.ims.avif")
            .to_string_lossy()
            .to_string();
        write_ims_entries(&ims_path, &entries).unwrap();
        (dir, ims_path)
    }

    #[test]
    fn convert_to_jpeg_converts_undersized_avif() {
        // 未超标 AVIF 子图 + convert_to_jpeg=true → 转 JPEG（decode_avif 路径）
        let (_dir, ims_path) = make_ims_jpeg_cover_with_avif_sub(50, 50, 100, 100);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 0);
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        // 找到原 AVIF 子图（非封面）
        let avif_sub = info
            .images
            .iter()
            .find(|i| !i.is_cover)
            .expect("应有非封面子图");
        assert_eq!(avif_sub.format, 0, "AVIF 应转为 JPEG");
        assert!(avif_sub.filename.ends_with(".jpg"));
        let sub = read_sub_image(&ims_path, avif_sub.data_offset, avif_sub.data_length).unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 100));
    }

    /// 构造一个含单张 WebP 子图的 IMS，返回 (临时目录, ims 路径)。
    fn make_ims_single_webp(w: u32, h: u32) -> (tempfile::TempDir, String) {
        let dir = tempdir().unwrap();
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            w,
            h,
            image::Rgb([100, 120, 140]),
        ));
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::WebP)
            .unwrap();
        let entries = vec![UpEntry {
            filename: "cover.webp".into(),
            format: 2u8,
            bytes,
            is_cover: true,
        }];
        let ims_path = dir
            .path()
            .join("test.ims.webp")
            .to_string_lossy()
            .to_string();
        write_ims_entries(&ims_path, &entries).unwrap();
        (dir, ims_path)
    }

    #[test]
    fn convert_to_jpeg_converts_undersized_webp() {
        // 未超标 WebP + convert_to_jpeg=true → 转 JPEG
        let (_dir, ims_path) = make_ims_single_webp(100, 100);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 0);
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        assert_eq!(info.images[0].format, 0, "WebP 应转为 JPEG");
        assert!(info.images[0].filename.ends_with(".jpg"));
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 100));
    }

    /// 在临时目录写一张指定尺寸/格式的图片文件，返回 (临时目录, 文件路径)。
    fn write_temp_image(dir: &tempfile::TempDir, name: &str, w: u32, h: u32, rgba: bool) -> String {
        let path = dir.path().join(name);
        if rgba {
            let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                w, h, image::Rgba([100, 120, 140, 128]),
            ));
            img.save(&path).unwrap();
        } else {
            let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                w, h, image::Rgb([100, 120, 140]),
            ));
            img.save(&path).unwrap();
        }
        path.to_string_lossy().to_string()
    }

    #[test]
    fn pack_ims_converts_sub_image_png_to_jpeg() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.png", 200, 200, false);
        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        // 封面（index 0）保持 JPEG，子图（index 1）转 JPEG
        assert_eq!(info.images[0].format, 0, "封面应保持 JPEG");
        assert_eq!(info.images[1].format, 0, "子图应转为 JPEG");
        assert!(info.images[1].filename.ends_with(".jpg"), "子图扩展名应为 .jpg");
    }

    #[test]
    fn pack_ims_preserves_jpeg_sub_image_when_converting() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.jpg", 200, 200, false);
        let sub_original = std::fs::read(&sub).unwrap();
        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        let sub_bytes = crate::ims::read_sub_image(
            &output,
            info.images[1].data_offset,
            info.images[1].data_length,
        )
        .unwrap();
        assert_eq!(sub_bytes, sub_original, "已是 JPEG 的子图不应二次编码");
    }

    #[test]
    fn pack_ims_no_convert_keeps_png_sub_image() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.png", 200, 200, false);
        let output = dir.path().join("out.ims.png").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, false).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        assert_eq!(info.images[1].format, 1, "未勾选时 PNG 子图应保持 PNG");
    }

    #[test]
    fn pack_ims_convert_sub_image_flattens_alpha() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.png", 200, 200, true);
        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        let sub_bytes = crate::ims::read_sub_image(
            &output,
            info.images[1].data_offset,
            info.images[1].data_length,
        )
        .unwrap();
        let decoded = image::load_from_memory(&sub_bytes).unwrap();
        assert_ne!(
            decoded.color(),
            image::ColorType::Rgba8,
            "转 JPEG 后 alpha 应被合成"
        );
    }

    #[test]
    fn pack_ims_converts_sub_image_webp_to_jpeg() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        // image crate 已启用 webp 特性，可直接通过扩展名保存
        let sub = write_temp_image(&dir, "cover_00001_.webp", 200, 200, false);
        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        assert_eq!(info.images[0].format, 0, "封面应保持 JPEG");
        assert_eq!(info.images[1].format, 0, "WebP 子图应转为 JPEG");
        assert!(
            info.images[1].filename.ends_with(".jpg"),
            "子图扩展名应为 .jpg"
        );
    }

    #[test]
    fn pack_ims_converts_sub_image_avif_to_jpeg() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);

        // 构造 AVIF 子图字节（复用 make_ims_jpeg_cover_with_avif_sub 的编码方式）
        let avif_img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            200,
            200,
            image::Rgb([100, 120, 140]),
        ));
        let rgba = avif_img.to_rgba8();
        let mut avif_bytes = Vec::new();
        image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut avif_bytes, 10, 100)
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let sub_path = dir.path().join("cover_00001_.avif");
        std::fs::write(&sub_path, &avif_bytes).unwrap();
        let sub = sub_path.to_string_lossy().to_string();

        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        assert_eq!(info.images[0].format, 0, "封面应保持 JPEG");
        assert_eq!(info.images[1].format, 0, "AVIF 子图应转为 JPEG");
        assert!(
            info.images[1].filename.ends_with(".jpg"),
            "子图扩展名应为 .jpg"
        );
    }

    #[test]
    fn load_image_as_entry_to_jpeg_converts_png() {
        let dir = tempdir().unwrap();
        let png_path = write_temp_image(&dir, "img.png", 100, 100, false);
        let entry = load_image_as_entry(std::path::Path::new(&png_path), false, true).unwrap();
        assert_eq!(entry.format, 0, "应转为 JPEG");
        assert!(entry.filename.ends_with(".jpg"), "扩展名应为 .jpg");
    }

    #[test]
    fn load_image_as_entry_to_jpeg_preserves_jpeg() {
        let dir = tempdir().unwrap();
        let jpg_path = write_temp_image(&dir, "img.jpg", 100, 100, false);
        let original = std::fs::read(&jpg_path).unwrap();
        let entry = load_image_as_entry(std::path::Path::new(&jpg_path), false, true).unwrap();
        assert_eq!(entry.bytes, original, "已是 JPEG 不应二次编码");
    }
}

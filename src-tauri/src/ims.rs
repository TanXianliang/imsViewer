//! IMS 图片容器格式解析 — Rust 版
//!
//! 与 Python 版 ims_unpack.py 完全等价的 Rust 实现。
//! 使用 byteorder crate 处理小端字节序，对应 Python 的 struct.unpack("<...").

use byteorder::{LittleEndian, ReadBytesExt};
use serde::Serialize;
use std::io::{Cursor, Read};

/// IMS 文件魔数
pub const MAGIC: &[u8; 8] = b"IMSIMAGE";

// ---- 数据结构 ----

/// IMS 内单张图片的元数据
#[derive(Debug, Clone, Serialize)]
pub struct ImsImage {
    pub index: usize,
    pub filename: String,
    /// 0 = JPEG, 1 = PNG, 2 = WebP, 3 = AVIF
    pub format: u8,
    pub data_offset: u32,
    pub data_length: u32,
    pub is_cover: bool,
    /// SHA-256 内容哈希 (hex, v2+)
    pub hash: Option<String>,
}

/// IMS 文件整体元数据
#[derive(Debug, Clone, Serialize)]
pub struct ImsFileInfo {
    pub filepath: String,
    pub file_size: u64,
    pub version: u16,
    pub image_count: u32,
    pub data_start: u32,
    pub index_offset: u32,
    pub images: Vec<ImsImage>,
}

/// 文件夹中的一个可浏览图片条目
#[derive(Debug, Clone, Serialize)]
pub struct FolderEntry {
    pub filepath: String,
    pub filename: String,
    pub is_ims: bool,
    pub file_size: u64,
}

/// 打开文件后返回给前端的完整载荷
#[derive(Debug, Clone, Serialize)]
pub struct FilePayload {
    /// 文件夹中所有可浏览文件
    pub folder_list: Vec<FolderEntry>,
    /// 当前文件在 folder_list 中的索引
    pub file_index: usize,
    /// 文件名
    pub filename: String,
    /// 是否为 IMS 文件
    pub is_ims: bool,
    /// 文件大小 (bytes)
    pub file_size: u64,
    /// 文件类型标签 (JPEG / PNG)
    pub file_type: String,
    /// IMS 元数据 (非 IMS 文件时为 null，序列化为 None)
    pub ims_info: Option<ImsFileInfo>,
    /// 当前文件的所有子图元数据 + 封面 bytes (非 IMS 文件时仅一项)
    pub sub_images: Vec<SubImagePayload>,
}

/// 单张子图的元数据 + 数据
#[derive(Debug, Clone, Serialize)]
pub struct SubImagePayload {
    pub index: usize,
    pub filename: String,
    pub format: u8,
    pub data_offset: u32,
    pub data_length: u32,
    pub is_cover: bool,
    /// 图片原始字节 (base64 编码)
    pub bytes_base64: String,
    /// SHA-256 内容哈希 (hex, v2+)
    pub hash: Option<String>,
}

// ---- IMS 解析 ----

/// 解析 IMS 文件，返回元数据。不提取图片数据。
pub fn parse_ims(filepath: &str) -> Result<ImsFileInfo, String> {
    let data = std::fs::read(filepath).map_err(|e| format!("读取文件失败: {}", e))?;
    let file_size = data.len() as u64;

    if file_size < 4 {
        return Err("文件太小，不是有效的 IMS 文件".into());
    }

    // 末尾 4 字节 → index_offset (u32 LE)
    let index_offset = {
        let mut c = Cursor::new(&data[(file_size as usize) - 4..]);
        c.read_u32::<LittleEndian>()
            .map_err(|e| format!("读取 index_offset 失败: {}", e))?
    };

    if index_offset as u64 >= file_size - 4 {
        return Err("索引偏移异常，不是有效的 IMS 文件".into());
    }

    let mut c = Cursor::new(&data[index_offset as usize..]);

    // 验证 magic (8 bytes)
    let mut magic_buf = [0u8; 8];
    c.read_exact(&mut magic_buf)
        .map_err(|e| format!("读取 magic 失败: {}", e))?;
    if &magic_buf != MAGIC {
        return Err(format!(
            "Magic 不匹配: 期望 {}, 实际 {}",
            String::from_utf8_lossy(MAGIC),
            String::from_utf8_lossy(&magic_buf)
        ));
    }

    // header: version(2) + image_count(4) + data_start(4)
    let version = c.read_u16::<LittleEndian>().unwrap();
    let image_count = c.read_u32::<LittleEndian>().unwrap();
    let data_start = c.read_u32::<LittleEndian>().unwrap();

    if version != 1 && version != 2 {
        return Err(format!("不支持的 IMS 版本: {}", version));
    }

    // 遍历 entries
    let mut images = Vec::with_capacity(image_count as usize);
    for i in 0..image_count as usize {
        let name_len = c.read_u16::<LittleEndian>().unwrap() as usize;

        let mut name_buf = vec![0u8; name_len];
        c.read_exact(&mut name_buf)
            .map_err(|e| format!("读取图片 {} 名称失败: {}", i, e))?;
        let filename = String::from_utf8_lossy(&name_buf).to_string();

        let format = c.read_u8().unwrap();
        let data_offset = c.read_u32::<LittleEndian>().unwrap();
        let data_length = c.read_u32::<LittleEndian>().unwrap();
        let is_cover_byte = c.read_u8().unwrap();

        // v2+: 读取 hash
        let hash = if version >= 2 {
            let hash_algo = c.read_u8().unwrap();
            if hash_algo == 1 {
                // SHA-256: 32 bytes → hex
                let mut hash_buf = [0u8; 32];
                c.read_exact(&mut hash_buf)
                    .map_err(|e| format!("读取图片 {} hash 失败: {}", i, e))?;
                Some(hash_buf.iter().map(|b| format!("{:02x}", b)).collect())
            } else {
                None
            }
        } else {
            None
        };

        images.push(ImsImage {
            index: i,
            filename,
            format,
            data_offset,
            data_length,
            is_cover: is_cover_byte != 0,
            hash,
        });
    }

    Ok(ImsFileInfo {
        filepath: filepath.to_string(),
        file_size,
        version,
        image_count,
        data_start,
        index_offset,
        images,
    })
}

/// 从 IMS 文件中提取单张子图的原始字节
pub fn read_sub_image(filepath: &str, offset: u32, length: u32) -> Result<Vec<u8>, String> {
    let data = std::fs::read(filepath).map_err(|e| format!("读取文件失败: {}", e))?;
    let start = offset as usize;
    let end = (offset + length) as usize;
    if end > data.len() {
        return Err(format!("数据偏移超出文件范围: {} + {} > {}", offset, length, data.len()));
    }
    Ok(data[start..end].to_vec())
}

// ---- 文件夹扫描 ----

/// 支持的图片扩展名
const SUPPORTED_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "avif"];

/// 扫描目录，返回排序后的可浏览文件列表
pub fn scan_folder(dir_path: &str) -> Result<Vec<FolderEntry>, String> {
    let dir = std::fs::read_dir(dir_path)
        .map_err(|e| format!("读取目录失败: {}", e))?;

    let mut entries: Vec<FolderEntry> = dir
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();

            // 跳过目录
            if path.is_dir() {
                return None;
            }

            // 跳过隐藏文件
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with('.'))
                .unwrap_or(false)
            {
                return None;
            }

            let filename = path.file_name()?.to_str()?.to_string();
            let ext = path.extension()?.to_str()?.to_lowercase();
            let stem = path.file_stem()?.to_str()?.to_lowercase();

            let is_ims = stem.contains(".ims");
            let is_supported = SUPPORTED_EXTS.contains(&ext.as_str());

            // 支持: .jpg .jpeg .png .ims.jpg .ims.png
            if !is_supported {
                return None;
            }

            let file_size = std::fs::metadata(&path).ok()?.len();

            Some(FolderEntry {
                filepath: path.to_str()?.to_string(),
                filename,
                is_ims,
                file_size,
            })
        })
        .collect();

    // 按文件名自然排序
    entries.sort_by(|a, b| natord::compare(&a.filename, &b.filename));

    Ok(entries)
}

// 简易自然排序 (处理数字部分)
// 避免依赖 natord crate，使用简单的 alphanumeric 排序
mod natord {
    pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        let a_chars: Vec<char> = a_lower.chars().collect();
        let b_chars: Vec<char> = b_lower.chars().collect();

        let mut i = 0;
        let mut j = 0;

        while i < a_chars.len() && j < b_chars.len() {
            if a_chars[i].is_ascii_digit() && b_chars[j].is_ascii_digit() {
                // 提取数字部分
                let mut a_num = String::new();
                let mut b_num = String::new();
                while i < a_chars.len() && a_chars[i].is_ascii_digit() {
                    a_num.push(a_chars[i]);
                    i += 1;
                }
                while j < b_chars.len() && b_chars[j].is_ascii_digit() {
                    b_num.push(b_chars[j]);
                    j += 1;
                }
                let a_val: u64 = a_num.parse().unwrap_or(0);
                let b_val: u64 = b_num.parse().unwrap_or(0);
                match a_val.cmp(&b_val) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            } else {
                match a_chars[i].cmp(&b_chars[j]) {
                    std::cmp::Ordering::Equal => {
                        i += 1;
                        j += 1;
                    }
                    other => return other,
                }
            }
        }

        a_chars.len().cmp(&b_chars.len())
    }
}

// ---- 辅助函数 ----

/// 获取文件扩展名对应的 MIME/标签
pub fn file_type_label(filepath: &str) -> String {
    let lower = filepath.to_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "JPEG".into()
    } else if lower.ends_with(".png") {
        "PNG".into()
    } else if lower.ends_with(".webp") {
        "WebP".into()
    } else if lower.ends_with(".avif") {
        "AVIF".into()
    } else {
        "未知".into()
    }
}

/// 判断文件是否为 IMS 文件 (基于文件名模式)
pub fn is_ims_file(filepath: &str) -> bool {
    let path = std::path::Path::new(filepath);
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        stem.to_lowercase().contains(".ims")
    } else {
        false
    }
}

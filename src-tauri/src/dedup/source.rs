use super::types::DedupImageKind;
use crate::ims::{is_ims_file, parse_ims, read_sub_image};
use image::DynamicImage;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "avif"];

#[derive(Debug, Clone)]
pub struct SourceItem {
    pub path: PathBuf,
    pub kind: DedupImageKind,
    pub file_size: u64,
    pub modified_ns: i64,
}

pub struct DecodedSource {
    pub bytes: Vec<u8>,
    pub image: DynamicImage,
    pub format_label: String,
}

pub fn enumerate_sources(
    root: &Path,
    recursive: bool,
    max_depth: u32,
) -> Result<Vec<SourceItem>, String> {
    if !root.is_dir() {
        return Err(format!("图库目录不存在: {}", root.display()));
    }

    let mut walker = WalkDir::new(root).follow_links(false);
    walker = if recursive {
        if max_depth == 0 {
            walker
        } else {
            walker.max_depth(max_depth as usize + 1)
        }
    } else {
        walker.max_depth(1)
    };

    let mut items = Vec::new();
    for entry in walker.into_iter() {
        let entry = entry.map_err(|error| format!("扫描图库失败: {}", error))?;
        if !entry.file_type().is_file() || !is_supported_image(entry.path()) {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("读取文件信息失败 {}: {}", entry.path().display(), error))?;
        items.push(SourceItem {
            path: entry.path().to_path_buf(),
            kind: if is_ims_file(&entry.path().to_string_lossy()) {
                DedupImageKind::ImsCover
            } else {
                DedupImageKind::Standard
            },
            file_size: metadata.len(),
            modified_ns: modified_ns(metadata.modified().ok()),
        });
    }

    items.sort_by(|left, right| {
        left.path
            .to_string_lossy()
            .to_lowercase()
            .cmp(&right.path.to_string_lossy().to_lowercase())
    });
    Ok(items)
}

pub fn source_from_path(path: &Path) -> Result<SourceItem, String> {
    if !path.is_file() || !is_supported_image(path) {
        return Err(format!("不支持的图片文件: {}", path.display()));
    }
    let metadata = fs::metadata(path).map_err(|error| format!("读取文件信息失败: {}", error))?;
    Ok(SourceItem {
        path: path.to_path_buf(),
        kind: if is_ims_file(&path.to_string_lossy()) {
            DedupImageKind::ImsCover
        } else {
            DedupImageKind::Standard
        },
        file_size: metadata.len(),
        modified_ns: modified_ns(metadata.modified().ok()),
    })
}

pub fn decode_source(item: &SourceItem) -> Result<DecodedSource, String> {
    let bytes = match item.kind {
        DedupImageKind::Standard => fs::read(&item.path)
            .map_err(|error| format!("读取图片失败 {}: {}", item.path.display(), error))?,
        DedupImageKind::ImsCover => read_ims_cover(&item.path)?,
    };

    let detected_format = image::guess_format(&bytes).ok();
    let mut image = if detected_format == Some(image::ImageFormat::Avif) {
        decode_avif(&bytes)
            .map_err(|error| format!("解码 AVIF 失败 {}: {}", item.path.display(), error))?
    } else {
        image::load_from_memory(&bytes)
            .map_err(|error| format!("解码图片失败 {}: {}", item.path.display(), error))?
    };
    image = apply_exif_orientation(image, &bytes);
    let format_label = detected_format
        .map(format_label)
        .unwrap_or_else(|| "未知".to_string());

    Ok(DecodedSource {
        bytes,
        image,
        format_label,
    })
}

fn decode_avif(bytes: &[u8]) -> Result<DynamicImage, String> {
    avif::decode(bytes).map_err(|error| error.to_string())
}

pub fn is_supported_image(path: &Path) -> bool {
    if is_ims_file(&path.to_string_lossy()) {
        return true;
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| SUPPORTED_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn read_ims_cover(path: &Path) -> Result<Vec<u8>, String> {
    let path_string = path.to_string_lossy().to_string();
    let info = parse_ims(&path_string)?;
    let cover = info
        .images
        .iter()
        .find(|image| image.is_cover)
        .or_else(|| info.images.first())
        .ok_or_else(|| format!("IMS 没有封面: {}", path.display()))?;
    read_sub_image(&path_string, cover.data_offset, cover.data_length)
}

fn modified_ns(value: Option<SystemTime>) -> i64 {
    value
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn format_label(format: image::ImageFormat) -> String {
    match format {
        image::ImageFormat::Jpeg => "JPEG",
        image::ImageFormat::Png => "PNG",
        image::ImageFormat::WebP => "WebP",
        image::ImageFormat::Avif => "AVIF",
        _ => "未知",
    }
    .to_string()
}

fn apply_exif_orientation(image: DynamicImage, bytes: &[u8]) -> DynamicImage {
    let orientation = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
        .and_then(|exif| {
            exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|field| field.value.get_uint(0))
        })
        .unwrap_or(1);

    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};

    #[test]
    fn recognizes_supported_extensions() {
        assert!(is_supported_image(Path::new("a.jpg")));
        assert!(is_supported_image(Path::new("a.ims.png")));
        assert!(is_supported_image(Path::new("a.avif")));
        assert!(!is_supported_image(Path::new("a.gif")));
    }

    #[test]
    fn decodes_avif_pixels_and_alpha() {
        let source = RgbaImage::from_fn(48, 32, |x, y| {
            if x < 24 {
                Rgba([230, 30 + y as u8, 20, 255])
            } else {
                Rgba([20, 210, 40 + y as u8, 160])
            }
        });
        let mut bytes = Vec::new();
        image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut bytes, 10, 100)
            .write_image(source.as_raw(), 48, 32, ExtendedColorType::Rgba8)
            .unwrap();

        let decoded = decode_avif(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (48, 32));
        let left = decoded.get_pixel(8, 16);
        let right = decoded.get_pixel(40, 16);
        assert!(left[0] > left[1] && left[0] > left[2]);
        assert!(right[1] > right[0] && right[1] > right[2]);
        assert!(right[3] < 220);
    }
}

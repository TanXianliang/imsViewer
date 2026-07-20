use super::index::{open_index, read_record_by_id};
use super::source::{decode_source, source_from_path};
use super::types::ThumbnailPayload;
use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use std::path::Path;

pub fn load_result_thumbnail(
    app_data: &Path,
    root_path: &Path,
    item_id: i64,
) -> Result<ThumbnailPayload, String> {
    let (_paths, connection, _vectors) = open_index(app_data, root_path)?;
    let record = read_record_by_id(&connection, item_id)?;
    let source = source_from_path(Path::new(&record.absolute_path))?;
    let decoded = decode_source(&source)?;
    let thumbnail = decoded.image.thumbnail(160, 160).to_rgb8();
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 72)
        .encode_image(&thumbnail)
        .map_err(|error| format!("生成结果缩略图失败: {}", error))?;
    Ok(ThumbnailPayload {
        mime: "image/jpeg".to_string(),
        bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

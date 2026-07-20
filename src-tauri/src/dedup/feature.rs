use super::hash::{perceptual_hash, sha256_bytes};
use super::model::SscdModel;
use super::source::{decode_source, SourceItem};
use super::types::EMBEDDING_DIM;

pub struct ImageFeatures {
    pub sha256: [u8; 32],
    pub phash: u64,
    pub embedding: [f32; EMBEDDING_DIM],
    pub width: u32,
    pub height: u32,
    pub format_label: String,
}

pub fn extract_features(item: &SourceItem, model: &SscdModel) -> Result<ImageFeatures, String> {
    let decoded = decode_source(item)?;
    Ok(ImageFeatures {
        sha256: sha256_bytes(&decoded.bytes),
        phash: perceptual_hash(&decoded.image),
        embedding: model.infer(&decoded.image)?,
        width: decoded.image.width(),
        height: decoded.image.height(),
        format_label: decoded.format_label,
    })
}

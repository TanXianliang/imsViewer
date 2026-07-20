use super::feature::extract_features;
use super::hash::phash_distance;
use super::index::{normalize_path, open_index, read_records};
use super::model::SscdModel;
use super::score::{calibrated_confidence, is_template_risk};
use super::source::source_from_path;
use super::types::{DedupSearchRequest, DedupSearchResponse, DedupSearchResult, EMBEDDING_DIM};
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub fn search_index(
    app_data: &Path,
    request: &DedupSearchRequest,
    model: Arc<SscdModel>,
    cancel: &AtomicBool,
) -> Result<DedupSearchResponse, String> {
    let started = Instant::now();
    let source = source_from_path(Path::new(&request.source_path))?;
    let source_features = extract_features(&source, &model)?;
    let root = std::fs::canonicalize(&request.root_path)
        .map_err(|error| format!("图库路径无效 {}: {}", request.root_path, error))?;
    let (_paths, connection, vectors) = open_index(app_data, &root)?;
    let records = read_records(&connection)?;
    let source_key = normalize_path(&source.path);
    let threshold = request.threshold.clamp(-1.0, 1.0);

    let mut results = records
        .par_iter()
        .filter_map(|record| {
            if cancel.load(Ordering::Relaxed) || record.feature_status != 0 {
                return None;
            }
            if request.exclude_source
                && normalize_path(Path::new(&record.absolute_path)) == source_key
            {
                return None;
            }
            let similarity = dot_embedding(
                &source_features.embedding,
                &vectors,
                record.vector_offset,
                record.vector_length,
            )
            .ok()?;
            let exact = record.sha256.as_slice() == source_features.sha256;
            if !exact && similarity < threshold {
                return None;
            }
            let phash = phash_distance(source_features.phash, record.phash);
            let template_risk = is_template_risk(similarity, phash);
            Some(DedupSearchResult {
                item_id: record.id,
                filepath: record.absolute_path.clone(),
                filename: record.filename.clone(),
                kind: record.kind.as_str().to_string(),
                image_format: record.image_format.clone(),
                width: record.width,
                height: record.height,
                modified_ms: record.modified_ns / 1_000_000,
                raw_similarity: similarity,
                confidence: calibrated_confidence(similarity, exact),
                match_type: if exact {
                    "完全相同".to_string()
                } else if template_risk {
                    "待复核".to_string()
                } else {
                    "近似图片".to_string()
                },
                exact,
                phash_distance: phash,
                template_risk,
            })
        })
        .collect::<Vec<_>>();

    if cancel.load(Ordering::Relaxed) {
        return Err("图片查重已取消".to_string());
    }
    results.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| left.filepath.cmp(&right.filepath))
    });
    Ok(DedupSearchResponse {
        source_path: source.path.to_string_lossy().to_string(),
        root_path: root.to_string_lossy().to_string(),
        threshold,
        elapsed_ms: started.elapsed().as_millis() as u64,
        total_indexed: records.len() as u64,
        results,
    })
}

fn dot_embedding(
    query: &[f32; EMBEDDING_DIM],
    vectors: &[u8],
    offset: u64,
    length: u64,
) -> Result<f32, String> {
    if length as usize != EMBEDDING_DIM * 4 {
        return Err("索引向量长度异常".to_string());
    }
    let start = offset as usize;
    let end = start.saturating_add(length as usize);
    if end > vectors.len() {
        return Err("索引向量偏移越界".to_string());
    }
    let mut dot = 0.0f32;
    for (index, bytes) in vectors[start..end].chunks_exact(4).enumerate() {
        let value = f32::from_le_bytes(bytes.try_into().unwrap());
        dot += query[index] * value;
    }
    Ok(dot.clamp(-1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dot_product_from_binary_vector() {
        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;
        let mut bytes = Vec::with_capacity(EMBEDDING_DIM * 4);
        for value in query {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        assert!((dot_embedding(&query, &bytes, 0, bytes.len() as u64).unwrap() - 1.0).abs() < 1e-6);
    }
}

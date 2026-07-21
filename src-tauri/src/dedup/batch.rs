use super::batch_group::{build_groups, ConnectivityState};
use super::batch_store::BatchStore;
use super::hash::phash_distance;
use super::index::{embedding_values, get_index_status, normalize_path, open_index_snapshot};
use super::score::is_template_risk;
use super::types::{
    AcceptedEdge, BatchCheckpoint, BatchMemberSource, BatchProgressPayload, BatchRunSummary,
    BatchScanMode, BatchScanRequest,
};
use hnsw_rs::prelude::*;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_BATCH_ITEMS: usize = 100_000;
const QUICK_NEIGHBOURS: usize = 96;
const QUICK_EF_SEARCH: usize = 160;
const PHASH_BUCKET_LIMIT: usize = 256;
const COMPLETE_BLOCK_SIZE: usize = 256;

pub struct PairVerification {
    pub local_verified: bool,
    pub template_risk: bool,
    pub reason: String,
}

struct BatchItem {
    member: BatchMemberSource,
    sha256: Vec<u8>,
    phash: u64,
    embedding: Vec<f32>,
}

struct PreparedBatch {
    items: Vec<BatchItem>,
    revision: String,
}

struct ScanResult {
    processed_pairs: u64,
    candidate_pairs: u64,
    accepted_edges: Vec<AcceptedEdge>,
}

pub fn start_batch_scan<V, P>(
    app_data: &Path,
    run_id: &str,
    request: &BatchScanRequest,
    cancel: &AtomicBool,
    verify: V,
    progress: P,
) -> Result<BatchRunSummary, String>
where
    V: Fn(&str, &str, f32) -> Result<PairVerification, String> + Sync,
    P: Fn(BatchProgressPayload) + Sync,
{
    validate_request(request)?;
    progress(progress_payload(
        run_id,
        "preparing",
        0,
        0,
        0,
        0,
        "正在读取图库索引",
        "running",
    ));
    let prepared = prepare_batch(app_data, &request.root_paths)?;
    let store = BatchStore::create(app_data, run_id, request, &prepared.revision)?;
    execute_scan(
        &store, run_id, request, prepared, None, cancel, &verify, &progress,
    )
}

pub fn resume_batch_scan<V, P>(
    app_data: &Path,
    run_id: &str,
    cancel: &AtomicBool,
    verify: V,
    progress: P,
) -> Result<BatchRunSummary, String>
where
    V: Fn(&str, &str, f32) -> Result<PairVerification, String> + Sync,
    P: Fn(BatchProgressPayload) + Sync,
{
    let store = BatchStore::open(app_data, run_id)?;
    let request = store.request()?;
    if request.mode != BatchScanMode::Complete {
        return Err("只有完整扫描任务支持从检查点继续".to_string());
    }
    let checkpoint = store
        .load_checkpoint()?
        .ok_or_else(|| "批量任务没有可用检查点".to_string())?;
    let prepared = prepare_batch(app_data, &request.root_paths)?;
    if let Err(error) = store.validate_revision(&prepared.revision) {
        let _ = store.set_status("stale", Some(&error), false);
        return Err(error);
    }
    execute_scan(
        &store,
        run_id,
        &request,
        prepared,
        Some(checkpoint),
        cancel,
        &verify,
        &progress,
    )
}

pub fn current_revision(app_data: &Path, root_paths: &[String]) -> Result<String, String> {
    let mut revision_parts = Vec::new();
    for root_path in root_paths {
        let root = std::fs::canonicalize(root_path)
            .map_err(|error| format!("图库路径无效 {}: {}", root_path, error))?;
        let status = get_index_status(app_data, &root)?;
        let revision = status
            .revision
            .ok_or_else(|| "图库索引缺少修订号，请先更新索引".to_string())?;
        revision_parts.push(format!("{}:{}", normalize_path(&root), revision));
    }
    Ok(combined_revision(revision_parts))
}

fn execute_scan<V, P>(
    store: &BatchStore,
    run_id: &str,
    request: &BatchScanRequest,
    prepared: PreparedBatch,
    checkpoint: Option<BatchCheckpoint>,
    cancel: &AtomicBool,
    verify: &V,
    progress: &P,
) -> Result<BatchRunSummary, String>
where
    V: Fn(&str, &str, f32) -> Result<PairVerification, String> + Sync,
    P: Fn(BatchProgressPayload) + Sync,
{
    let total_pairs = match request.mode {
        BatchScanMode::Quick => prepared.items.len() as u64,
        BatchScanMode::Complete => unordered_pair_count(prepared.items.len())?,
    };
    store.set_total(total_pairs)?;
    let result = match request.mode {
        BatchScanMode::Quick => scan_quick(
            store,
            run_id,
            &prepared.items,
            request.threshold,
            cancel,
            verify,
            progress,
        ),
        BatchScanMode::Complete => scan_complete(
            store,
            run_id,
            &prepared.items,
            request.threshold,
            checkpoint,
            cancel,
            verify,
            progress,
        ),
    };

    let scan = match result {
        Ok(value) => value,
        Err(error) => {
            let cancelled = cancel.load(Ordering::Relaxed) || error.contains("已取消");
            let resumable = cancelled
                && request.mode == BatchScanMode::Complete
                && store.load_checkpoint()?.is_some();
            store.set_status(
                if cancelled { "cancelled" } else { "failed" },
                Some(&error),
                resumable,
            )?;
            progress(progress_payload(
                run_id,
                "finished",
                store.summary()?.processed_pairs,
                total_pairs,
                store.summary()?.candidate_pairs,
                store.summary()?.accepted_pairs,
                &error,
                if cancelled { "cancelled" } else { "failed" },
            ));
            return Err(error);
        }
    };

    store.update_progress(
        scan.processed_pairs,
        scan.candidate_pairs,
        scan.accepted_edges.len() as u64,
    )?;
    store.set_status("finalizing", None, false)?;
    progress(progress_payload(
        run_id,
        "grouping",
        scan.processed_pairs,
        total_pairs,
        scan.candidate_pairs,
        scan.accepted_edges.len() as u64,
        "正在整理重复图片组",
        "running",
    ));
    let sources = prepared
        .items
        .iter()
        .map(|item| item.member.clone())
        .collect::<Vec<_>>();
    let groups = build_groups(&sources, &scan.accepted_edges);
    store.replace_groups(&groups)?;
    store.remove_checkpoint()?;
    let summary = store.summary()?;
    progress(progress_payload(
        run_id,
        "finished",
        summary.processed_pairs,
        summary.total_pairs,
        summary.candidate_pairs,
        summary.accepted_pairs,
        &format!("批量查重完成，共 {} 组", summary.group_count),
        "completed",
    ));
    Ok(summary)
}

fn prepare_batch(app_data: &Path, root_paths: &[String]) -> Result<PreparedBatch, String> {
    if root_paths.is_empty() {
        return Err("请至少选择一个图库目录".to_string());
    }
    let mut revision_parts = Vec::new();
    let mut items = Vec::new();
    let mut seen_paths = HashSet::new();
    for root_path in root_paths {
        let root = std::fs::canonicalize(root_path)
            .map_err(|error| format!("图库路径无效 {}: {}", root_path, error))?;
        let snapshot = open_index_snapshot(app_data, &root)?;
        let normalized_root = normalize_path(&root);
        revision_parts.push(format!("{}:{}", normalized_root, snapshot.revision));
        for record in &snapshot.records {
            if record.feature_status != 0 {
                continue;
            }
            let normalized_path = normalize_path(Path::new(&record.absolute_path));
            if !seen_paths.insert(normalized_path) {
                continue;
            }
            let embedding = embedding_values(&snapshot.vectors, record)?;
            items.push(BatchItem {
                member: BatchMemberSource {
                    batch_item_id: 0,
                    item_id: record.id,
                    index_root_path: root.to_string_lossy().to_string(),
                    filepath: record.absolute_path.clone(),
                    filename: record.filename.clone(),
                    kind: record.kind.as_str().to_string(),
                    image_format: record.image_format.clone(),
                    width: record.width,
                    height: record.height,
                    file_size: record.file_size,
                    modified_ms: record.modified_ns / 1_000_000,
                },
                sha256: record.sha256.clone(),
                phash: record.phash,
                embedding,
            });
        }
    }
    if items.len() > MAX_BATCH_ITEMS {
        return Err(format!(
            "批量查重最多支持 {} 张，当前索引包含 {} 张有效图片",
            MAX_BATCH_ITEMS,
            items.len()
        ));
    }
    items.sort_by(|left, right| {
        normalize_path(Path::new(&left.member.filepath))
            .cmp(&normalize_path(Path::new(&right.member.filepath)))
    });
    for (index, item) in items.iter_mut().enumerate() {
        item.member.batch_item_id = index as i64 + 1;
    }
    Ok(PreparedBatch {
        items,
        revision: combined_revision(revision_parts),
    })
}

fn combined_revision(mut revision_parts: Vec<String>) -> String {
    revision_parts.sort();
    let mut hasher = Sha256::new();
    for part in revision_parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

fn scan_quick<V, P>(
    store: &BatchStore,
    run_id: &str,
    items: &[BatchItem],
    threshold: f32,
    cancel: &AtomicBool,
    verify: &V,
    progress: &P,
) -> Result<ScanResult, String>
where
    V: Fn(&str, &str, f32) -> Result<PairVerification, String> + Sync,
    P: Fn(BatchProgressPayload) + Sync,
{
    if items.len() < 2 {
        return Ok(ScanResult {
            processed_pairs: items.len() as u64,
            candidate_pairs: 0,
            accepted_edges: Vec::new(),
        });
    }
    progress(progress_payload(
        run_id,
        "ann_build",
        0,
        items.len() as u64,
        0,
        0,
        "正在建立快速候选索引",
        "running",
    ));
    let layer_count = 16.min((items.len() as f32).ln().ceil().max(1.0) as usize);
    let hnsw = Hnsw::<f32, DistCosine>::new(24, items.len(), layer_count, 200, DistCosine {});
    let indexed = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.embedding.as_slice(), index))
        .collect::<Vec<_>>();
    hnsw.parallel_insert_slice(&indexed);

    let item_ids = items
        .iter()
        .map(|item| item.member.batch_item_id)
        .collect::<Vec<_>>();
    let mut connectivity = ConnectivityState::new(item_ids);
    let mut accepted_edges = Vec::new();
    let mut candidate_pairs = 0u64;
    let mut exact_representatives = HashMap::<Vec<u8>, usize>::new();
    let mut phash_buckets = HashMap::<(usize, u16), Vec<usize>>::new();
    let threshold = threshold.clamp(-1.0, 1.0);

    for (index, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("批量快速扫描已取消".to_string());
        }
        let mut candidates = BTreeSet::new();
        if let Some(previous) = exact_representatives.get(&item.sha256) {
            candidates.insert(*previous);
        }
        for part in 0..4 {
            let key = ((item.phash >> (part * 16)) & 0xffff) as u16;
            if let Some(bucket) = phash_buckets.get(&(part, key)) {
                candidates.extend(bucket.iter().rev().take(PHASH_BUCKET_LIMIT).copied());
            }
        }
        for neighbour in hnsw.search(
            &item.embedding,
            QUICK_NEIGHBOURS.min(items.len()),
            QUICK_EF_SEARCH,
        ) {
            if neighbour.d_id < index {
                candidates.insert(neighbour.d_id);
            }
        }

        candidate_pairs = candidate_pairs.saturating_add(candidates.len() as u64);
        for previous in candidates {
            let left = &items[previous];
            let similarity = dot(&left.embedding, &item.embedding);
            let exact = left.sha256 == item.sha256;
            let phash = phash_distance(left.phash, item.phash);
            if let Some((reason, score)) =
                classify_pair(left, item, exact, similarity, phash, threshold, verify)?
            {
                if connectivity.connect(left.member.batch_item_id, item.member.batch_item_id)? {
                    accepted_edges.push(AcceptedEdge::new(
                        left.member.batch_item_id,
                        item.member.batch_item_id,
                        score,
                        &reason,
                    ));
                }
            }
        }

        exact_representatives
            .entry(item.sha256.clone())
            .or_insert(index);
        for part in 0..4 {
            let key = ((item.phash >> (part * 16)) & 0xffff) as u16;
            phash_buckets.entry((part, key)).or_default().push(index);
        }
        if index % 128 == 0 || index + 1 == items.len() {
            let processed = index as u64 + 1;
            store.update_progress(processed, candidate_pairs, accepted_edges.len() as u64)?;
            progress(progress_payload(
                run_id,
                "quick",
                processed,
                items.len() as u64,
                candidate_pairs,
                accepted_edges.len() as u64,
                "正在搜索近似候选",
                "running",
            ));
        }
    }
    Ok(ScanResult {
        processed_pairs: items.len() as u64,
        candidate_pairs,
        accepted_edges,
    })
}

#[allow(clippy::too_many_arguments)]
fn scan_complete<V, P>(
    store: &BatchStore,
    run_id: &str,
    items: &[BatchItem],
    threshold: f32,
    checkpoint: Option<BatchCheckpoint>,
    cancel: &AtomicBool,
    verify: &V,
    progress: &P,
) -> Result<ScanResult, String>
where
    V: Fn(&str, &str, f32) -> Result<PairVerification, String> + Sync,
    P: Fn(BatchProgressPayload) + Sync,
{
    let total_pairs = unordered_pair_count(items.len())?;
    let item_ids = items
        .iter()
        .map(|item| item.member.batch_item_id)
        .collect::<Vec<_>>();
    let mut checkpoint = checkpoint.unwrap_or_else(|| BatchCheckpoint {
        next_left_block: 0,
        processed_pairs: 0,
        candidate_pairs: 0,
        parent: (0..items.len()).collect(),
        rank: vec![0; items.len()],
        accepted_edges: Vec::new(),
    });
    let mut connectivity = ConnectivityState::from_parts(
        item_ids,
        checkpoint.parent.clone(),
        checkpoint.rank.clone(),
    )?;
    let block_count = items.len().div_ceil(COMPLETE_BLOCK_SIZE);
    let block_pairs = block_count
        .checked_mul(block_count.saturating_add(1))
        .and_then(|value| value.checked_div(2))
        .ok_or_else(|| "完整扫描分块数量溢出".to_string())?;
    let threshold = threshold.clamp(-1.0, 1.0);

    for ordinal in checkpoint.next_left_block..block_pairs {
        if cancel.load(Ordering::Relaxed) {
            checkpoint.next_left_block = ordinal;
            let (parent, rank) = connectivity.checkpoint_parts();
            checkpoint.parent = parent;
            checkpoint.rank = rank;
            store.save_checkpoint(&checkpoint)?;
            return Err("批量完整扫描已取消".to_string());
        }
        let (left_block, right_block) = block_pair_at(block_count, ordinal)
            .ok_or_else(|| "完整扫描分块检查点无效".to_string())?;
        let left_start = left_block * COMPLETE_BLOCK_SIZE;
        let left_end = (left_start + COMPLETE_BLOCK_SIZE).min(items.len());
        let right_start = right_block * COMPLETE_BLOCK_SIZE;
        let right_end = (right_start + COMPLETE_BLOCK_SIZE).min(items.len());
        let candidates = (left_start..left_end)
            .into_par_iter()
            .map(|left_index| {
                let first_right = if left_block == right_block {
                    (left_index + 1).max(right_start)
                } else {
                    right_start
                };
                let mut local = Vec::new();
                for right_index in first_right..right_end {
                    let left = &items[left_index];
                    let right = &items[right_index];
                    let exact = left.sha256 == right.sha256;
                    let similarity = dot(&left.embedding, &right.embedding);
                    if exact || similarity >= threshold {
                        local.push((left_index, right_index, similarity, exact));
                    }
                }
                local
            })
            .reduce(Vec::new, |mut left, mut right| {
                left.append(&mut right);
                left
            });

        checkpoint.processed_pairs =
            checkpoint
                .processed_pairs
                .saturating_add(pair_count_for_block(
                    left_start,
                    left_end,
                    right_start,
                    right_end,
                    left_block == right_block,
                )?);
        checkpoint.candidate_pairs = checkpoint
            .candidate_pairs
            .saturating_add(candidates.len() as u64);
        for (left_index, right_index, similarity, exact) in candidates {
            let left = &items[left_index];
            let right = &items[right_index];
            let phash = phash_distance(left.phash, right.phash);
            if let Some((reason, score)) =
                classify_pair(left, right, exact, similarity, phash, threshold, verify)?
            {
                if connectivity.connect(left.member.batch_item_id, right.member.batch_item_id)? {
                    checkpoint.accepted_edges.push(AcceptedEdge::new(
                        left.member.batch_item_id,
                        right.member.batch_item_id,
                        score,
                        &reason,
                    ));
                }
            }
        }
        checkpoint.next_left_block = ordinal + 1;
        let (parent, rank) = connectivity.checkpoint_parts();
        checkpoint.parent = parent;
        checkpoint.rank = rank;
        store.save_checkpoint(&checkpoint)?;
        store.update_progress(
            checkpoint.processed_pairs,
            checkpoint.candidate_pairs,
            checkpoint.accepted_edges.len() as u64,
        )?;
        progress(progress_payload(
            run_id,
            "complete",
            checkpoint.processed_pairs,
            total_pairs,
            checkpoint.candidate_pairs,
            checkpoint.accepted_edges.len() as u64,
            "正在完整比较图片对",
            "running",
        ));
    }
    Ok(ScanResult {
        processed_pairs: checkpoint.processed_pairs,
        candidate_pairs: checkpoint.candidate_pairs,
        accepted_edges: checkpoint.accepted_edges,
    })
}

fn classify_pair<V>(
    left: &BatchItem,
    right: &BatchItem,
    exact: bool,
    similarity: f32,
    phash: u32,
    threshold: f32,
    verify: &V,
) -> Result<Option<(String, f32)>, String>
where
    V: Fn(&str, &str, f32) -> Result<PairVerification, String> + Sync,
{
    if exact {
        return Ok(Some(("sha256".to_string(), 1.0)));
    }
    if similarity < threshold {
        return Ok(None);
    }
    if !is_template_risk(similarity, phash) {
        return Ok(Some(("sscd".to_string(), similarity)));
    }
    let verification = verify(&left.member.filepath, &right.member.filepath, similarity)?;
    if verification.local_verified && !verification.template_risk {
        Ok(Some((verification.reason, similarity)))
    } else {
        Ok(None)
    }
}

fn validate_request(request: &BatchScanRequest) -> Result<(), String> {
    if request.root_paths.is_empty() {
        return Err("请至少选择一个图库目录".to_string());
    }
    if !request.threshold.is_finite() || !(-1.0..=1.0).contains(&request.threshold) {
        return Err("查重阈值必须位于 -1 到 1 之间".to_string());
    }
    Ok(())
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

fn unordered_pair_count(count: usize) -> Result<u64, String> {
    let count = u64::try_from(count).map_err(|_| "图片数量无法转换".to_string())?;
    count
        .checked_mul(count.saturating_sub(1))
        .and_then(|value| value.checked_div(2))
        .ok_or_else(|| "图片对数量溢出".to_string())
}

fn pair_count_for_block(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
    diagonal: bool,
) -> Result<u64, String> {
    let left_len = u64::try_from(left_end.saturating_sub(left_start))
        .map_err(|_| "分块长度无法转换".to_string())?;
    if diagonal {
        left_len
            .checked_mul(left_len.saturating_sub(1))
            .and_then(|value| value.checked_div(2))
            .ok_or_else(|| "分块图片对数量溢出".to_string())
    } else {
        let right_len = u64::try_from(right_end.saturating_sub(right_start))
            .map_err(|_| "分块长度无法转换".to_string())?;
        left_len
            .checked_mul(right_len)
            .ok_or_else(|| "分块图片对数量溢出".to_string())
    }
}

fn block_pair_at(block_count: usize, ordinal: usize) -> Option<(usize, usize)> {
    let mut current = 0usize;
    for left in 0..block_count {
        let row_len = block_count - left;
        if ordinal < current + row_len {
            return Some((left, left + ordinal - current));
        }
        current += row_len;
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn progress_payload(
    run_id: &str,
    phase: &str,
    processed_pairs: u64,
    total_pairs: u64,
    candidate_pairs: u64,
    accepted_pairs: u64,
    message: &str,
    status: &str,
) -> BatchProgressPayload {
    BatchProgressPayload {
        run_id: run_id.to_string(),
        phase: phase.to_string(),
        processed_pairs,
        total_pairs,
        candidate_pairs,
        accepted_pairs,
        message: message.to_string(),
        status: status.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, axis: usize, sha: u8) -> BatchItem {
        let mut embedding = vec![0.0; super::super::types::EMBEDDING_DIM];
        embedding[axis] = 1.0;
        BatchItem {
            member: BatchMemberSource {
                batch_item_id: id,
                item_id: id,
                index_root_path: "C:/gallery".to_string(),
                filepath: format!("C:/gallery/{id}.png"),
                filename: format!("{id}.png"),
                kind: "standard".to_string(),
                image_format: "PNG".to_string(),
                width: 64,
                height: 64,
                file_size: 100,
                modified_ms: 1,
            },
            sha256: vec![sha; 32],
            phash: axis as u64,
            embedding,
        }
    }

    #[test]
    fn block_pairs_cover_triangle_once() {
        let pairs = (0..6)
            .map(|ordinal| block_pair_at(3, ordinal).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pairs, vec![(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)]);
        assert_eq!(unordered_pair_count(4).unwrap(), 6);
    }

    #[test]
    fn strict_classifier_accepts_hash_and_rejects_below_threshold() {
        let left = item(1, 0, 1);
        let exact = item(2, 1, 1);
        let different = item(3, 1, 2);
        let verify = |_: &str, _: &str, _: f32| -> Result<PairVerification, String> {
            panic!("verifier should not run")
        };
        assert_eq!(
            classify_pair(&left, &exact, true, 0.0, 64, 0.85, &verify)
                .unwrap()
                .unwrap()
                .0,
            "sha256"
        );
        assert!(
            classify_pair(&left, &different, false, 0.0, 1, 0.85, &verify)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn quick_and_complete_scans_connect_near_vectors() {
        let app_data = tempfile::tempdir().unwrap();
        let mut negative = item(4, 0, 4);
        negative.embedding[0] = -1.0;
        let items = vec![item(1, 0, 1), item(2, 0, 2), item(3, 1, 3), negative];
        let verify = |_: &str, _: &str, _: f32| {
            Ok(PairVerification {
                local_verified: true,
                template_risk: false,
                reason: "test".to_string(),
            })
        };
        let progress = |_: BatchProgressPayload| {};

        let quick_request = BatchScanRequest {
            root_paths: vec!["C:/gallery".to_string()],
            mode: BatchScanMode::Quick,
            threshold: 0.85,
        };
        let quick_store =
            BatchStore::create(app_data.path(), "quick-test", &quick_request, "rev").unwrap();
        quick_store.set_total(items.len() as u64).unwrap();
        let quick = scan_quick(
            &quick_store,
            "quick-test",
            &items,
            0.85,
            &AtomicBool::new(false),
            &verify,
            &progress,
        )
        .unwrap();
        assert_eq!(quick.accepted_edges.len(), 1);

        let complete_request = BatchScanRequest {
            mode: BatchScanMode::Complete,
            ..quick_request
        };
        let complete_store =
            BatchStore::create(app_data.path(), "complete-test", &complete_request, "rev").unwrap();
        complete_store.set_total(6).unwrap();
        let complete = scan_complete(
            &complete_store,
            "complete-test",
            &items,
            0.85,
            None,
            &AtomicBool::new(false),
            &verify,
            &progress,
        )
        .unwrap();
        assert_eq!(complete.processed_pairs, 6);
        assert_eq!(complete.accepted_edges.len(), 1);
    }
}

# 文件夹批量图片查重 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在现有图片查重子工具中增加 10 万张以内图库的文件夹批量查重，以快速扫描或完整扫描生成严格近重复图片关联组。

**Architecture:** 复用现有 SQLite 元数据、SHA-256、pHash、SSCD 向量和 ORB/OCR 复核。快速模式通过 `hnsw_rs` 与 pHash 分桶生成候选，完整模式分块遍历全部无序图片对；两种模式都用同一严格判定器和并查集分组，并把任务、结果与可恢复检查点持久化到应用数据目录。

**Tech Stack:** Rust 2021、Tauri 2、rusqlite、memmap2、Rayon、wide 0.7.33、hnsw_rs 0.3.4、serde/serde_json、原生 JavaScript、HTML、CSS。

---

仓库约束：本计划不包含 `git commit` 步骤。只有用户明确要求时才提交；每个任务以定向测试、`git diff --check` 和可审阅 diff 作为检查点。

## 文件结构

新增文件：

- `src-tauri/src/dedup/batch_group.rs`：并查集、稳定分组和推荐保留项。
- `src-tauri/src/dedup/batch_store.rs`：批量任务数据库、结果分页、检查点和过期判定。
- `src-tauri/src/dedup/ann.rs`：HNSW 派生索引、pHash 候选和快速扫描候选生成。
- `src-tauri/src/dedup/batch.rs`：严格图片对判定、快速/完整扫描编排和进度回调。
- `src-tauri/src/dedup/test_support.rs`：仅测试使用的临时向量索引和固定向量夹具。
- `src/dedup-batch.js`：批量模式前端状态、任务恢复、重复组和成员渲染。

修改文件：

- `src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`：加入 `wide = "0.7.33"` 和 `hnsw_rs = "0.3.4"`。
- `src-tauri/src/dedup/types.rs`：批量请求、任务、分组、分页和进度类型。
- `src-tauri/src/dedup/index.rs`：索引修订号、只读快照和安全向量读取。
- `src-tauri/src/dedup/mod.rs`：注册新后端模块。
- `src-tauri/src/dedup/service.rs`：读写锁、批量 Tauri Commands 和事件。
- `src-tauri/src/dedup/tests.rs`：真实图片、IMS 封面、取消恢复和过期结果集成测试。
- `src-tauri/src/lib.rs`：注册批量 Commands。
- `src/dedup.js`：模式切换、公共任务控制和批量控制器接入。
- `index.html`：模式选择、扫描方式、批量任务及重复组容器。
- `src/styles.css`：批量模式、组卡片、成员网格和响应式样式。
- `IMAGE_DEDUP_DESIGN.md`：实施完成后同步状态和实际参数。
- `README.md`：补充文件夹批量查重使用说明。

## Task 1：索引修订号、快照与并发读写边界

**Files:**

- Modify: `src-tauri/src/dedup/types.rs`
- Modify: `src-tauri/src/dedup/index.rs`
- Modify: `src-tauri/src/dedup/service.rs`
- Modify: `src-tauri/src/dedup/tests.rs`

- [ ] **Step 1：先给现有索引往返测试增加修订号断言**

在 `index_update_search_and_cleanup_round_trip` 的首次更新后加入：

```rust
let first_status = get_index_status(&app_data, &gallery).unwrap();
let first_revision = first_status.revision.clone().expect("index revision");
assert!(!first_revision.is_empty());
```

在取消更新后加入：

```rust
assert_eq!(
    get_index_status(&app_data, &gallery).unwrap().revision,
    Some(first_revision.clone()),
);
```

在成功增量更新后加入：

```rust
let second_revision = get_index_status(&app_data, &gallery)
    .unwrap()
    .revision
    .expect("updated revision");
assert_ne!(second_revision, first_revision);

let unchanged = update_index(&app_data, &config, model.clone(), &cancel, |_, _, _, _| {}).unwrap();
assert_eq!(unchanged.added + unchanged.updated + unchanged.removed + unchanged.failed, 0);
assert_eq!(
    get_index_status(&app_data, &gallery).unwrap().revision,
    Some(second_revision),
);
```

为了继续执行无变化更新断言，把现有第二次 `update_index` 调用的模型参数从 `model` 改为 `model.clone()`。

- [ ] **Step 2：运行测试并确认因字段缺失而失败**

Run:

```powershell
cd src-tauri
cargo test dedup::tests::index_update_search_and_cleanup_round_trip -- --nocapture
```

Expected: 编译失败，指出 `DedupIndexStatus` 没有 `revision` 字段。

- [ ] **Step 3：加入修订号和索引快照类型**

在 `types.rs` 的 `DedupIndexStatus` 与 `DedupIndexSummary` 中增加：

```rust
pub revision: Option<String>,
```

在 `index.rs` 增加：

```rust
pub struct IndexSnapshot {
    pub paths: IndexPaths,
    pub revision: String,
    pub records: Vec<IndexRecord>,
    pub vectors: Mmap,
}

pub fn open_index_snapshot(app_data: &Path, root: &Path) -> Result<IndexSnapshot, String> {
    let (paths, connection, vectors) = open_index(app_data, root)?;
    let revision = read_meta(&connection, "revision")
        .ok_or_else(|| "图库索引缺少修订号，请更新索引".to_string())?;
    let records = read_records(&connection)?;
    Ok(IndexSnapshot {
        paths,
        revision,
        records,
        vectors,
    })
}

pub fn embedding_slice<'a>(vectors: &'a [u8], record: &IndexRecord) -> Result<&'a [u8], String> {
    if record.vector_length as usize != VECTOR_BYTES {
        return Err(format!("索引向量长度异常: {}", record.absolute_path));
    }
    let start = usize::try_from(record.vector_offset)
        .map_err(|_| "索引向量偏移无法转换".to_string())?;
    let end = start
        .checked_add(record.vector_length as usize)
        .ok_or_else(|| "索引向量偏移溢出".to_string())?;
    vectors
        .get(start..end)
        .ok_or_else(|| format!("索引向量偏移越界: {}", record.absolute_path))
}

pub fn embedding_values(vectors: &[u8], record: &IndexRecord) -> Result<Vec<f32>, String> {
    Ok(embedding_slice(vectors, record)?
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect())
}
```

修订号表示索引内容版本。完整重建，或新增、更新、删除、失败状态、模型版本和预处理版本发生变化时生成新 UUID；无变化增量扫描沿用旧 revision。失败或取消不能改写旧修订号：

```rust
let content_changed = force_rebuild
    || added > 0
    || updated > 0
    || removed > 0
    || failed > 0
    || previous_model_version != Some(MODEL_VERSION)
    || previous_preprocess_version != Some(PREPROCESS_VERSION);
let revision = if content_changed {
    uuid::Uuid::new_v4().to_string()
} else {
    previous_revision.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
};
write_meta(&transaction, "revision", &revision)?;
```

扩展 `load_existing` 返回 `previous_revision: Option<String>`，无变化更新不得让现有 HNSW 和历史批量结果无故过期。

`get_index_status` 和 `list_indexes` 从 `index_meta.revision` 填充新字段。

- [ ] **Step 4：把服务层索引互斥锁改为读写锁**

将 `service.rs` 的字段和初始化改为：

```rust
use std::sync::{Arc, Mutex, RwLock};

struct DedupServiceInner {
    model: Mutex<Option<Arc<SscdModel>>>,
    verifier: Mutex<Option<Arc<OcrVerifier>>>,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    index_lock: RwLock<()>,
    thumbnail_lock: Mutex<()>,
    verify_lock: Mutex<()>,
}
```

索引更新、重建和删除取得 `write()`；单图搜索、缩略图和后续批量扫描取得 `read()`。锁失败统一映射为现有中文错误格式。

- [ ] **Step 5：运行定向和全量 Rust 测试**

Run:

```powershell
cd src-tauri
cargo test dedup::tests::index_update_search_and_cleanup_round_trip -- --nocapture
cargo test
```

Expected: 定向测试通过；现有全部测试通过。

- [ ] **Step 6：检查差异**

Run:

```powershell
git diff --check -- src-tauri/src/dedup/types.rs src-tauri/src/dedup/index.rs src-tauri/src/dedup/service.rs src-tauri/src/dedup/tests.rs
```

Expected: 无输出。

## Task 2：稳定关联分组和推荐保留项

**Files:**

- Create: `src-tauri/src/dedup/batch_group.rs`
- Modify: `src-tauri/src/dedup/mod.rs`
- Modify: `src-tauri/src/dedup/types.rs`

- [ ] **Step 1：先写连通分组和稳定推荐测试**

在 `batch_group.rs` 创建测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: i64, pixels: u64, file_size: u64, path: &str) -> BatchMemberSource {
        BatchMemberSource {
            item_id: id,
            filepath: path.to_string(),
            filename: path.to_string(),
            kind: "standard".to_string(),
            image_format: "PNG".to_string(),
            width: pixels as u32,
            height: 1,
            file_size,
            modified_ms: 1,
        }
    }

    #[test]
    fn transitive_edges_form_one_group() {
        let members = vec![
            member(1, 100, 10, "a.png"),
            member(2, 200, 20, "b.png"),
            member(3, 150, 30, "c.png"),
            member(4, 50, 5, "unrelated.png"),
        ];
        let edges = vec![
            AcceptedEdge::new(1, 2, 0.92, "sscd"),
            AcceptedEdge::new(2, 3, 0.89, "orb"),
        ];

        let groups = build_groups(&members, &edges);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_id, 1);
        assert_eq!(groups[0].recommended_item_id, 2);
        assert_eq!(groups[0].members.iter().map(|m| m.item_id).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn representative_ties_break_by_normalized_path() {
        let members = vec![member(7, 100, 20, "z.png"), member(9, 100, 20, "a.png")];
        let groups = build_groups(&members, &[AcceptedEdge::new(7, 9, 1.0, "sha256")]);
        assert_eq!(groups[0].recommended_item_id, 9);
    }
}
```

- [ ] **Step 2：运行测试并确认模块尚未注册**

Run:

```powershell
cd src-tauri
cargo test dedup::batch_group -- --nocapture
```

Expected: 编译失败，指出 `batch_group` 或相关类型不存在。

- [ ] **Step 3：定义批量成员、边和分组输出类型**

在 `types.rs` 增加可序列化输出：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchMemberSource {
    pub item_id: i64,
    pub filepath: String,
    pub filename: String,
    pub kind: String,
    pub image_format: String,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub modified_ms: i64,
}

impl BatchMemberSource {
    pub fn from_record(record: &IndexRecord) -> Self {
        Self {
            item_id: record.id,
            filepath: record.absolute_path.clone(),
            filename: record.filename.clone(),
            kind: record.kind.as_str().to_string(),
            image_format: record.image_format.clone(),
            width: record.width,
            height: record.height,
            file_size: record.file_size,
            modified_ms: record.modified_ns / 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptedEdge {
    pub left_item_id: i64,
    pub right_item_id: i64,
    pub similarity: f32,
    pub reason: String,
}

impl AcceptedEdge {
    pub fn new(left_item_id: i64, right_item_id: i64, similarity: f32, reason: &str) -> Self {
        Self {
            left_item_id,
            right_item_id,
            similarity,
            reason: reason.to_string(),
        }
    }
}
```

同时增加以下完整输出类型：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchGroupMember {
    pub item_id: i64,
    pub filepath: String,
    pub filename: String,
    pub kind: String,
    pub image_format: String,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub modified_ms: i64,
    pub linked_item_id: Option<i64>,
    pub similarity: Option<f32>,
    pub reason: Option<String>,
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchGroup {
    pub group_id: i64,
    pub member_count: u64,
    pub total_bytes: u64,
    pub reclaimable_bytes: u64,
    pub recommended_item_id: i64,
    pub min_similarity: f32,
    pub max_similarity: f32,
    pub members: Vec<BatchGroupMember>,
    pub edges: Vec<AcceptedEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchGroupSummary {
    pub group_id: i64,
    pub member_count: u64,
    pub total_bytes: u64,
    pub reclaimable_bytes: u64,
    pub recommended_item_id: i64,
    pub min_similarity: f32,
    pub max_similarity: f32,
}
```

- [ ] **Step 4：实现并查集和稳定分组**

核心实现使用 item ID 到连续下标的映射：

```rust
struct DisjointSet {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl DisjointSet {
    fn new(len: usize) -> Self {
        Self { parent: (0..len).collect(), rank: vec![0; len] }
    }

    fn find(&mut self, index: usize) -> usize {
        if self.parent[index] != index {
            self.parent[index] = self.find(self.parent[index]);
        }
        self.parent[index]
    }

    fn union(&mut self, left: usize, right: usize) -> bool {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root == right_root { return false; }
        match self.rank[left_root].cmp(&self.rank[right_root]) {
            std::cmp::Ordering::Less => self.parent[left_root] = right_root,
            std::cmp::Ordering::Greater => self.parent[right_root] = left_root,
            std::cmp::Ordering::Equal => {
                self.parent[right_root] = left_root;
                self.rank[left_root] += 1;
            }
        }
        true
    }
}
```

在并查集外提供可检查点化的 ID 层：

```rust
pub struct ConnectivityState {
    item_ids: Vec<i64>,
    index_by_id: std::collections::HashMap<i64, usize>,
    sets: DisjointSet,
}

impl ConnectivityState {
    pub fn new(item_ids: Vec<i64>) -> Self {
        let index_by_id = item_ids.iter().enumerate().map(|(index, item_id)| (*item_id, index)).collect();
        let sets = DisjointSet::new(item_ids.len());
        Self { item_ids, index_by_id, sets }
    }

    pub fn from_parts(item_ids: Vec<i64>, parent: Vec<usize>, rank: Vec<u8>) -> Result<Self, String> {
        if parent.len() != item_ids.len() || rank.len() != item_ids.len() {
            return Err("批量检查点并查集长度不匹配".to_string());
        }
        if parent.iter().any(|value| *value >= item_ids.len()) {
            return Err("批量检查点并查集索引越界".to_string());
        }
        let index_by_id = item_ids.iter().enumerate().map(|(index, item_id)| (*item_id, index))
            .collect::<std::collections::HashMap<_, _>>();
        if index_by_id.len() != item_ids.len() {
            return Err("批量检查点包含重复 item ID".to_string());
        }
        Ok(Self { item_ids, index_by_id, sets: DisjointSet { parent, rank } })
    }

    pub fn connect(&mut self, left_item_id: i64, right_item_id: i64) -> Result<bool, String> {
        let left = *self.index_by_id.get(&left_item_id).ok_or_else(|| format!("未知 item ID: {left_item_id}"))?;
        let right = *self.index_by_id.get(&right_item_id).ok_or_else(|| format!("未知 item ID: {right_item_id}"))?;
        Ok(self.sets.union(left, right))
    }

    pub fn checkpoint_parts(&self) -> (Vec<usize>, Vec<u8>) {
        (self.sets.parent.clone(), self.sets.rank.clone())
    }
}
```

`connect` 只在两个连通分量首次合并时返回 true。扫描器仅在 true 时保存该关联边，因此最多保存 `有效图片数 - 1` 条边。`build_groups` 忽略孤立成员，只输出至少两张图片的连通分量；组 ID 取组内最小 item ID。推荐项按像素面积、文件大小降序，规范化路径升序确定。成员按推荐项优先、路径升序输出。

- [ ] **Step 5：运行分组测试**

Run:

```powershell
cd src-tauri
cargo test dedup::batch_group -- --nocapture
```

Expected: 两个测试通过。

## Task 3：任务数据库、分页和原子检查点

**Files:**

- Create: `src-tauri/src/dedup/batch_store.rs`
- Modify: `src-tauri/src/dedup/mod.rs`
- Modify: `src-tauri/src/dedup/types.rs`

- [ ] **Step 1：先写任务往返、过期和检查点测试**

```rust
#[test]
fn run_store_round_trip_and_revision_validation() {
    let app_data = tempfile::tempdir().unwrap();
    let store = BatchStore::create(
        app_data.path(),
        "run-1",
        &BatchScanRequest::complete("C:/gallery", 0.85),
        "index-1",
        "revision-a",
    ).unwrap();

    let checkpoint = BatchCheckpoint::new(2, 3, vec![0, 0, 2], vec![1, 0, 0], vec![]);
    store.save_checkpoint(&checkpoint).unwrap();
    assert_eq!(store.load_checkpoint().unwrap(), Some(checkpoint));
    assert!(store.validate_revision("revision-a").is_ok());
    assert!(store.validate_revision("revision-b").is_err());
}

#[test]
fn persisted_groups_are_paginated_without_duplicate_members() {
    let app_data = tempfile::tempdir().unwrap();
    let store = BatchStore::create(
        app_data.path(),
        "run-2",
        &BatchScanRequest::complete("C:/gallery", 0.85),
        "index-1",
        "revision-a",
    ).unwrap();
    let groups = vec![stored_group(1, &[1, 2]), stored_group(3, &[3, 4, 5])];
    store.replace_groups(&groups).unwrap();
    let page = store.get_groups(0, 1).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items.len(), 1);
    let members = store.get_group_members(page.items[0].group_id, 0, 50).unwrap();
    let ids = members.items.iter().map(|item| item.item_id).collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), members.items.len());
}

#[test]
fn run_id_cannot_escape_batch_directory() {
    let app_data = tempfile::tempdir().unwrap();
    let request = BatchScanRequest::complete("C:/gallery", 0.85);
    assert!(BatchStore::create(app_data.path(), "../escape", &request, "index-1", "revision-a").is_err());
    assert!(BatchStore::open(app_data.path(), "C:\\escape").is_err());
}

fn stored_group(group_id: i64, item_ids: &[i64]) -> BatchGroup {
    let members = item_ids.iter().map(|item_id| BatchGroupMember {
        item_id: *item_id,
        filepath: format!("C:/gallery/{item_id}.png"),
        filename: format!("{item_id}.png"),
        kind: "standard".to_string(),
        image_format: "PNG".to_string(),
        width: 64,
        height: 64,
        file_size: 100,
        modified_ms: 1,
        linked_item_id: (*item_id != group_id).then_some(group_id),
        similarity: (*item_id != group_id).then_some(0.95),
        reason: (*item_id != group_id).then(|| "sscd".to_string()),
        recommended: *item_id == group_id,
    }).collect::<Vec<_>>();
    let edges = item_ids.iter().skip(1).map(|item_id| AcceptedEdge::new(group_id, *item_id, 0.95, "sscd")).collect();
    BatchGroup {
        group_id,
        member_count: item_ids.len() as u64,
        total_bytes: item_ids.len() as u64 * 100,
        reclaimable_bytes: item_ids.len().saturating_sub(1) as u64 * 100,
        recommended_item_id: group_id,
        min_similarity: 0.95,
        max_similarity: 0.95,
        members,
        edges,
    }
}
```

- [ ] **Step 2：运行测试并确认存储类型缺失**

Run:

```powershell
cd src-tauri
cargo test dedup::batch_store -- --nocapture
```

Expected: 编译失败，指出 `BatchStore`、`BatchCheckpoint` 等尚未定义。

- [ ] **Step 3：定义扫描请求、状态、分页和检查点类型**

在 `types.rs` 增加：

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchScanMode { Quick, Complete }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchScanRequest {
    pub root_path: String,
    pub mode: BatchScanMode,
    pub threshold: f32,
}

impl BatchScanRequest {
    pub fn complete(root_path: &str, threshold: f32) -> Self {
        Self { root_path: root_path.to_string(), mode: BatchScanMode::Complete, threshold }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchCheckpoint {
    pub next_left_block: usize,
    pub next_right_block: usize,
    pub processed_pairs: u64,
    pub candidate_pairs: u64,
    pub parent: Vec<usize>,
    pub rank: Vec<u8>,
    pub accepted_edges: Vec<AcceptedEdge>,
}

impl BatchCheckpoint {
    pub fn new(
        next_left_block: usize,
        next_right_block: usize,
        parent: Vec<usize>,
        rank: Vec<u8>,
        accepted_edges: Vec<AcceptedEdge>,
    ) -> Self {
        Self {
            next_left_block,
            next_right_block,
            processed_pairs: 0,
            candidate_pairs: 0,
            parent,
            rank,
            accepted_edges,
        }
    }

    pub fn advance(&mut self, left_block: usize, right_block: usize, block_count: usize) {
        if right_block + 1 < block_count {
            self.next_left_block = left_block;
            self.next_right_block = right_block + 1;
        } else {
            self.next_left_block = left_block + 1;
            self.next_right_block = left_block + 1;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRunSummary {
    pub run_id: String,
    pub root_path: String,
    pub mode: BatchScanMode,
    pub status: String,
    pub index_revision: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub processed_pairs: u64,
    pub total_pairs: u64,
    pub candidate_pairs: u64,
    pub accepted_pairs: u64,
    pub group_count: u64,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRunDetail {
    pub summary: BatchRunSummary,
    pub index_id: String,
    pub threshold: f32,
    pub model_version: u32,
    pub preprocess_version: u32,
    pub profile_version: u32,
    pub resumable: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchGroupPage {
    pub page: u64,
    pub page_size: u64,
    pub total: u64,
    pub items: Vec<BatchGroupSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMemberPage {
    pub group_id: i64,
    pub page: u64,
    pub page_size: u64,
    pub total: u64,
    pub items: Vec<BatchGroupMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteBatchRunResult {
    pub deleted_runs: u64,
    pub released_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchProgressPayload {
    pub run_id: String,
    pub phase: String,
    pub processed_pairs: u64,
    pub total_pairs: u64,
    pub candidate_pairs: u64,
    pub accepted_pairs: u64,
    pub message: String,
    pub status: String,
}

impl BatchProgressPayload {
    pub fn running(
        run_id: &str,
        phase: &str,
        processed_pairs: u64,
        total_pairs: u64,
        candidate_pairs: u64,
        accepted_pairs: u64,
    ) -> Self {
        Self {
            run_id: run_id.to_string(),
            phase: phase.to_string(),
            processed_pairs,
            total_pairs,
            candidate_pairs,
            accepted_pairs,
            message: String::new(),
            status: "running".to_string(),
        }
    }
}
```

- [ ] **Step 4：实现 SQLite schema 和原子检查点**

`run.sqlite3` 初始化 SQL：

```sql
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;
CREATE TABLE run_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE groups (
  group_id INTEGER PRIMARY KEY,
  member_count INTEGER NOT NULL,
  total_bytes INTEGER NOT NULL,
  reclaimable_bytes INTEGER NOT NULL,
  recommended_item_id INTEGER NOT NULL,
  min_similarity REAL NOT NULL,
  max_similarity REAL NOT NULL
);
CREATE TABLE members (
  group_id INTEGER NOT NULL,
  item_id INTEGER NOT NULL,
  filepath TEXT NOT NULL,
  filename TEXT NOT NULL,
  kind TEXT NOT NULL,
  image_format TEXT NOT NULL,
  width INTEGER NOT NULL,
  height INTEGER NOT NULL,
  file_size INTEGER NOT NULL,
  modified_ms INTEGER NOT NULL,
  linked_item_id INTEGER,
  similarity REAL,
  reason TEXT,
  recommended INTEGER NOT NULL,
  PRIMARY KEY (group_id, item_id)
);
CREATE TABLE edges (
  left_item_id INTEGER NOT NULL,
  right_item_id INTEGER NOT NULL,
  similarity REAL NOT NULL,
  reason TEXT NOT NULL,
  PRIMARY KEY (left_item_id, right_item_id)
);
CREATE INDEX idx_members_group ON members(group_id, recommended DESC, filepath);
```

检查点写入必须先写 `checkpoint.bin.tmp` 并调用 `sync_all()`。Windows 下先把现有 `checkpoint.bin` 重命名为 `checkpoint.bin.prev`，再把 tmp 重命名为正式文件，成功后删除 prev；恢复时依次尝试正式文件和 prev。内容使用 `serde_json::to_vec`，避免增加第二套二进制序列化依赖。

- [ ] **Step 5：实现分页和任务状态方法**

至少实现：

```rust
pub fn create(
    app_data: &Path,
    run_id: &str,
    request: &BatchScanRequest,
    index_id: &str,
    index_revision: &str,
) -> Result<Self, String>;
pub fn open(app_data: &Path, run_id: &str) -> Result<Self, String>;
pub fn list(app_data: &Path, root_path: Option<&str>) -> Result<Vec<BatchRunSummary>, String>;
pub fn set_status(&self, status: &str, error: Option<&str>) -> Result<(), String>;
pub fn update_progress(&self, processed_pairs: u64, candidate_pairs: u64, accepted_pairs: u64) -> Result<(), String>;
pub fn summary(&self) -> Result<BatchRunSummary, String>;
pub fn detail(&self) -> Result<BatchRunDetail, String>;
pub fn replace_groups(&self, groups: &[BatchGroup]) -> Result<(), String>;
pub fn get_groups(&self, page: u64, page_size: u64) -> Result<BatchGroupPage, String>;
pub fn get_group_members(&self, group_id: i64, page: u64, page_size: u64) -> Result<BatchMemberPage, String>;
pub fn validate_revision(&self, revision: &str) -> Result<(), String>;
pub fn refresh_stale_status(&self, current_revision: &str) -> Result<bool, String>;
pub fn remove_checkpoint(&self) -> Result<(), String>;
pub fn delete(self) -> Result<u64, String>;
```

所有分页参数在后端限制为 `page_size 1..=200`，所有整数从 SQLite 转换时使用检查转换。

组分页固定使用 `ORDER BY reclaimable_bytes DESC, group_id ASC`；成员分页固定使用 `ORDER BY recommended DESC, filepath COLLATE NOCASE ASC, item_id ASC`，保证同一结果重复打开时顺序稳定。

`run_id` 只接受 1 到 64 个 ASCII 字母、数字或连字符；`create`、`open` 和 `delete` 在拼接路径前都调用同一个校验函数，拒绝斜杠、反斜杠、点号和空字符串。

- [ ] **Step 6：运行存储测试和格式检查**

Run:

```powershell
cd src-tauri
cargo test dedup::batch_store -- --nocapture
cargo fmt -- --check
```

Expected: 存储测试通过；格式检查通过。

## Task 4：统一严格图片对判定器

**Files:**

- Create: `src-tauri/src/dedup/batch.rs`
- Create: `src-tauri/src/dedup/test_support.rs`
- Modify: `src-tauri/src/dedup/mod.rs`
- Modify: `src-tauri/src/dedup/verify.rs`
- Modify: `src-tauri/src/dedup/types.rs`

- [ ] **Step 1：先写完全相同、阈值过滤和风险复核测试**

```rust
#[test]
fn strict_matcher_accepts_exact_hash_without_verifier() {
    let verifier = FakeVerifier::rejecting();
    let fixture = PairFixture::new(true, 0.2, 64);
    let decision = StrictMatcher::new(0.85, &verifier)
        .classify(&fixture.candidate())
        .unwrap();
    assert!(decision.accepted);
    assert_eq!(decision.reason, "sha256");
    assert_eq!(verifier.calls(), 0);
}

#[test]
fn strict_matcher_rejects_non_exact_below_threshold() {
    let verifier = FakeVerifier::accepting();
    let fixture = PairFixture::new(false, 0.84, 2);
    let decision = StrictMatcher::new(0.85, &verifier)
        .classify(&fixture.candidate())
        .unwrap();
    assert!(!decision.accepted);
    assert_eq!(verifier.calls(), 0);
}

#[test]
fn template_risk_requires_local_or_ocr_verification() {
    let verifier = FakeVerifier::accepting();
    let fixture = PairFixture::new(false, 0.90, 22);
    let decision = StrictMatcher::new(0.85, &verifier)
        .classify(&fixture.candidate())
        .unwrap();
    assert!(decision.accepted);
    assert_eq!(decision.reason, "orb");
    assert_eq!(verifier.calls(), 1);
}

struct FakeVerifier {
    accept: bool,
    calls: std::sync::atomic::AtomicUsize,
}

impl FakeVerifier {
    fn accepting() -> Self { Self { accept: true, calls: std::sync::atomic::AtomicUsize::new(0) } }
    fn rejecting() -> Self { Self { accept: false, calls: std::sync::atomic::AtomicUsize::new(0) } }
    fn calls(&self) -> usize { self.calls.load(std::sync::atomic::Ordering::Relaxed) }
}

impl BatchPairVerifier for FakeVerifier {
    fn verify(&self, _left_path: &str, _right_path: &str, _similarity: f32) -> Result<PairVerification, String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(PairVerification {
            local_verified: self.accept,
            template_risk: !self.accept,
            reason: "orb",
        })
    }
}

struct PairFixture {
    left: IndexRecord,
    right: IndexRecord,
    exact: bool,
    similarity: f32,
    phash_distance: u32,
}

impl PairFixture {
    fn new(exact: bool, similarity: f32, phash_distance: u32) -> Self {
        let sha = if exact { vec![1; 32] } else { vec![2; 32] };
        Self {
            left: crate::dedup::test_support::index_record(1, vec![1; 32], 0),
            right: crate::dedup::test_support::index_record(2, sha, phash_distance as u64),
            exact,
            similarity,
            phash_distance,
        }
    }

    fn candidate(&self) -> PairCandidate<'_> {
        PairCandidate {
            left: &self.left,
            right: &self.right,
            exact: self.exact,
            similarity: self.similarity,
            phash_distance: self.phash_distance,
        }
    }
}
```

- [ ] **Step 2：运行测试并确认判定器缺失**

Run:

```powershell
cd src-tauri
cargo test dedup::batch::tests::strict_matcher -- --nocapture
```

Expected: 编译失败，指出 `StrictMatcher` 和测试夹具未定义。

在 `mod.rs` 同时注册：

```rust
#[cfg(test)]
mod test_support;
```

`test_support.rs` 先加入完整记录夹具：

```rust
use super::index::IndexRecord;
use super::types::DedupImageKind;

pub(crate) fn index_record(id: i64, sha256: Vec<u8>, phash: u64) -> IndexRecord {
    IndexRecord {
        id,
        absolute_path: format!("C:/gallery/{id}.png"),
        relative_path: format!("{id}.png"),
        filename: format!("{id}.png"),
        kind: DedupImageKind::Standard,
        file_size: 100,
        modified_ns: 1_000_000,
        width: 64,
        height: 64,
        image_format: "PNG".to_string(),
        sha256,
        phash,
        vector_offset: 0,
        vector_length: (super::types::EMBEDDING_DIM * 4) as u64,
        feature_status: 0,
        error_message: None,
    }
}
```

- [ ] **Step 3：定义可替换复核接口和判定结果**

```rust
pub trait BatchPairVerifier: Send + Sync {
    fn verify(&self, left_path: &str, right_path: &str, similarity: f32) -> Result<PairVerification, String>;
}

pub struct PairVerification {
    pub local_verified: bool,
    pub template_risk: bool,
    pub reason: &'static str,
}

pub struct PairCandidate<'a> {
    pub left: &'a IndexRecord,
    pub right: &'a IndexRecord,
    pub exact: bool,
    pub similarity: f32,
    pub phash_distance: u32,
}

pub struct PairDecision {
    pub accepted: bool,
    pub reason: String,
    pub similarity: f32,
}

pub const STRICT_PROFILE_VERSION: u32 = 1;

pub struct StrictMatcher<'a, V: BatchPairVerifier> {
    threshold: f32,
    verifier: &'a V,
}

impl<'a, V: BatchPairVerifier> StrictMatcher<'a, V> {
    pub fn new(threshold: f32, verifier: &'a V) -> Self {
        Self { threshold: threshold.clamp(-1.0, 1.0), verifier }
    }
}

impl PairDecision {
    pub(crate) fn accepted(similarity: f32, reason: &str) -> Self {
        Self { accepted: true, reason: reason.to_string(), similarity }
    }

    pub(crate) fn rejected(similarity: f32, reason: &str) -> Self {
        Self { accepted: false, reason: reason.to_string(), similarity }
    }
}
```

在 `verify.rs` 增加真实适配器，把现有 `verify_pair` 的 `local_verified`、`template_risk` 和 OCR 结论转换为 `PairVerification`。

- [ ] **Step 4：实现唯一严格判定流程**

```rust
impl<'a, V: BatchPairVerifier> StrictMatcher<'a, V> {
    pub fn classify(&self, candidate: &PairCandidate<'_>) -> Result<PairDecision, String> {
        if candidate.exact {
            return Ok(PairDecision::accepted(candidate.similarity, "sha256"));
        }
        if candidate.similarity < self.threshold {
            return Ok(PairDecision::rejected(candidate.similarity, "below_threshold"));
        }
        if !is_template_risk(candidate.similarity, candidate.phash_distance) {
            return Ok(PairDecision::accepted(candidate.similarity, "sscd"));
        }
        let verification = self.verifier.verify(
            &candidate.left.absolute_path,
            &candidate.right.absolute_path,
            candidate.similarity,
        )?;
        if verification.local_verified && !verification.template_risk {
            Ok(PairDecision::accepted(candidate.similarity, verification.reason))
        } else {
            Ok(PairDecision::rejected(candidate.similarity, "needs_review"))
        }
    }
}
```

该流程保证快速和完整模式不会出现两套不同的“严格相似”语义。

- [ ] **Step 5：运行判定测试和现有复核测试**

Run:

```powershell
cd src-tauri
cargo test dedup::batch::tests::strict_matcher -- --nocapture
cargo test dedup::verify -- --nocapture
```

Expected: 新旧测试全部通过。

## Task 5：可恢复完整分块扫描

**Files:**

- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/src/dedup/batch.rs`
- Modify: `src-tauri/src/dedup/batch_group.rs`
- Modify: `src-tauri/src/dedup/batch_store.rs`
- Modify: `src-tauri/src/dedup/index.rs`
- Modify: `src-tauri/src/dedup/test_support.rs`

- [ ] **Step 1：先写全部无序图片对、恢复和取消测试**

```rust
use crate::dedup::test_support::unit_vector;

#[test]
fn complete_scan_visits_each_unordered_pair_once() {
    let fixture = crate::dedup::test_support::synthetic_snapshot(&[
        unit_vector(0),
        unit_vector(0),
        unit_vector(1),
        unit_vector(2),
    ]);
    let seen = Mutex::new(Vec::new());
    let result = scan_complete(
        &fixture.snapshot,
        -1.0,
        None,
        &AtomicBool::new(false),
        |left, right, similarity| {
            seen.lock().unwrap().push((left.id, right.id));
            Ok(if similarity >= 0.85 {
                PairDecision::accepted(similarity, "test")
            } else {
                PairDecision::rejected(similarity, "test")
            })
        },
        |_| Ok(()),
        |_, _, _, _| {},
    ).unwrap();
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 6);
    assert_eq!(seen.iter().collect::<std::collections::HashSet<_>>().len(), 6);
    assert_eq!(result.accepted_edges.len(), 1);
}

#[test]
fn complete_scan_resume_matches_uninterrupted_result() {
    let fixture = crate::dedup::test_support::synthetic_snapshot_with_count(12);
    let cancel = AtomicBool::new(false);
    let saved = Mutex::new(Vec::<BatchCheckpoint>::new());
    let interrupted = scan_complete(
        &fixture.snapshot,
        0.85,
        None,
        &cancel,
        |_left, _right, similarity| Ok(if similarity >= 0.85 {
            PairDecision::accepted(similarity, "test")
        } else {
            PairDecision::rejected(similarity, "test")
        }),
        |checkpoint| {
            let mut checkpoints = saved.lock().unwrap();
            checkpoints.push(checkpoint.clone());
            if checkpoints.len() == 2 { cancel.store(true, Ordering::Relaxed); }
            Ok(())
        },
        |_, _, _, _| {},
    );
    assert!(interrupted.unwrap_err().contains("已取消"));
    let checkpoint = saved.lock().unwrap().last().cloned().unwrap();
    cancel.store(false, Ordering::Relaxed);
    let resumed = scan_complete(
        &fixture.snapshot,
        0.85,
        Some(checkpoint),
        &cancel,
        |_left, _right, similarity| Ok(if similarity >= 0.85 {
            PairDecision::accepted(similarity, "test")
        } else {
            PairDecision::rejected(similarity, "test")
        }),
        |_| Ok(()),
        |_, _, _, _| {},
    ).unwrap();
    let uninterrupted = scan_complete(
        &fixture.snapshot,
        0.85,
        None,
        &AtomicBool::new(false),
        |_left, _right, similarity| Ok(if similarity >= 0.85 {
            PairDecision::accepted(similarity, "test")
        } else {
            PairDecision::rejected(similarity, "test")
        }),
        |_| Ok(()),
        |_, _, _, _| {},
    ).unwrap();
    assert_eq!(resumed.accepted_edges, uninterrupted.accepted_edges);
    assert_eq!(resumed.processed_pairs, uninterrupted.processed_pairs);
}
```

测试模块导入 `unit_vector`；并在 `test_support.rs` 增加完整临时索引夹具：

```rust
use super::index::{IndexPaths, IndexSnapshot};
use super::types::EMBEDDING_DIM;
use memmap2::MmapOptions;
use std::fs::File;
use std::io::Write;

pub(crate) struct SyntheticSnapshot {
    pub temp: tempfile::TempDir,
    pub snapshot: IndexSnapshot,
    pub vectors: Vec<Vec<f32>>,
}

pub(crate) fn unit_vector(axis: usize) -> Vec<f32> {
    let mut vector = vec![0.0; EMBEDDING_DIM];
    vector[axis % EMBEDDING_DIM] = 1.0;
    vector
}

pub(crate) fn near_unit_vector(axis: usize) -> Vec<f32> {
    let mut vector = unit_vector(axis);
    vector[(axis + 1) % EMBEDDING_DIM] = 0.1;
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    vector.iter_mut().for_each(|value| *value /= norm);
    vector
}

pub(crate) fn synthetic_snapshot(vectors: &[Vec<f32>]) -> SyntheticSnapshot {
    synthetic_snapshot_owned(vectors.to_vec(), "revision-test")
}

pub(crate) fn synthetic_snapshot_with_count(count: usize) -> SyntheticSnapshot {
    let vectors = (0..count).map(|index| unit_vector(index % 4)).collect::<Vec<_>>();
    synthetic_snapshot(&vectors)
}

pub(crate) fn synthetic_snapshot_with_revision(vectors: &[Vec<f32>], revision: &str) -> SyntheticSnapshot {
    synthetic_snapshot_owned(vectors.to_vec(), revision)
}

pub(crate) fn synthetic_snapshot_owned(vectors: Vec<Vec<f32>>, revision: &str) -> SyntheticSnapshot {
    let temp = tempfile::tempdir().unwrap();
    let root_dir = temp.path().join("dedup").join("synthetic");
    std::fs::create_dir_all(&root_dir).unwrap();
    let vector_path = root_dir.join("vectors.bin");
    let mut file = File::create(&vector_path).unwrap();
    for vector in &vectors {
        assert_eq!(vector.len(), EMBEDDING_DIM);
        for value in vector { file.write_all(&value.to_le_bytes()).unwrap(); }
    }
    file.sync_all().unwrap();
    drop(file);
    let mapped_file = File::open(&vector_path).unwrap();
    let mmap = unsafe { MmapOptions::new().map(&mapped_file).unwrap() };
    let records = vectors.iter().enumerate().map(|(index, _)| {
        let mut record = index_record(index as i64 + 1, vec![index as u8; 32], index as u64);
        record.vector_offset = (index * EMBEDDING_DIM * 4) as u64;
        record
    }).collect();
    let paths = IndexPaths {
        index_id: "synthetic".to_string(),
        database: root_dir.join("index.sqlite3"),
        vectors: vector_path,
        root_dir,
    };
    SyntheticSnapshot {
        temp,
        snapshot: IndexSnapshot { paths, revision: revision.to_string(), records, vectors: mmap },
        vectors,
    }
}

pub(crate) fn deterministic_vector_fixture(count: usize) -> SyntheticSnapshot {
    let vectors = (0..count).map(|index| {
        let mut state = index as u64 + 1;
        let mut vector = (0..EMBEDDING_DIM).map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as u32 as f32 / u32::MAX as f32) - 0.5
        }).collect::<Vec<_>>();
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        vector.iter_mut().for_each(|value| *value /= norm);
        vector
    }).collect();
    synthetic_snapshot_owned(vectors, "revision-benchmark")
}
```

- [ ] **Step 2：运行测试并确认扫描函数缺失**

Run:

```powershell
cd src-tauri
cargo test dedup::batch::tests::complete_scan -- --nocapture
```

Expected: 编译失败，指出 `scan_complete` 等未定义。

- [ ] **Step 3：实现精确点积和分块游标**

先在 `Cargo.toml` 增加稳定 SIMD 依赖：

```toml
wide = "0.7.33"
```

在 `index.rs` 增加复用函数：

```rust
pub fn dot_records(snapshot: &IndexSnapshot, left: &IndexRecord, right: &IndexRecord) -> Result<f32, String> {
    let left_bytes = embedding_slice(&snapshot.vectors, left)?;
    let right_bytes = embedding_slice(&snapshot.vectors, right)?;
    let mut dot = 0.0f32;
    for (left_value, right_value) in left_bytes.chunks_exact(4).zip(right_bytes.chunks_exact(4)) {
        dot += f32::from_le_bytes(left_value.try_into().unwrap())
            * f32::from_le_bytes(right_value.try_into().unwrap());
    }
    Ok(dot.clamp(-1.0, 1.0))
}
```

完整扫描使用 `COMPLETE_BLOCK_SIZE: usize = 1024`，按 `(left_block, right_block)` 顺序处理，块内只接受 `left_index < right_index`。

`compare_block` 先用 `embedding_values` 把左右块各解码一次，再用 Rayon 并行左侧行和 `wide::f32x8` 点积：

```rust
fn simd_dot(left: &[f32], right: &[f32]) -> Result<f32, String> {
    if left.len() != right.len() { return Err("向量维度不一致".to_string()); }
    let lane_end = left.len() / 8 * 8;
    let mut sum = wide::f32x8::ZERO;
    for (left_lane, right_lane) in left[..lane_end].chunks_exact(8).zip(right[..lane_end].chunks_exact(8)) {
        let left_array: [f32; 8] = left_lane.try_into().unwrap();
        let right_array: [f32; 8] = right_lane.try_into().unwrap();
        sum += wide::f32x8::new(left_array) * wide::f32x8::new(right_array);
    }
    let tail = left[lane_end..].iter().zip(&right[lane_end..]).map(|(a, b)| a * b).sum::<f32>();
    Ok((sum.reduce_add() + tail).clamp(-1.0, 1.0))
}
```

增加 `simd_dot_matches_scalar_dot` 单元测试，用固定 512 维向量断言 SIMD 与标量结果误差小于 `1e-5`。

在 `batch.rs` 固定扫描返回值和函数签名：

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct BatchScanResult {
    pub accepted_edges: Vec<AcceptedEdge>,
    pub processed_pairs: u64,
    pub candidate_pairs: u64,
    pub duplicate_candidate_pairs: u64,
    pub peak_candidate_buffer: usize,
}

pub fn scan_complete<C, S, P>(
    snapshot: &IndexSnapshot,
    threshold: f32,
    checkpoint: Option<BatchCheckpoint>,
    cancel: &AtomicBool,
    classify: C,
    save_checkpoint: S,
    progress: P,
) -> Result<BatchScanResult, String>
where
    C: Fn(&IndexRecord, &IndexRecord, f32) -> Result<PairDecision, String> + Sync,
    S: Fn(&BatchCheckpoint) -> Result<(), String>,
    P: Fn(u64, u64, u64, u64),
```

函数开始时按 `records` 的稳定 ID 顺序恢复连通状态：

```rust
let valid_records = snapshot.records.iter().filter(|record| record.feature_status == 0)
    .collect::<Vec<_>>();
let item_ids = valid_records.iter().map(|record| record.id).collect::<Vec<_>>();
let mut checkpoint = checkpoint.unwrap_or_else(|| BatchCheckpoint::new(
    0,
    0,
    (0..item_ids.len()).collect(),
    vec![0; item_ids.len()],
    Vec::new(),
));
let mut connectivity = ConnectivityState::from_parts(
    item_ids,
    checkpoint.parent.clone(),
    checkpoint.rank.clone(),
)?;
```

- [ ] **Step 4：实现内存有界扫描和检查点提交**

核心循环：

```rust
for left_block in checkpoint.next_left_block..block_count {
    let first_right = if left_block == checkpoint.next_left_block {
        checkpoint.next_right_block.max(left_block)
    } else {
        left_block
    };
    for right_block in first_right..block_count {
        if cancel.load(Ordering::Relaxed) {
            checkpoint.next_left_block = left_block;
            checkpoint.next_right_block = right_block;
            save_checkpoint(&checkpoint)?;
            return Err("批量完整扫描已取消".to_string());
        }
        let candidates = compare_block(snapshot, &valid_records, left_block, right_block, threshold)?;
        checkpoint.processed_pairs += unordered_pair_count_for_block(
            valid_records.len(),
            left_block,
            right_block,
            COMPLETE_BLOCK_SIZE,
        )?;
        checkpoint.candidate_pairs += candidates.len() as u64;
        for candidate in candidates {
            let decision = classify(candidate.left, candidate.right, candidate.similarity)?;
            if decision.accepted && connectivity.connect(candidate.left.id, candidate.right.id)? {
                checkpoint.accepted_edges.push(AcceptedEdge::new(
                    candidate.left.id,
                    candidate.right.id,
                    decision.similarity,
                    &decision.reason,
                ));
            }
        }
        checkpoint.advance(left_block, right_block, block_count);
        (checkpoint.parent, checkpoint.rank) = connectivity.checkpoint_parts();
        save_checkpoint(&checkpoint)?;
        progress(
            checkpoint.processed_pairs,
            total_pairs,
            checkpoint.candidate_pairs,
            checkpoint.accepted_edges.len() as u64,
        );
    }
}
```

`compare_block` 用 Rayon 按左侧行并行，只返回达到 SSCD 阈值或 SHA-256 相同的候选，不能为所有图片对分配结果数组。

```rust
struct BlockCandidate<'a> {
    left: &'a IndexRecord,
    right: &'a IndexRecord,
    similarity: f32,
}

fn compare_block<'a>(
    snapshot: &IndexSnapshot,
    records: &[&'a IndexRecord],
    left_block: usize,
    right_block: usize,
    threshold: f32,
) -> Result<Vec<BlockCandidate<'a>>, String>;
```

`fn unordered_pair_count_for_block(total: usize, left_block: usize, right_block: usize, block_size: usize) -> Result<u64, String>` 对角块返回 `n * (n - 1) / 2`，非对角块返回 `left_len * right_len`；所有乘法先转换为 `u64` 并使用 `checked_mul`，溢出时返回明确错误。

- [ ] **Step 5：运行完整扫描测试**

Run:

```powershell
cd src-tauri
cargo test dedup::batch::tests::simd_dot -- --nocapture
cargo test dedup::batch::tests::complete_scan -- --nocapture
```

Expected: SIMD 精度、无序图片对、恢复和取消测试通过。

## Task 6：HNSW 派生索引与快速扫描

**Files:**

- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Create: `src-tauri/src/dedup/ann.rs`
- Modify: `src-tauri/src/dedup/mod.rs`
- Modify: `src-tauri/src/dedup/batch.rs`
- Modify: `src-tauri/src/dedup/index.rs`

- [ ] **Step 1：先写 HNSW 往返、失效重建和快速候选测试**

```rust
use crate::dedup::test_support::{near_unit_vector, unit_vector};

#[test]
fn persisted_ann_returns_near_vector_after_reload() {
    let fixture = crate::dedup::test_support::synthetic_snapshot(&[
        unit_vector(0),
        near_unit_vector(0),
        unit_vector(1),
    ]);
    build_ann(&fixture.snapshot).unwrap();
    let neighbours = query_ann(&fixture.snapshot, &fixture.vectors[0], 8).unwrap();
    assert!(neighbours.iter().any(|item| item.d_id == 2));
}

#[test]
fn ann_metadata_revision_mismatch_requires_rebuild() {
    let mut fixture = crate::dedup::test_support::synthetic_snapshot_with_revision(
        &[unit_vector(0), unit_vector(1)],
        "revision-a",
    );
    build_ann(&fixture.snapshot).unwrap();
    fixture.snapshot.revision = "revision-b".to_string();
    assert!(matches!(ann_status(&fixture.snapshot).unwrap(), AnnStatus::Stale));
}

#[test]
fn quick_scan_unions_sha_phash_and_hnsw_candidates_once() {
    let fixture = crate::dedup::test_support::synthetic_snapshot(&[
        unit_vector(0),
        near_unit_vector(0),
        unit_vector(1),
        unit_vector(2),
    ]);
    build_ann(&fixture.snapshot).unwrap();
    let verifier = AlwaysAcceptVerifier;
    let matcher = StrictMatcher::new(0.85, &verifier);
    let result = scan_quick(
        &fixture.snapshot,
        &matcher,
        &AtomicBool::new(false),
        |_, _, _, _| {},
    ).unwrap();
    assert!(result.accepted_edges.iter().any(|edge| edge.left_item_id == 1 && edge.right_item_id == 2));
    assert!(result.candidate_pairs >= result.accepted_edges.len() as u64);
    assert_eq!(result.duplicate_candidate_pairs, 0);
    assert!(result.peak_candidate_buffer <= 256);
}

struct AlwaysAcceptVerifier;

impl BatchPairVerifier for AlwaysAcceptVerifier {
    fn verify(&self, _left_path: &str, _right_path: &str, _similarity: f32) -> Result<PairVerification, String> {
        Ok(PairVerification { local_verified: true, template_risk: false, reason: "orb" })
    }
}
```

- [ ] **Step 2：运行测试并确认依赖和模块缺失**

Run:

```powershell
cd src-tauri
cargo test dedup::ann -- --nocapture
```

Expected: 编译失败，指出 `hnsw_rs` 或 `ann` 未定义。

- [ ] **Step 3：加入固定版本依赖并建立 ANN 元数据**

在 `Cargo.toml` 增加：

```toml
hnsw_rs = "0.3.4"
```

`AnnMetadata` 必须包含：

```rust
#[derive(Serialize, Deserialize)]
pub(crate) struct AnnMetadata {
    schema_version: u32,
    index_revision: String,
    model_version: u32,
    preprocess_version: u32,
    vector_count: usize,
    dimensions: usize,
    max_connections: usize,
    ef_construction: usize,
    pub(crate) ef_search: usize,
    pub(crate) neighbours_per_query: usize,
    basename: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnStatus {
    Missing,
    Ready,
    Stale,
}

pub fn ann_status(snapshot: &IndexSnapshot) -> Result<AnnStatus, String>;
pub fn build_ann(snapshot: &IndexSnapshot) -> Result<(), String>;
fn load_ann_metadata(snapshot: &IndexSnapshot) -> Result<AnnMetadata, String>;
```

加载元数据时校验 `basename` 只能包含 ASCII 字母、数字、连字符和下划线，不能包含路径分隔符；派生元数据损坏一律视为 `AnnStatus::Stale`。

- [ ] **Step 4：实现 HNSW 构建、原生双文件持久化和加载**

```rust
use hnsw_rs::prelude::*;

const MAX_CONNECTIONS: usize = 32;
const EF_CONSTRUCTION: usize = 200;
const EF_SEARCH: usize = 128;
const NEIGHBOURS_PER_QUERY: usize = 64;

let valid_records = snapshot.records.iter()
    .filter(|record| record.feature_status == 0)
    .collect::<Vec<_>>();
let vectors = valid_records.iter().map(|record| embedding_values(&snapshot.vectors, record))
    .collect::<Result<Vec<_>, _>>()?;
let hnsw = Hnsw::<f32, DistDot>::new(
    MAX_CONNECTIONS,
    vectors.len(),
    16.min((vectors.len().max(2) as f32).ln().ceil() as usize),
    EF_CONSTRUCTION,
    DistDot {},
);
let ids = valid_records.iter().map(|record| {
    usize::try_from(record.id).map_err(|_| format!("HNSW item ID 非法: {}", record.id))
}).collect::<Result<Vec<_>, _>>()?;
let inputs = vectors.iter().zip(ids).map(|(vector, id)| (vector, id)).collect::<Vec<_>>();
hnsw.parallel_insert(&inputs);
let basename = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    hnsw.file_dump(&snapshot.paths.root_dir, "neighbors.new")
}))
.map_err(|_| "写入 HNSW 派生索引时发生内部错误".to_string())?
.map_err(|error| format!("写入 HNSW 派生索引失败: {}", error))?;
```

将生成的 `.hnsw.graph` 和 `.hnsw.data` 重命名为稳定名称，最后原子写入 `neighbors.meta.json`。元数据缺失、版本不符或任一数据文件损坏时返回 `AnnStatus::Stale`，不得影响基础索引。

`hnsw_rs::HnswIo::load_hnsw` 的返回生命周期绑定到 loader，因此不要返回裸 `Hnsw`，也不能为每张图片重新加载文件。整个快速扫描必须在一次 loader 生命周期内完成：

```rust
pub(crate) fn with_loaded_ann<R, F>(
    snapshot: &IndexSnapshot,
    operation: F,
) -> Result<R, String>
where
    F: for<'h> FnOnce(&Hnsw<'h, f32, DistDot>, &AnnMetadata) -> Result<R, String>,
{
    let metadata = load_ann_metadata(snapshot)?;
    let mut loader = HnswIo::new(&snapshot.paths.root_dir, &metadata.basename);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let hnsw = loader
            .load_hnsw::<f32, DistDot>()
            .map_err(|error| format!("读取 HNSW 派生索引失败: {}", error))?;
        operation(&hnsw, &metadata)
    }))
    .map_err(|_| "读取 HNSW 派生索引时检测到损坏数据".to_string())?
}

pub(crate) fn query_ann(snapshot: &IndexSnapshot, query: &[f32], count: usize) -> Result<Vec<Neighbour>, String> {
    with_loaded_ann(snapshot, |hnsw, metadata| {
        Ok(hnsw.search(query, count, metadata.ef_search.max(count)))
    })
}
```

- [ ] **Step 5：实现 pHash 分桶与去重候选集合**

按 8 个 8-bit band 建立 `HashMap<(u8, u8), Vec<usize>>`。逐个源记录生成候选，使用当前源记录私有的 `BTreeSet<i64>` 合并 pHash 和 HNSW 候选，并只处理 `source_id < candidate_id`，因此同一无序图片对最多处理一次；不得把整套图库候选永久保存在全局 `HashSet`。SHA-256 相同项按哈希桶直接连接到桶内最小 item ID，避免相同图片很多时生成平方级边。所有通过严格判定的关系先调用 `ConnectivityState::connect`，只有首次连接两个分量时才保存边。HNSW 的近似距离只用于候选生成，最终始终调用 `dot_records` 和 `StrictMatcher`。

快速扫描函数签名固定为：

```rust
pub fn scan_quick<V, P>(
    snapshot: &IndexSnapshot,
    matcher: &StrictMatcher<'_, V>,
    cancel: &AtomicBool,
    progress: P,
) -> Result<BatchScanResult, String>
where
    V: BatchPairVerifier,
    P: Fn(u64, u64, u64, u64),
```

`scan_quick` 先调用一次 `with_loaded_ann`，并在 `operation` 闭包内部遍历全部有效记录、调用 `hnsw.search`、合并当前源记录的 pHash 候选、精确复算和连接分组；测试用 `query_ann` 仅用于单次往返断言，生产循环不得调用它。

- [ ] **Step 6：运行 ANN 和快速扫描测试**

Run:

```powershell
cd src-tauri
cargo test dedup::ann -- --nocapture
cargo test dedup::batch::tests::quick_scan -- --nocapture
```

Expected: HNSW 往返、版本失效和快速分组测试通过。

## Task 7：批量任务编排、完成落库与恢复

**Files:**

- Modify: `src-tauri/src/dedup/batch.rs`
- Modify: `src-tauri/src/dedup/batch_store.rs`
- Modify: `src-tauri/src/dedup/batch_group.rs`
- Modify: `src-tauri/src/dedup/types.rs`

- [ ] **Step 1：先写启动、完成、恢复和过期测试**

```rust
use crate::dedup::test_support::{near_unit_vector, unit_vector};

#[test]
fn completed_run_persists_groups_and_clears_checkpoint() {
    let app_data = tempfile::tempdir().unwrap();
    let fixture = crate::dedup::test_support::synthetic_snapshot(&[
        unit_vector(0),
        near_unit_vector(0),
        unit_vector(1),
    ]);
    let request = BatchScanRequest::complete("C:/gallery", 0.85);
    let verifier = FakeVerifier::accepting();
    let cancel = AtomicBool::new(false);
    let progress = |_payload: BatchProgressPayload| {};
    let context = BatchRunContext {
        app_data: app_data.path(),
        run_id: "run-complete",
        request: &request,
        snapshot: &fixture.snapshot,
        verifier: &verifier,
        cancel: &cancel,
        progress: &progress,
    };
    let summary = run_batch_scan(&context).unwrap();
    assert_eq!(summary.status, "completed");
    assert!(summary.group_count > 0);
    let store = BatchStore::open(app_data.path(), "run-complete").unwrap();
    assert!(store.load_checkpoint().unwrap().is_none());
    assert_eq!(store.get_groups(0, 50).unwrap().total, summary.group_count);
}

#[test]
fn resume_rejects_changed_index_revision() {
    let app_data = tempfile::tempdir().unwrap();
    let fixture = crate::dedup::test_support::synthetic_snapshot_with_revision(
        &[unit_vector(0), near_unit_vector(0)],
        "revision-b",
    );
    let request = BatchScanRequest::complete("C:/gallery", 0.85);
    let store = BatchStore::create(
        app_data.path(),
        "run-stale",
        &request,
        "synthetic",
        "revision-a",
    ).unwrap();
    store.set_status("cancelled", None).unwrap();
    store.save_checkpoint(&BatchCheckpoint::new(0, 0, vec![0, 1], vec![0, 0], vec![])).unwrap();
    let verifier = FakeVerifier::accepting();
    let cancel = AtomicBool::new(false);
    let progress = |_payload: BatchProgressPayload| {};
    let context = BatchRunContext {
        app_data: app_data.path(),
        run_id: "run-stale",
        request: &request,
        snapshot: &fixture.snapshot,
        verifier: &verifier,
        cancel: &cancel,
        progress: &progress,
    };
    let error = resume_batch_scan(&context).unwrap_err();
    assert!(error.contains("索引版本已经变化"));
    assert_eq!(store.detail().unwrap().summary.status, "stale");
}
```

- [ ] **Step 2：运行测试并确认编排 API 缺失**

Run:

```powershell
cd src-tauri
cargo test dedup::batch::tests::completed_run -- --nocapture
cargo test dedup::batch::tests::resume_rejects -- --nocapture
```

Expected: 编译失败，指出 `run_batch_scan` 或 `resume_batch_scan` 未定义。

- [ ] **Step 3：实现任务上下文和 10 万张限制**

```rust
pub struct BatchRunContext<'a, V: BatchPairVerifier> {
    pub app_data: &'a Path,
    pub run_id: &'a str,
    pub request: &'a BatchScanRequest,
    pub snapshot: &'a IndexSnapshot,
    pub verifier: &'a V,
    pub cancel: &'a AtomicBool,
    pub progress: &'a dyn Fn(BatchProgressPayload),
}

const MAX_BATCH_ITEMS: usize = 100_000;
```

有效记录超过限制时返回包含实际数量和限制值的错误；`feature_status != 0` 的记录不参与扫描，并计入跳过统计。

- [ ] **Step 4：实现统一任务生命周期**

`run_batch_scan` 必须按以下状态推进：

```text
created -> preparing -> running -> finalizing -> completed
                              -> cancelled
                              -> failed
                              -> stale
```

创建任务后固定 `index_id`、`revision`、模型/预处理版本、严格判定版本和阈值。快速模式调用 `scan_quick`，完整模式调用 `scan_complete`；扫描结束后调用 `build_groups` 和 `replace_groups`，成功提交结果后删除检查点。

编排函数固定为：

```rust
pub fn run_batch_scan<V: BatchPairVerifier>(context: &BatchRunContext<'_, V>) -> Result<BatchRunSummary, String> {
    let valid_count = context.snapshot.records.iter().filter(|record| record.feature_status == 0).count();
    if valid_count > MAX_BATCH_ITEMS {
        return Err(format!("批量查重最多支持 {} 张，当前索引包含 {} 张有效图片", MAX_BATCH_ITEMS, valid_count));
    }
    let store = BatchStore::create(
        context.app_data,
        context.run_id,
        context.request,
        &context.snapshot.paths.index_id,
        &context.snapshot.revision,
    )?;
    execute_batch_scan(context, &store, None)
}

pub fn resume_batch_scan<V: BatchPairVerifier>(context: &BatchRunContext<'_, V>) -> Result<BatchRunSummary, String> {
    let store = BatchStore::open(context.app_data, context.run_id)?;
    store.validate_revision(&context.snapshot.revision).map_err(|_| {
        let _ = store.set_status("stale", Some("索引版本已经变化"));
        "索引版本已经变化，无法继续扫描".to_string()
    })?;
    let checkpoint = store.load_checkpoint()?.ok_or_else(|| "批量任务没有可恢复检查点".to_string())?;
    execute_batch_scan(context, &store, Some(checkpoint))
}

fn execute_batch_scan<V: BatchPairVerifier>(
    context: &BatchRunContext<'_, V>,
    store: &BatchStore,
    checkpoint: Option<BatchCheckpoint>,
) -> Result<BatchRunSummary, String> {
    store.set_status("running", None)?;
    let matcher = StrictMatcher::new(context.request.threshold, context.verifier);
    let result = match context.request.mode {
        BatchScanMode::Quick => scan_quick(context.snapshot, &matcher, context.cancel, |done, total, candidates, accepted| {
            (context.progress)(BatchProgressPayload::running(context.run_id, "quick", done, total, candidates, accepted));
        }),
        BatchScanMode::Complete => scan_complete(
            context.snapshot,
            context.request.threshold,
            checkpoint,
            context.cancel,
            |left, right, similarity| {
                let candidate = PairCandidate {
                    left,
                    right,
                    exact: left.sha256 == right.sha256,
                    similarity,
                    phash_distance: phash_distance(left.phash, right.phash),
                };
                matcher.classify(&candidate)
            },
            |value| store.save_checkpoint(value),
            |done, total, candidates, accepted| {
                (context.progress)(BatchProgressPayload::running(context.run_id, "complete", done, total, candidates, accepted));
            },
        ),
    };
    let result = match result {
        Ok(value) => value,
        Err(error) if context.cancel.load(Ordering::Relaxed) => {
            store.set_status("cancelled", None)?;
            return Err(error);
        }
        Err(error) => {
            store.set_status("failed", Some(&error))?;
            return Err(error);
        }
    };
    store.set_status("finalizing", None)?;
    let members = context.snapshot.records.iter().filter(|record| record.feature_status == 0)
        .map(BatchMemberSource::from_record).collect::<Vec<_>>();
    let groups = build_groups(&members, &result.accepted_edges);
    store.replace_groups(&groups)?;
    store.update_progress(result.processed_pairs, result.candidate_pairs, result.accepted_edges.len() as u64)?;
    store.remove_checkpoint()?;
    store.set_status("completed", None)?;
    store.summary()
}
```

- [ ] **Step 5：实现恢复和历史任务标准化**

恢复只允许 `cancelled` 或 `interrupted` 的完整任务。应用启动后读取到 `running`、`preparing` 或 `finalizing` 且内存中无对应活动 job 时，把状态标准化为 `interrupted`。索引修订号不匹配时改为 `stale` 并拒绝恢复。

- [ ] **Step 6：运行任务编排测试**

Run:

```powershell
cd src-tauri
cargo test dedup::batch -- --nocapture
```

Expected: 严格判定、完整扫描、快速扫描和生命周期测试全部通过。

## Task 8：Tauri 服务命令、进度事件和锁集成

**Files:**

- Modify: `src-tauri/src/dedup/service.rs`
- Modify: `src-tauri/src/dedup/types.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1：先写服务层可取消任务和并发读测试**

为 `DedupService` 增加不依赖 Tauri State 的单元测试入口，测试：同一个 run ID 不能重复启动；取消后 `AtomicBool` 变为 true；两个读锁可并存；写锁会等待读锁释放。

```rust
#[test]
fn batch_job_registration_is_unique_and_cancellable() {
    let service = DedupService::new();
    let cancel = service.begin_job("run-1").unwrap();
    assert!(service.begin_job("run-1").is_err());
    assert!(service.cancel_job("run-1"));
    assert!(cancel.load(Ordering::Relaxed));
    service.finish_job("run-1");
}

#[test]
fn only_one_batch_scan_runs_per_gallery() {
    let service = DedupService::new();
    service.begin_batch_root("c:/gallery").unwrap();
    assert!(service.begin_batch_root("c:/gallery").is_err());
    service.finish_batch_root("c:/gallery");
    assert!(service.begin_batch_root("c:/gallery").is_ok());
}

#[test]
fn index_lock_allows_parallel_readers_and_excludes_writer() {
    let service = DedupService::new();
    let first_reader = service.inner.index_lock.read().unwrap();
    let second_reader = service.inner.index_lock.try_read().expect("second reader");
    assert!(service.inner.index_lock.try_write().is_err());
    drop(second_reader);
    drop(first_reader);
    assert!(service.inner.index_lock.try_write().is_ok());
}
```

- [ ] **Step 2：运行测试并确认私有接口或命令缺失**

Run:

```powershell
cd src-tauri
cargo test dedup::service -- --nocapture
```

Expected: 新测试在接口可见性或命令缺失处失败。

- [ ] **Step 3：实现批量 Commands**

在 `DedupServiceInner` 增加 `active_batch_roots: Mutex<HashSet<String>>`，`begin_batch_root` 和 `finish_batch_root` 使用规范化图库路径确保同一图库同时最多运行一个批量扫描，避免 HNSW 派生文件和任务结果发生并发写入。

增加并注册：

```rust
dedup_start_batch_scan(job_id: String, request: BatchScanRequest) -> BatchRunSummary
dedup_resume_batch_scan(run_id: String) -> BatchRunSummary
dedup_get_batch_run(run_id: String) -> BatchRunDetail
dedup_list_batch_runs(root_path: Option<String>) -> Vec<BatchRunSummary>
dedup_get_batch_groups(run_id: String, page: u64, page_size: u64) -> BatchGroupPage
dedup_get_batch_group_members(run_id: String, group_id: i64, page: u64, page_size: u64) -> BatchMemberPage
dedup_delete_batch_run(run_id: String) -> DeleteBatchRunResult
```

启动和恢复命令使用 `spawn_blocking`，先注册规范化图库路径，再取得索引 `read()` 锁并保持到扫描完成；索引更新和删除使用 `write()` 锁。任务结束无论成功失败都调用 `finish_job` 和 `finish_batch_root`。

`dedup_list_batch_runs` 取得当前 `jobs` 的 ID 快照，并把数据库中状态为 `preparing`、`running` 或 `finalizing`、但内存中不存在对应 job 的任务标准化为 `interrupted`。对仍存在图库索引的任务读取当前 revision 并调用 `refresh_stale_status`；索引已经删除时保留历史成员快照并把任务标记为 stale。`dedup_delete_batch_run` 在 run ID 仍位于 `jobs` 时返回“请先取消任务”的错误，不删除活动任务目录。

- [ ] **Step 4：发出稳定进度事件**

事件名为 `dedup-batch-progress`，复用 Task 3 已定义的 payload（不要重复声明类型）：

```rust
let payload = BatchProgressPayload::running(
    &run_id,
    "complete",
    processed_pairs,
    total_pairs,
    candidate_pairs,
    accepted_pairs,
);
let _ = progress_app.emit("dedup-batch-progress", payload);
```

节流为最多每 100 ms 发送一次，阶段切换和终态不节流：

```rust
let last_emit = Arc::new(Mutex::new(std::time::Instant::now() - std::time::Duration::from_millis(100)));
let last_phase = Arc::new(Mutex::new(String::new()));
let emit_progress = move |payload: BatchProgressPayload| {
    let terminal = payload.status != "running";
    let phase_changed = last_phase.lock().map(|phase| *phase != payload.phase).unwrap_or(true);
    let due = last_emit.lock().map(|last| last.elapsed() >= std::time::Duration::from_millis(100)).unwrap_or(true);
    if terminal || phase_changed || due {
        if let Ok(mut phase) = last_phase.lock() { *phase = payload.phase.clone(); }
        if let Ok(mut last) = last_emit.lock() { *last = std::time::Instant::now(); }
        let _ = progress_app.emit("dedup-batch-progress", payload);
    }
};
```

- [ ] **Step 5：注册 Commands 并运行全量 Rust 测试**

Run:

```powershell
cd src-tauri
cargo test
cargo fmt -- --check
```

Expected: 全部 Rust 测试通过；格式检查通过。

## Task 9：批量模式界面、分组分页和后台缩略图

**Files:**

- Create: `src/dedup-batch.js`
- Create: `scripts/check_dedup_batch_ui.mjs`
- Modify: `src/dedup.js`
- Modify: `index.html`
- Modify: `src/styles.css`

- [ ] **Step 1：先增加静态 DOM 合约检查脚本**

项目没有前端测试框架，因此使用 Node 内置文件读取检查稳定 ID。在 `scripts/check_dedup_batch_ui.mjs` 新增：

```javascript
import { readFileSync } from 'node:fs';

const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const js = [
  readFileSync(new URL('../src/dedup.js', import.meta.url), 'utf8'),
  readFileSync(new URL('../src/dedup-batch.js', import.meta.url), 'utf8'),
].join('\n');
for (const id of [
  'dedup-mode-single',
  'dedup-mode-batch',
  'dedup-batch-scan-mode',
  'dedup-batch-runs',
  'dedup-batch-groups',
]) {
  if (!html.includes(`id="${id}"`)) throw new Error(`missing #${id}`);
  if (!js.includes(`#${id}`)) throw new Error(`unused #${id}`);
}
console.log('dedup batch UI contract OK');
```

- [ ] **Step 2：运行检查并确认文件或元素缺失**

Run:

```powershell
node scripts/check_dedup_batch_ui.mjs
```

Expected: FAIL，指出 `src/dedup-batch.js` 或首个批量元素不存在。

- [ ] **Step 3：增加模式和批量结果 DOM**

在设置区顶部增加单图/批量切换；批量模式包含扫描方式、任务历史、恢复按钮和组容器：

```html
<div class="dedup-mode-switch" role="tablist" aria-label="查重模式">
  <button id="dedup-mode-single" role="tab" aria-selected="true">单张图片查重</button>
  <button id="dedup-mode-batch" role="tab" aria-selected="false">文件夹批量查重</button>
</div>
<label id="dedup-batch-scan-mode-wrap" hidden>扫描方式
  <select id="dedup-batch-scan-mode">
    <option value="quick">快速扫描</option>
    <option value="complete">完整扫描</option>
  </select>
</label>
<section id="dedup-batch-runs" hidden aria-label="批量任务"></section>
<section id="dedup-batch-groups" hidden aria-live="polite"></section>
```

把现有源图片行改为 `<div id="dedup-source-field" class="dedup-field dedup-field-wide">`。单图结果头、表格和分页保留原 ID，并共同包入 `<section id="dedup-single-results">`，避免两种模式共享含义不同的 DOM。

- [ ] **Step 4：实现独立批量控制器**

`dedup-batch.js` 导出：

```javascript
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export function createDedupBatchController({
  getGalleryPath,
  getThreshold,
  ensureIndex,
  onJobStart,
  onJobFinish,
  setMessage,
  openFile,
  formatSize,
}) {
  const state = {
    activeRunId: null,
    selectedRunId: null,
    groupsPage: 0,
    groupPageSize: 50,
    expandedGroupId: null,
    thumbnailGeneration: 0,
    thumbnailQueue: [],
    thumbnailActive: 0,
  };
  listen('dedup-batch-progress', event => {
    const payload = event.payload;
    if (payload.run_id !== state.activeRunId) return;
    const percent = payload.total_pairs > 0
      ? ((payload.processed_pairs / payload.total_pairs) * 100).toFixed(1)
      : '0.0';
    setMessage(`${payload.phase} ${percent}% · 候选 ${payload.candidate_pairs} · 已连接 ${payload.accepted_pairs}`, false);
  });
  return {
    activate,
    deactivate,
    start,
    refreshRuns,
    cancelActive,
  };

  async function activate() {
    document.querySelector('#dedup-batch-runs').hidden = false;
    document.querySelector('#dedup-batch-groups').hidden = false;
    await refreshRuns();
  }

  function deactivate() {
    state.thumbnailGeneration += 1;
    state.thumbnailQueue = [];
    document.querySelector('#dedup-batch-runs').hidden = true;
    document.querySelector('#dedup-batch-groups').hidden = true;
  }

  async function start() {
    const rootPath = getGalleryPath();
    if (!rootPath) throw new Error('请先选择目标图库');
    const indexSummary = await ensureIndex();
    const mode = document.querySelector('#dedup-batch-scan-mode').value;
    if (mode === 'complete') {
      const count = Number(indexSummary.final_count) || 0;
      const pairs = (count * Math.max(0, count - 1)) / 2;
      if (!confirm(`完整扫描将检查 ${pairs.toLocaleString()} 个图片对，可能运行很久。是否继续？`)) return;
    }
    const runId = crypto.randomUUID();
    state.activeRunId = runId;
    onJobStart(runId, '正在执行文件夹批量查重...');
    try {
      const summary = await invoke('dedup_start_batch_scan', {
        jobId: runId,
        request: {
          root_path: rootPath,
          mode,
          threshold: getThreshold(),
        },
      });
      state.selectedRunId = summary.run_id;
      await Promise.all([refreshRuns(), loadGroupPage(summary.run_id, 0)]);
      onJobFinish(`批量查重完成：${summary.group_count} 组`, false);
    } catch (error) {
      onJobFinish(`批量查重失败：${error}`, true);
    } finally {
      state.activeRunId = null;
    }
  }

  async function refreshRuns() {
    const runs = await invoke('dedup_list_batch_runs', { rootPath: getGalleryPath() || null });
    renderRunList(runs);
  }

  function renderRunList(runs) {
    const container = document.querySelector('#dedup-batch-runs');
    container.replaceChildren(...runs.map(run => {
      const row = document.createElement('div');
      row.className = `dedup-batch-run status-${run.status}`;
      const open = document.createElement('button');
      open.type = 'button';
      open.textContent = `${run.mode === 'quick' ? '快速' : '完整'} · ${run.status} · ${run.group_count} 组`;
      open.addEventListener('click', () => {
        state.selectedRunId = run.run_id;
        loadGroupPage(run.run_id, 0).catch(error => setMessage(String(error), true));
      });
      row.append(open);
      if (run.status === 'cancelled' || run.status === 'interrupted') {
        const resume = document.createElement('button');
        resume.type = 'button';
        resume.textContent = '继续扫描';
        resume.addEventListener('click', () => resumeRun(run.run_id));
        row.append(resume);
      }
      const remove = document.createElement('button');
      remove.type = 'button';
      remove.textContent = '删除记录';
      remove.disabled = run.status === 'preparing' || run.status === 'running' || run.status === 'finalizing';
      remove.addEventListener('click', async () => {
        if (!confirm(`确定删除批量查重记录 ${run.run_id} 吗？\n不会删除图库索引或原始图片。`)) return;
        await invoke('dedup_delete_batch_run', { runId: run.run_id });
        await refreshRuns();
      });
      row.append(remove);
      return row;
    }));
  }

  async function resumeRun(runId) {
    state.activeRunId = runId;
    onJobStart(runId, '正在继续完整扫描...');
    try {
      const summary = await invoke('dedup_resume_batch_scan', { runId });
      await Promise.all([refreshRuns(), loadGroupPage(runId, 0)]);
      onJobFinish(`批量查重完成：${summary.group_count} 组`, false);
    } catch (error) {
      onJobFinish(`继续扫描失败：${error}`, true);
    } finally {
      state.activeRunId = null;
    }
  }

  async function cancelActive() {
    if (state.activeRunId) await invoke('dedup_cancel_job', { jobId: state.activeRunId });
  }
}
```

控制器监听 `dedup-batch-progress`，启动前调用 `ensureIndex()`，使用 `crypto.randomUUID()` 同时作为 job ID 和 run ID。完整任务取消后显示“继续扫描”；索引版本变化时显示“结果已过期”。

`dedup.js` 只负责模式切换和公共任务桥接：

```javascript
import { createDedupBatchController } from './dedup-batch.js';

dedupState.mode = 'single';
let batchController = null;

function initializeBatchController() {
  batchController = createDedupBatchController({
    getGalleryPath: () => dedupState.galleryPath,
    getThreshold: currentThreshold,
    ensureIndex: () => runIndexUpdate(dedupState.galleryPath),
    onJobStart: beginTask,
    onJobFinish: finishTask,
    setMessage,
    openFile: async path => {
      hideDedupWorkspace();
      await openFileCallback(path);
    },
    formatSize,
  });
}

function setDedupMode(mode) {
  dedupState.mode = mode;
  const batch = mode === 'batch';
  document.querySelector('#dedup-mode-single').setAttribute('aria-selected', String(!batch));
  document.querySelector('#dedup-mode-batch').setAttribute('aria-selected', String(batch));
  document.querySelector('#dedup-source-field').hidden = batch;
  document.querySelector('#dedup-batch-scan-mode-wrap').hidden = !batch;
  document.querySelector('#dedup-single-results').hidden = batch;
  document.querySelector('#dedup-start').textContent = batch ? '开始批量查重' : '开始查重';
  if (batch) batchController.activate().catch(error => setMessage(String(error), true));
  else batchController.deactivate();
}

async function startCurrentDedupMode() {
  if (dedupState.mode === 'batch') return batchController.start();
  return startDedupSearch();
}

async function cancelActiveJob() {
  if (dedupState.mode === 'batch') return batchController.cancelActive();
  if (dedupState.activeJobId) await invoke('dedup_cancel_job', { jobId: dedupState.activeJobId });
}
```

在 `setupDedupTool` 中设置 `openFileCallback` 后调用一次 `initializeBatchController`。把 `#dedup-start` 的监听改为 `startCurrentDedupMode`，并为两个模式按钮注册 `setDedupMode`。现有 `cancelActiveJob` 不再重复定义。

- [ ] **Step 5：实现重复组和成员分页**

组卡片只加载当前页；展开一个组时调用 `dedup_get_batch_group_members`。组内成员缩略图复用 `dedup_get_result_thumbnail(rootPath, itemId)`，继续采用 `IntersectionObserver`、3 路并发和结果代次校验。索引已删除的历史成员显示路径快照和“无法预览”，不尝试使用失效 item ID。

```javascript
async function loadGroupPage(runId, page = 0) {
  const response = await invoke('dedup_get_batch_groups', {
    runId,
    page,
    pageSize: state.groupPageSize,
  });
  state.groupsPage = response.page;
  const container = document.querySelector('#dedup-batch-groups');
  container.replaceChildren(...response.items.map(renderGroupCard));
}

function renderGroupCard(group) {
  const card = document.createElement('article');
  card.className = 'dedup-group-card';
  card.dataset.groupId = String(group.group_id);
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'dedup-group-toggle';
  button.setAttribute('aria-expanded', 'false');
  button.textContent = `${group.member_count} 张 · 可释放 ${formatSize(group.reclaimable_bytes)}`;
  const members = document.createElement('div');
  members.className = 'dedup-group-members';
  members.hidden = true;
  button.addEventListener('click', () => toggleGroup(group.group_id, button, members));
  card.append(button, members);
  return card;
}

async function toggleGroup(groupId, button, container) {
  const opening = container.hidden;
  button.setAttribute('aria-expanded', String(opening));
  container.hidden = !opening;
  if (!opening || container.dataset.loaded === 'true') return;
  const response = await invoke('dedup_get_batch_group_members', {
    runId: state.selectedRunId,
    groupId,
    page: 0,
    pageSize: 200,
  });
  container.replaceChildren(...response.items.map(renderBatchMember));
  container.dataset.loaded = 'true';
  observeVisibleBatchThumbnails(container, ++state.thumbnailGeneration);
}

function renderBatchMember(member) {
  const item = document.createElement('div');
  item.className = `dedup-group-member${member.recommended ? ' recommended' : ''}`;
  const preview = document.createElement('div');
  preview.className = 'dedup-thumb-placeholder';
  preview.textContent = '后台加载';
  preview.dataset.itemId = String(member.item_id);
  const name = document.createElement('button');
  name.type = 'button';
  name.className = 'dedup-member-open';
  name.textContent = `${member.recommended ? '建议保留 · ' : ''}${member.filename}`;
  name.title = member.filepath;
  name.addEventListener('click', () => openFile(member.filepath));
  item.append(preview, name);
  return item;
}

function observeVisibleBatchThumbnails(container, generation) {
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      observer.unobserve(entry.target);
      state.thumbnailQueue.push({ element: entry.target, generation });
      pumpBatchThumbnails();
    }
  }, { root: container, rootMargin: '120px' });
  container.querySelectorAll('.dedup-thumb-placeholder').forEach(element => observer.observe(element));
}

function pumpBatchThumbnails() {
  while (state.thumbnailActive < 3 && state.thumbnailQueue.length) {
    const request = state.thumbnailQueue.shift();
    state.thumbnailActive += 1;
    loadBatchThumbnail(request).finally(() => {
      state.thumbnailActive -= 1;
      pumpBatchThumbnails();
    });
  }
}

async function loadBatchThumbnail({ element, generation }) {
  try {
    const payload = await invoke('dedup_get_result_thumbnail', {
      rootPath: getGalleryPath(),
      itemId: Number(element.dataset.itemId),
    });
    if (generation !== state.thumbnailGeneration || !element.isConnected) return;
    const image = document.createElement('img');
    image.className = 'dedup-result-thumb';
    image.alt = '';
    image.src = `data:${payload.mime};base64,${payload.bytes_base64}`;
    element.replaceWith(image);
  } catch {
    if (generation === state.thumbnailGeneration && element.isConnected) element.textContent = '无法预览';
  }
}
```

- [ ] **Step 6：增加响应式样式和键盘语义**

组卡片在宽屏显示摘要与成员预览，在 600 px 窗口折为单列；模式按钮、展开按钮、恢复按钮和成员操作支持 Tab、Enter、Space。不要用颜色作为过期、失败或推荐状态的唯一表达。

- [ ] **Step 7：运行 UI 合约和生产构建**

Run:

```powershell
node scripts/check_dedup_batch_ui.mjs
npm run build
```

Expected: 输出 `dedup batch UI contract OK`；Vite 构建成功。

## Task 10：真实图片集成测试、性能基准和发布验收

**Files:**

- Modify: `src-tauri/src/dedup/tests.rs`
- Modify: `README.md`
- Modify: `IMAGE_DEDUP_DESIGN.md`

- [ ] **Step 1：先写真实图片批量分组集成测试**

复用 `patterned_image` 创建原图、缩放图、裁剪图、重压缩图、无关图和 IMS 容器。建立索引后分别运行快速和完整扫描：

```rust
struct IndexedBatchFixture {
    workspace: tempfile::TempDir,
    gallery: std::path::PathBuf,
    app_data: std::path::PathBuf,
}

struct IntegrationVerifier;

impl BatchPairVerifier for IntegrationVerifier {
    fn verify(&self, _left_path: &str, _right_path: &str, _similarity: f32) -> Result<PairVerification, String> {
        Ok(PairVerification { local_verified: true, template_risk: false, reason: "orb" })
    }
}

fn indexed_batch_gallery_fixture() -> IndexedBatchFixture {
    let workspace = tempfile::tempdir().unwrap();
    let gallery = workspace.path().join("gallery");
    let app_data = workspace.path().join("app-data");
    std::fs::create_dir_all(&gallery).unwrap();
    let source_path = gallery.join("source.png");
    let resized_path = gallery.join("resized.jpg");
    let cropped_path = gallery.join("cropped.png");
    let recompressed_path = gallery.join("recompressed.jpg");
    let unrelated_path = gallery.join("unrelated.png");
    let ims_path = gallery.join("container.ims.png");
    let source = patterned_image(320, 240);
    source.save(&source_path).unwrap();
    source.resize_exact(960, 720, image::imageops::FilterType::Lanczos3).save(&resized_path).unwrap();
    source.crop_imm(32, 24, 256, 192)
        .resize_exact(320, 240, image::imageops::FilterType::Lanczos3)
        .save(&cropped_path).unwrap();
    let mut jpeg = std::fs::File::create(&recompressed_path).unwrap();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 55).encode_image(&source).unwrap();
    DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 240, Rgb([4, 240, 40])))
        .save(&unrelated_path).unwrap();
    pack_ims(
        &[source_path.to_string_lossy().to_string(), unrelated_path.to_string_lossy().to_string()],
        0,
        &ims_path.to_string_lossy(),
    ).unwrap();
    let model_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources").join("models").join("sscd_disc_mixup.onnx");
    let model = Arc::new(SscdModel::load(&model_path, false, 2).unwrap());
    let config = DedupIndexConfig {
        root_path: gallery.to_string_lossy().to_string(),
        recursive: true,
        max_depth: 0,
        multi_threaded: true,
        worker_count: Some(2),
    };
    update_index(&app_data, &config, model, &AtomicBool::new(false), |_, _, _, _| {}).unwrap();
    IndexedBatchFixture { workspace, gallery, app_data }
}

fn run_test_batch(fixture: &IndexedBatchFixture, mode: BatchScanMode, threshold: f32) -> Result<Vec<std::collections::BTreeSet<String>>, String> {
    let snapshot = open_index_snapshot(&fixture.app_data, &fixture.gallery)?;
    let run_id = match mode { BatchScanMode::Quick => "integration-quick", BatchScanMode::Complete => "integration-complete" };
    let request = BatchScanRequest { root_path: fixture.gallery.to_string_lossy().to_string(), mode, threshold };
    let verifier = IntegrationVerifier;
    let cancel = AtomicBool::new(false);
    let progress = |_payload: BatchProgressPayload| {};
    run_batch_scan(&BatchRunContext {
        app_data: &fixture.app_data,
        run_id,
        request: &request,
        snapshot: &snapshot,
        verifier: &verifier,
        cancel: &cancel,
        progress: &progress,
    })?;
    let store = BatchStore::open(&fixture.app_data, run_id)?;
    let groups = store.get_groups(0, 200)?;
    groups.items.into_iter().map(|group| {
        let members = store.get_group_members(group.group_id, 0, 200)?;
        Ok(members.items.into_iter().map(|member| member.filename).collect())
    }).collect()
}

#[test]
fn batch_quick_and_complete_group_transformed_images() {
    let fixture = indexed_batch_gallery_fixture();
    let quick = run_test_batch(&fixture, BatchScanMode::Quick, 0.65).unwrap();
    let complete = run_test_batch(&fixture, BatchScanMode::Complete, 0.65).unwrap();

    let expected = std::collections::BTreeSet::from([
        "source.png",
        "resized.jpg",
        "cropped.png",
        "recompressed.jpg",
        "container.ims.png",
    ]);
    assert_eq!(quick, vec![expected.clone()]);
    assert_eq!(complete, vec![expected]);
    assert!(!complete.iter().any(|group| group.contains("unrelated.png")));
}
```

- [ ] **Step 2：增加取消恢复和索引过期集成测试**

完整扫描在第一个检查点设置取消标志，断言状态为 `cancelled`、检查点存在；恢复后结果与不中断扫描一致。随后成功更新图库索引，断言旧任务状态转为 `stale` 且恢复返回明确错误。

```rust
#[test]
fn complete_batch_cancel_resume_and_stale_round_trip() {
    let app_data = tempfile::tempdir().unwrap();
    let fixture = crate::dedup::test_support::synthetic_snapshot_with_count(2_050);
    let request = BatchScanRequest::complete("C:/gallery", 0.99);
    let verifier = IntegrationVerifier;
    let cancel = AtomicBool::new(false);
    let cancelling_progress = |payload: BatchProgressPayload| {
        if payload.processed_pairs > 0 { cancel.store(true, Ordering::Relaxed); }
    };
    let context = BatchRunContext {
        app_data: app_data.path(),
        run_id: "cancel-resume",
        request: &request,
        snapshot: &fixture.snapshot,
        verifier: &verifier,
        cancel: &cancel,
        progress: &cancelling_progress,
    };
    assert!(run_batch_scan(&context).unwrap_err().contains("已取消"));
    let store = BatchStore::open(app_data.path(), "cancel-resume").unwrap();
    assert_eq!(store.summary().unwrap().status, "cancelled");
    assert!(store.load_checkpoint().unwrap().is_some());

    cancel.store(false, Ordering::Relaxed);
    let quiet_progress = |_payload: BatchProgressPayload| {};
    let resumed = resume_batch_scan(&BatchRunContext {
        progress: &quiet_progress,
        ..context
    }).unwrap();
    assert_eq!(resumed.status, "completed");
    assert!(store.load_checkpoint().unwrap().is_none());

    assert!(store.refresh_stale_status("revision-after-index-update").unwrap());
    assert_eq!(store.summary().unwrap().status, "stale");
}
```

- [ ] **Step 3：增加忽略的 10 万向量性能基准**

```rust
#[test]
#[ignore = "manual 100k batch benchmark"]
fn benchmark_batch_scan_100k_vectors() {
    let fixture = crate::dedup::test_support::deterministic_vector_fixture(100_000);
    build_ann(&fixture.snapshot).unwrap();
    let verifier = IntegrationVerifier;
    let matcher = StrictMatcher::new(0.85, &verifier);
    let started = std::time::Instant::now();
    let stats = scan_quick(
        &fixture.snapshot,
        &matcher,
        &AtomicBool::new(false),
        |_, _, _, _| {},
    ).unwrap();
    eprintln!("elapsed={:?} stats={:?}", started.elapsed(), stats);
    assert!(stats.peak_candidate_buffer < 10_000);
}

#[test]
#[ignore = "manual 100k complete block throughput benchmark"]
fn benchmark_batch_complete_block_throughput_100k() {
    let fixture = crate::dedup::test_support::deterministic_vector_fixture(100_000);
    let cancel = AtomicBool::new(false);
    let checkpoint = Mutex::new(None::<BatchCheckpoint>);
    let started = std::time::Instant::now();
    let result = scan_complete(
        &fixture.snapshot,
        0.99,
        None,
        &cancel,
        |_left, _right, similarity| Ok(PairDecision::rejected(similarity, "benchmark")),
        |value| {
            *checkpoint.lock().unwrap() = Some(value.clone());
            cancel.store(true, Ordering::Relaxed);
            Ok(())
        },
        |_, _, _, _| {},
    );
    assert!(result.unwrap_err().contains("已取消"));
    let processed = checkpoint.lock().unwrap().as_ref().unwrap().processed_pairs;
    let pairs_per_second = processed as f64 / started.elapsed().as_secs_f64();
    eprintln!("processed_pairs={processed} pairs_per_second={pairs_per_second:.0}");
    assert!(processed > 0);
}
```

该测试只在性能验收时手动运行，不进入日常 CI。

- [ ] **Step 4：运行完整验证矩阵**

Run:

```powershell
cd src-tauri
cargo test
cargo test benchmark_batch -- --ignored --nocapture
cd ..
node scripts/check_dedup_batch_ui.mjs
npm run build
npm run tauri build -- --no-bundle
```

Expected: Rust 全套测试通过；手动基准输出耗时和统计且内存断言通过；UI 合约、Vite 和 Tauri Release 构建成功。

- [ ] **Step 5：执行 Windows 原生冒烟测试**

使用至少包含 60 张图片、缩放图、裁剪图、重压缩图、无关图和 IMS 封面的测试图库验证：

1. `工具 → 图片查重 → 文件夹批量查重` 可以进入。
2. 快速模式建立或复用索引并显示重复组。
3. 展开组时成员缩略图后台加载，滚动和翻页保持可操作。
4. 完整模式显示总图片对、进度、取消和继续扫描。
5. A-B、B-C 连通组只出现一次，每张图片只属于一个组。
6. 推荐保留项、相似度范围、空间估算和文件操作正确。
7. 更新图库索引后旧任务显示为过期。
8. 删除任务记录不删除图库索引或任何原始图片。

- [ ] **Step 6：同步文档状态和实际参数**

在 `IMAGE_DEDUP_DESIGN.md` 把 Phase 5 批量项更新为实际完成状态，记录最终 HNSW 参数、完整扫描块大小、测试数量和构建结果。`README.md` 增加快速/完整模式差异、完整扫描耗时提示、恢复方法和“不自动删除文件”说明。

- [ ] **Step 7：最终差异和格式检查**

Run:

```powershell
git diff --check
```

Expected: 无输出。检查工作区只包含本功能及此前用户已有改动，不覆盖无关文件。

## 完成定义

- 用户可以对一套 10 万张以内的本地图库启动快速或完整批量查重。
- 严格近重复允许缩放、重压缩、轻微裁剪和调色，并排除无关内容。
- 结果按连通关系分组，每张图片最多属于一个组。
- 快速模式使用持久 HNSW 派生索引并精确复算候选分数。
- 完整模式遍历全部有效无序图片对，支持取消、崩溃恢复和索引版本校验。
- 任务、结果、成员快照和检查点可持久化、分页和删除。
- 缩略图按可见区域后台加载，不阻塞界面。
- 不自动删除、移动或覆盖任何原始图片。
- Rust 测试、前端构建、Tauri Release 构建和 Windows 原生冒烟测试全部通过。

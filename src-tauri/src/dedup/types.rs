use serde::{Deserialize, Serialize};

pub const MODEL_ID: &str = "sscd_disc_mixup";
pub const MODEL_VERSION: u32 = 1;
pub const PREPROCESS_VERSION: u32 = 1;
pub const EMBEDDING_DIM: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupImageKind {
    Standard = 0,
    ImsCover = 1,
}

impl DedupImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::ImsCover => "ims_cover",
        }
    }

    pub fn from_i64(value: i64) -> Self {
        if value == Self::ImsCover as i64 {
            Self::ImsCover
        } else {
            Self::Standard
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DedupIndexConfig {
    pub root_path: String,
    #[serde(default = "default_true")]
    pub recursive: bool,
    #[serde(default)]
    pub max_depth: u32,
    #[serde(default = "default_true")]
    pub multi_threaded: bool,
    pub worker_count: Option<usize>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct DedupSearchRequest {
    pub source_path: String,
    pub root_path: String,
    pub threshold: f32,
    #[serde(default)]
    pub exclude_source: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupModelStatus {
    pub available: bool,
    pub model_id: String,
    pub model_version: u32,
    pub provider: Option<String>,
    pub model_path: Option<String>,
    pub model_sha256: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupIndexStatus {
    pub index_id: String,
    pub root_path: String,
    pub max_depth: u32,
    pub revision: Option<String>,
    pub exists: bool,
    pub item_count: u64,
    pub total_bytes: u64,
    pub last_updated_ms: Option<u64>,
    pub model_version: Option<u32>,
    pub preprocess_version: Option<u32>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupIndexSummary {
    pub index_id: String,
    pub root_path: String,
    pub max_depth: u32,
    pub revision: Option<String>,
    pub item_count: u64,
    pub total_bytes: u64,
    pub last_updated_ms: Option<u64>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupIndexUpdateSummary {
    pub index_id: String,
    pub root_path: String,
    pub max_depth: u32,
    pub unchanged: u64,
    pub added: u64,
    pub updated: u64,
    pub removed: u64,
    pub failed: u64,
    pub final_count: u64,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteDedupIndexResult {
    pub deleted_indexes: u64,
    pub released_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupSearchResult {
    pub item_id: i64,
    pub filepath: String,
    pub filename: String,
    pub kind: String,
    pub image_format: String,
    pub width: u32,
    pub height: u32,
    pub modified_ms: i64,
    pub raw_similarity: f32,
    pub confidence: f32,
    pub match_type: String,
    pub exact: bool,
    pub phash_distance: u32,
    pub template_risk: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupSearchResponse {
    pub source_path: String,
    pub root_path: String,
    pub threshold: f32,
    pub elapsed_ms: u64,
    pub total_indexed: u64,
    pub results: Vec<DedupSearchResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThumbnailPayload {
    pub mime: String,
    pub bytes_base64: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DedupVerificationRequest {
    pub source_path: String,
    pub candidate_path: String,
    pub raw_similarity: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupVerificationResult {
    pub local_match_score: f32,
    pub local_verified: bool,
    pub source_text: String,
    pub candidate_text: String,
    pub ocr_similarity: f32,
    pub template_risk: bool,
    pub verdict: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupProgressPayload {
    pub job_id: String,
    pub phase: String,
    pub current: u64,
    pub total: u64,
    pub current_file: Option<String>,
    pub message: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchScanMode {
    Quick,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchScanRequest {
    pub root_paths: Vec<String>,
    pub mode: BatchScanMode,
    pub threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchMemberSource {
    pub batch_item_id: i64,
    pub item_id: i64,
    pub index_root_path: String,
    pub filepath: String,
    pub filename: String,
    pub kind: String,
    pub image_format: String,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub modified_ms: i64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchGroupMember {
    pub batch_item_id: i64,
    pub item_id: i64,
    pub index_root_path: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRunSummary {
    pub run_id: String,
    pub root_paths: Vec<String>,
    pub mode: BatchScanMode,
    pub status: String,
    pub index_revision: String,
    pub threshold: f32,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub processed_pairs: u64,
    pub total_pairs: u64,
    pub candidate_pairs: u64,
    pub accepted_pairs: u64,
    pub group_count: u64,
    pub resumable: bool,
    pub error_message: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchCheckpoint {
    pub next_left_block: usize,
    pub processed_pairs: u64,
    pub candidate_pairs: u64,
    pub parent: Vec<usize>,
    pub rank: Vec<u8>,
    pub accepted_edges: Vec<AcceptedEdge>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteBatchRunResult {
    pub deleted_runs: u64,
    pub released_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteDedupFileResult {
    pub deleted: bool,
    pub filepath: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchAutoDeletePreview {
    pub threshold: f32,
    pub candidate_count: u64,
    pub candidate_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchAutoDeleteFailure {
    pub filepath: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchAutoDeleteResult {
    pub threshold: f32,
    pub candidate_count: u64,
    pub deleted_count: u64,
    pub missing_count: u64,
    pub failed_count: u64,
    pub released_bytes: u64,
    pub remaining_group_count: u64,
    pub failures: Vec<BatchAutoDeleteFailure>,
}

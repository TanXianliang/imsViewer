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
    pub item_count: u64,
    pub total_bytes: u64,
    pub last_updated_ms: Option<u64>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupIndexUpdateSummary {
    pub index_id: String,
    pub root_path: String,
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

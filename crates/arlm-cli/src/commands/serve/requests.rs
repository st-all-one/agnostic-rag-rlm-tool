use serde::Deserialize;

/// Request body for `POST /context`.
#[derive(Deserialize)]
pub struct ContextRequest {
    pub task: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Agent name for metrics tracking.
    #[serde(default)]
    pub agent: Option<String>,
}

/// Request body for `POST /search`.
#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    pub file_pattern: Option<String>,
    pub min_score: Option<f32>,
    /// Agent name for metrics tracking.
    #[serde(default)]
    pub agent: Option<String>,
}

/// Request body for `POST /index`.
#[derive(Deserialize)]
pub struct IndexRequest {
    pub path: Option<String>,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
}

fn default_top_k() -> usize {
    10
}

fn default_chunk_size() -> usize {
    512
}

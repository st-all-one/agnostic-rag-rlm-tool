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

/// Request body for `POST /run`.
#[derive(Deserialize)]
pub struct RunRequest {
    pub task: String,
    #[serde(default = "default_depth")]
    pub depth: u32,
    #[serde(default = "default_max_nodes")]
    pub max_nodes: u32,
    pub backend: Option<String>,
    pub model: Option<String>,
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

fn default_depth() -> u32 {
    3
}

fn default_max_nodes() -> u32 {
    50
}

fn default_chunk_size() -> usize {
    512
}

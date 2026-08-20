//! Hierarchical summarization engine.
//!
//! Produces file → module → project summaries for a project's chunks and
//! persists them in the `summaries` table. Runs on the background worker (see
//! [`worker`]) and publishes live progress to the server event hub.

pub mod cost;
pub mod engine;
pub mod progress;
pub mod strategy;
pub mod worker;

use sha2::{Digest, Sha256};

pub use engine::Summarizer;
pub use worker::{SummarizeSender, spawn_worker};

/// A summarization job enqueued by `trigger_summarize`.
#[derive(Debug, Clone)]
pub struct SummarizeJob {
    pub run_id: String,
    pub buffer_id: i64,
    pub project: String,
    /// Proto `SummaryScope` enum value (0=file, 1=module, 2=project).
    pub max_scope: i32,
    pub max_concurrent: u32,
    pub force_refresh: bool,
}

/// Result of a summarization pass.
#[derive(Debug, Clone, Default)]
pub struct SummaryResult {
    pub file_summaries: u32,
    pub module_summaries: u32,
    pub project_summaries: u32,
    pub total_summarized: u32,
}

/// Compute a SHA-256 hex digest for change detection.
#[must_use]
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Rough token estimate (1 token per 4 characters).
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32).saturating_div(4)
}

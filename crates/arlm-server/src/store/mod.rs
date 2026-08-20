//! Typed, pool-safe data access for the gRPC handlers.
//!
//! The server runs the storage pool in `Pooled` mode, where the
//! single-connection helpers on [`arlm_storage::Storage`] would panic. Every
//! query here goes through [`arlm_storage::Storage::connection`], which works
//! in both single and pooled modes.
//!
//! The module is split by domain (projects, sessions, runs, chunks, summaries)
//! so each file stays small, focused and easy to audit independently.

pub mod chunks;
pub mod projects;
pub mod runs;
pub mod sessions;
pub mod summaries;

use anyhow::{Context, Result};

pub use chunks::*;
pub use projects::*;
pub use runs::*;
pub use sessions::*;
pub use summaries::*;

/// Project (buffer) row.
#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: i64,
    pub uuid: Option<String>,
    pub name: String,
    pub path: String,
    pub total_chunks: i64,
    pub total_files: i64,
    pub created_at: i64,
}

/// Session row.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub project: String,
    pub title: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// Run row used by the server handlers.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: String,
    pub project: Option<String>,
    pub task: String,
    pub backend: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub answer: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub nodes_visited: i64,
    pub max_depth: i64,
}

/// Run a store operation on the blocking pool.
///
/// All SQLite access in async contexts should go through this helper so the
/// async runtime is never blocked on pool acquisition or I/O.
///
/// # Errors
///
/// Returns an error if the operation fails or the blocking task panics.
pub async fn blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .context("blocking store task panicked")?
}

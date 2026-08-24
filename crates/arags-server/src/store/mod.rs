//! Typed, pool-safe data access for the gRPC handlers.
//!
//! The server runs the storage pool in `Pooled` mode, where the
//! single-connection helpers on [`arags_storage::Storage`] would panic. Every
//! query here goes through [`arags_storage::Storage::connection`], which works
//! in both single and pooled modes.
//!
//! The module is split by domain (projects, chunks, qa_cache)
//! so each file stays small, focused and easy to audit independently.

pub mod chunks;
pub mod projects;

use anyhow::{Context, Result};

pub use chunks::*;
pub use projects::*;

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

use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::{debug, instrument};

use crate::commands::serve::state::AppState;
use crate::util::data_dir;

/// List all indexed projects.
///
/// # Errors
/// Returns an error if the storage backend cannot be opened or listing buffers
/// fails.
#[instrument(skip_all)]
pub fn handle_status_all(_state: &AppState) -> Result<Value> {
    let start = Instant::now();

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffers = storage.list_buffers().context("failed to list buffers")?;

    let mut items: Vec<Value> = Vec::with_capacity(buffers.len());
    for b in &buffers {
        items.push(serde_json::json!({
            "id": b.id,
            "name": b.name,
            "path": b.path,
            "total_chunks": b.total_chunks,
            "total_files": b.total_files,
            "last_indexed_at": b.last_indexed_at,
        }));
    }

    debug!(elapsed_ms = %start.elapsed().as_millis(), "status all");
    Ok(serde_json::json!({
        "projects": items,
        "count": buffers.len(),
    }))
}

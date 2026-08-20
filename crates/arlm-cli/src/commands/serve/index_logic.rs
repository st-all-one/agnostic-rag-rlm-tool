use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::{debug, instrument};

use crate::commands::serve::requests::IndexRequest;
use crate::commands::serve::state::AppState;
use crate::util::data_dir;

/// Index (or re-index) a project directory.
///
/// # Errors
/// Returns an error if the storage backend cannot be opened, the buffer cannot
/// be created, or indexing the directory fails.
#[instrument(skip_all)]
pub fn handle_index(state: &AppState, req: &IndexRequest) -> Result<Value> {
    let start = Instant::now();

    let index_path = match &req.path {
        Some(p) => PathBuf::from(p)
            .canonicalize()
            .with_context(|| format!("failed to resolve path: {p}"))?,
        None => state.project.clone(),
    };

    let data_dir = data_dir();

    let storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?;

    let _buffer_id = if let Some(buf) = buffer {
        buf.id
    } else {
        storage
            .insert_buffer(&arlm_storage::sqlite::buffers::NewBuffer {
                name: state.project_name.clone(),
                path: index_path.to_string_lossy().to_string(),
            })
            .context("failed to create buffer")?
    };

    let knowledge = arlm_memory::KnowledgeEngine::new(storage);
    let opts = arlm_memory::knowledge::IndexOptions {
        max_chunk_bytes: req.chunk_size * 4,
        ..Default::default()
    };

    let result = knowledge
        .index_directory(&state.project_name, &index_path, &opts)
        .context("failed to index directory")?;

    debug!(elapsed_ms = %start.elapsed().as_millis(), "index completed");
    Ok(serde_json::json!({
        "project": state.project_name,
        "path": index_path.display().to_string(),
        "files_processed": result.files_processed,
        "chunks_created": result.chunks_created,
        "duration_ms": result.duration_ms,
    }))
}

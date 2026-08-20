use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::{debug, instrument};

use crate::commands::serve::requests::{ContextRequest, SearchRequest};
use crate::commands::serve::state::AppState;
use crate::util::data_dir;

/// Build context for a task by running hybrid search and assembling a prompt.
///
/// # Errors
/// Returns an error if the storage backend cannot be opened, the project is not
/// indexed, hybrid search fails, or context assembly fails.
#[instrument(skip_all)]
pub async fn handle_context(state: &AppState, req: &ContextRequest) -> Result<Value> {
    let start = Instant::now();

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let options = arlm_search::SearchOptions {
        tier: arlm_search::SearchTier::Entity,
        top_k: req.top_k,
    };

    let results = hybrid
        .search(&req.task, None, buffer.id, &options, None, Some(&storage))
        .await
        .context("hybrid search failed")?;

    state.metrics.record_search(results.len() as u64);

    // Record agent metrics if agent name provided
    if let Some(ref agent) = req.agent {
        state.metrics.record_agent_request(agent, 0);
    }

    let context =
        arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt, None)
            .context("failed to build context")?;

    debug!(elapsed_ms = %start.elapsed().as_millis(), "context built");
    Ok(serde_json::json!({
        "task": req.task,
        "project": state.project_name,
        "context": context,
        "results_count": results.len(),
    }))
}

/// Search the project and return matching chunks.
///
/// # Errors
/// Returns an error if the storage backend cannot be opened, the project is not
/// indexed, hybrid search fails, or result assembly fails.
#[instrument(skip_all)]
pub async fn handle_search(state: &AppState, req: &SearchRequest) -> Result<Value> {
    let start = Instant::now();

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let options = arlm_search::SearchOptions {
        tier: arlm_search::SearchTier::Entity,
        top_k: req.top_k,
    };

    let results = hybrid
        .search(&req.query, None, buffer.id, &options, None, Some(&storage))
        .await
        .context("hybrid search failed")?;

    let search_results = arlm_search::build_search_results(&storage, &results, None)
        .context("failed to build results")?;

    let items: Vec<Value> = search_results
        .iter()
        .filter(|r| req.min_score.is_none_or(|min| r.score >= min))
        .filter(|r| {
            req.file_pattern
                .as_ref()
                .is_none_or(|pat| r.file_path.contains(pat.as_str()))
        })
        .map(|r| {
            serde_json::json!({
                "chunk_id": r.chunk_id,
                "file": r.file_path,
                "line_start": r.line_start,
                "line_end": r.line_end,
                "score": r.score,
                "content": r.content,
                "language": r.language,
            })
        })
        .collect();

    state.metrics.record_search(items.len() as u64);

    // Record agent metrics if agent name provided
    if let Some(ref agent) = req.agent {
        state.metrics.record_agent_request(agent, 0);
    }

    debug!(elapsed_ms = %start.elapsed().as_millis(), "search completed");
    Ok(serde_json::json!({
        "query": req.query,
        "results": items,
        "count": items.len(),
    }))
}

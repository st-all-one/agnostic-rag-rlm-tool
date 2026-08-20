use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, info, instrument};

use crate::commands::mcp::McpState;
use crate::util::data_dir;

#[instrument(skip_all, fields(tool = "rlm_context", project = %state.project_name))]
pub(crate) fn call_rlm_context(
    state: &McpState,
    args: Option<&serde_json::Value>,
) -> Result<String> {
    let start = Instant::now();
    let default_params = json!({});
    let params = args.unwrap_or(&default_params);

    let task = params
        .get("task")
        .and_then(|v| v.as_str())
        .context("Missing required parameter: task")?;

    let top_k = params
        .get("top_k")
        .and_then(serde_json::Value::as_u64)
        .map_or(10, |v| v as usize);

    debug!(task, top_k, "opening storage for rlm_context");
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let results = hybrid
        .search_fts(task, buffer.id, top_k, None)
        .context("FTS search failed")?;

    let context =
        arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt, None)
            .context("failed to build context")?;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    info!(elapsed_ms, task, top_k, "rlm_context completed");

    Ok(format!(
        "Context for task: {task}\nProject: {}\n\n{context}",
        state.project_name
    ))
}

#[instrument(skip_all, fields(tool = "rlm_search", project = %state.project_name))]
pub(crate) fn call_rlm_search(
    state: &McpState,
    args: Option<&serde_json::Value>,
) -> Result<String> {
    let start = Instant::now();
    let default_params = json!({});
    let params = args.unwrap_or(&default_params);

    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .context("Missing required parameter: query")?;

    let top_k = params
        .get("top_k")
        .and_then(serde_json::Value::as_u64)
        .map_or(10, |v| v as usize);

    let file_pattern = params.get("file_pattern").and_then(|v| v.as_str());

    let min_score = params
        .get("min_score")
        .and_then(serde_json::Value::as_f64)
        .map(|v| v as f32);

    debug!(query, top_k, "opening storage for rlm_search");
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let results = hybrid
        .search_fts(query, buffer.id, top_k, None)
        .context("FTS search failed")?;

    let search_results = arlm_search::build_search_results(&storage, &results, None)
        .context("failed to build results")?;

    let items: Vec<serde_json::Value> = search_results
        .iter()
        .filter(|r| min_score.is_none_or(|min| r.score >= min))
        .filter(|r| {
            #[allow(clippy::unnecessary_map_or)]
            file_pattern
                .as_ref()
                .map_or(true, |pat| r.file_path.contains(&**pat))
        })
        .map(|r| {
            json!({
                "file": r.file_path,
                "line_start": r.line_start,
                "line_end": r.line_end,
                "score": r.score,
                "content": r.content,
                "language": r.language,
            })
        })
        .collect();

    let output = json!({
        "query": query,
        "results": items,
        "count": items.len(),
    });

    let elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        elapsed_ms,
        query,
        count = items.len(),
        "rlm_search completed"
    );

    serde_json::to_string_pretty(&output).context("failed to serialize search results")
}

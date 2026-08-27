//! Chunk-context building for `BuildContext`, plus time-travel chunk hydration
//! and markdown rendering.

use std::collections::HashMap;
use std::fmt::Write as _;

use arags_search::SearchResult;
use tonic::{Response, Status};

use crate::grpc::error::{internal, invalid_arg};
use crate::grpc::search::hybrid::{buffer_id_for, hybrid_search};
use crate::grpc::util::sanitize_fts;
use crate::grpc::util::to_proto_results;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{
    ContextRequest, ContextResponse, ContextStats, SearchResult as ProtoResult,
};

/// Time-travel (plan 021): rewrite each chunk candidate to the revision active
/// at `as_of_epoch`. A candidate whose live revision did not yet exist at T is
/// dropped; a candidate superseded before T is replaced by the text of its
/// prior (pre-supersede) revision. Scores are preserved from the original hit.
pub(crate) async fn apply_chunk_as_of(
    state: &AppState,
    as_of_epoch: i64,
    candidates: Vec<SearchResult>,
) -> anyhow::Result<Vec<SearchResult>> {
    if as_of_epoch <= 0 {
        return Ok(candidates);
    }
    let storage = state.storage.clone();
    let ids: Vec<i64> = candidates.iter().map(|c| c.chunk_id).collect();
    let scores: HashMap<i64, f32> = candidates.iter().map(|c| (c.chunk_id, c.score)).collect();
    store::blocking(move || {
        let mut out: Vec<SearchResult> = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(ch) = storage.get_chunk_as_of(id, as_of_epoch)? {
                let content = storage.get_chunk_content(ch.id)?.unwrap_or_default();
                out.push(SearchResult {
                    chunk_id: ch.id,
                    score: scores.get(&id).copied().unwrap_or(0.0),
                    file_path: ch.file_path,
                    line_start: ch.line_start,
                    line_end: ch.line_end,
                    content,
                    language: ch.language,
                    epoch: ch.epoch,
                    created_by: ch.created_by.clone(),
                    model: ch.model.clone(),
                    version: ch.version,
                });
            }
        }
        Ok(out)
    })
    .await
}

/// Render hydrated chunks into the markdown-style LLM context with a token
/// budget. Returns the body and the number of tokens consumed.
fn render_context(candidates: &[ProtoResult], max_tokens: u32) -> (String, u32) {
    let mut body = String::from("# Project Context\n\n");
    let mut budget: u32 = 0;
    for r in candidates {
        let tokens = (r.text.len() as u32).saturating_div(4);
        if tokens > 0 && budget + tokens > max_tokens {
            continue;
        }
        budget += tokens;
        let _ = write!(
            body,
            "## {} (score {:.2})\n```\n{}\n```\n\n",
            r.file_path, r.score, r.text
        );
    }
    (body, budget)
}

/// Build an LLM-ready context from the top relevant chunks of a project.
///
/// # Errors
///
/// Returns an error if storage access fails or the project is unknown.
pub(crate) async fn handle_build_context(
    state: &AppState,
    req: ContextRequest,
) -> Result<Response<ContextResponse>, Status> {
    let start = std::time::Instant::now();
    let project = req.project;
    let task = req.task;

    if task.trim().is_empty() {
        return Err(invalid_arg("task is required"));
    }

    let buffer_id = buffer_id_for(state, &project)
        .await?
        .ok_or_else(|| crate::grpc::error::not_found("project not found"))?;

    // Serving defaults from `server.toml [search]` (plan 020): an omitted
    // budget falls back to the configured `max_tokens`.
    let max_tokens: u32 = if req.max_tokens == 0 {
        state.config.search.max_tokens
    } else {
        req.max_tokens as u32
    };

    let fts_query = sanitize_fts(&task);
    // Context uses the full hybrid tier (BM25 + entity + semantic) so the
    // token budget keeps the strongest matches across both signals.
    let candidates = hybrid_search(
        state,
        buffer_id,
        &fts_query,
        arags_search::SearchTier::Vector,
        50,
    )
    .await
    .map_err(internal)?;

    let results = to_proto_results(&candidates);
    let (context, total_tokens) = render_context(&results, max_tokens);

    tracing::info!(
        project = %project,
        chunks = results.len(),
        total_tokens,
        elapsed_ms = start.elapsed().as_millis(),
        "build_context completed"
    );

    let raw_chunks = results.len() as i32;
    Ok(Response::new(ContextResponse {
        context,
        sources: results,
        stats: Some(ContextStats {
            total_tokens: total_tokens as i32,
            raw_chunks_included: raw_chunks,
            summary_chunks_included: 0,
            summary_ratio: 0.0,
        }),
    }))
}

//! Search and context-building RPCs: `Search`, `BuildContext`.
//!
//! Both run a unified hybrid search (`arlm_search::HybridSearch`) over the
//! project's chunks: BM25 (FTS5) is always the base tier, and the `entity`,
//! `vector` (semantic) tiers are fused on top via Reciprocal
//! Rank Fusion (RRF). The semantic tier is powered by the server's embedder
//! (native all-MiniLM-L6-v2; a hash fallback without weights), so vector
//! search degrades gracefully to BM25 when no vector store is configured.
//!
//! Result scores are min-max normalised to `[0, 1]` (higher = better) so that
//! `--min-score` thresholds and client ranking stay meaningful regardless of
//! which tiers contributed. Natural-language questions that return nothing
//! under FTS5's default AND semantics are retried with an OR pass.

use std::fmt::Write as _;
use std::time::Instant;

use arlm_proto::proto::*;
use arlm_search::{
    Bm25Search, EntitySearch, HybridSearch, SearchOptions, SearchTier as HybridTier,
    SemanticSearch, build_search_results,
};
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg, not_found};
use crate::state::AppState;
use crate::store;

/// Map a project reference (UUID or name) to its numeric buffer id.
pub(crate) async fn buffer_id_for(state: &AppState, project: &str) -> Result<Option<i64>, Status> {
    let project_owned = project.to_string();
    let storage = state.storage.clone();
    store::blocking(move || store::buffer_id_for_project(&storage, &project_owned))
        .await
        .map_err(internal)
}

/// Sanitise a user query for FTS5 `MATCH`: keep only alphanumeric and
/// whitespace, collapsing everything else to a space.
fn sanitize_fts(query: &str) -> String {
    query
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect()
}

/// Run the unified hybrid search and hydrate results into full chunks.
///
/// Always runs BM25; adds the `entity`/`vector` tiers according to `tier`.
/// When the query is a multi-word natural-language question that returns
/// nothing, a second OR-based BM25 pass recovers relevant chunks.
pub(crate) async fn hybrid_search(
    state: &AppState,
    buffer_id: i64,
    fts_query: &str,
    tier: HybridTier,
    top_k: usize,
) -> anyhow::Result<Vec<arlm_search::SearchResult>> {
    let storage = state.storage.clone();
    let bm25 = Bm25Search::new(&storage).map_err(|e| anyhow::anyhow!("bm25 init: {e}"))?;
    let entity = EntitySearch::new(storage.clone()).ok();
    let semantic = state
        .vector_store
        .as_ref()
        .map(|v| SemanticSearch::new(v.clone()));
    let hybrid = HybridSearch::new(bm25, entity, semantic);

    // Embedding inference is synchronous CPU work and would block the async
    // worker, so run it on a blocking task. Falls back to BM25-only when the
    // embed fails.
    let fts_query_owned = fts_query.to_string();
    let embedder = state.embedder.clone();
    let query_vector = tokio::task::spawn_blocking(move || embedder.embed(&fts_query_owned))
        .await
        .ok()
        .and_then(Result::ok);
    let query_vector = query_vector.as_deref();
    let options = SearchOptions {
        tier,
        top_k: top_k * 3,
    };

    let mut fused = hybrid
        .search(fts_query, query_vector, buffer_id, &options, None)
        .await?;

    // Natural-language fix: AND yields nothing on multi-word queries, retry
    // with OR so questions still surface relevant chunks.
    if fused.is_empty() && fts_query.split_whitespace().count() > 1 {
        let or_query = fts_query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" OR ");
        fused = hybrid
            .search(&or_query, query_vector, buffer_id, &options, None)
            .await?;
    }

    let mut results = build_search_results(&state.storage, &fused, None)?;
    normalize_scores(&mut results);
    Ok(results)
}

/// Min-max normalise RRF fusion scores to `[0, 1]` (higher = better) so that
/// `--min-score` thresholds and downstream ranking remain meaningful.
fn normalize_scores(results: &mut [arlm_search::SearchResult]) {
    if results.is_empty() {
        return;
    }
    let min = results
        .iter()
        .map(|r| r.score)
        .fold(f32::INFINITY, f32::min);
    let max = results
        .iter()
        .map(|r| r.score)
        .fold(f32::NEG_INFINITY, f32::max);

    if (max - min).abs() < f32::EPSILON {
        for r in results.iter_mut() {
            r.score = 1.0;
        }
        return;
    }
    for r in results.iter_mut() {
        r.score = (r.score - min) / (max - min);
    }
}

/// Map hydrated `arlm_search` results into the gRPC `SearchResult` shape.
fn to_proto_results(results: &[arlm_search::SearchResult]) -> Vec<SearchResult> {
    results
        .iter()
        .map(|r| SearchResult {
            chunk_id: r.chunk_id,
            text: r.content.clone(),
            score: r.score,
            file_path: r.file_path.clone(),
            start_line: r.line_start as i32,
            end_line: r.line_end as i32,
        })
        .collect()
}

/// Render hydrated chunks into the markdown-style LLM context with a token
/// budget. Returns the body and the number of tokens consumed.
fn render_context(candidates: &[SearchResult], max_tokens: u32) -> (String, u32) {
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

/// Search chunks in a project with BM25 (+ optional semantic) ranking.
///
/// # Errors
///
/// Returns an error if storage access fails or the query is invalid.
pub(crate) async fn handle_search(
    state: &AppState,
    request: Request<SearchRequest>,
) -> Result<Response<SearchResponse>, Status> {
    let start = Instant::now();
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();
    let project = req.project.clone();
    let query = req.query.clone();
    crate::grpc::memory::record_query_history(state, &ctx, &project, "search", &query).await;

    if query.trim().is_empty() {
        return Err(invalid_arg("search query is required"));
    }

    let buffer_id = buffer_id_for(state, &project)
        .await?
        .ok_or_else(|| not_found("project not found"))?;

    // Serving defaults from `server.toml [search]` (plan 020): an omitted
    // limit falls back to the configured `top_k`.
    let max_results = if req.max_results > 0 {
        req.max_results as usize
    } else {
        state.config.search.top_k
    };

    // Tier resolution (plan 020): `UNSPECIFIED`/unknown values resolve to the
    // `[search].tier` serving default from `server.toml`; explicit values are
    // honored as sent.
    let tier = match SearchTier::try_from(req.tier) {
        Ok(SearchTier::TierBm25) => HybridTier::Fts,
        Ok(SearchTier::TierEntity) => HybridTier::Entity,
        Ok(SearchTier::TierSemantic) => HybridTier::Vector,
        Ok(SearchTier::TierHybrid) => HybridTier::Vector,
        _ => match state.config.search.tier.to_ascii_lowercase().as_str() {
            "fts" | "bm25" => HybridTier::Fts,
            "entity" => HybridTier::Entity,
            _ => HybridTier::Vector,
        },
    };

    let fts_query = sanitize_fts(&query);
    let candidates = hybrid_search(state, buffer_id, &fts_query, tier, max_results)
        .await
        .map_err(internal)?;

    let results = to_proto_results(&candidates);
    let total_count = i32::try_from(results.len()).unwrap_or(i32::MAX);
    Ok(Response::new(SearchResponse {
        results,
        total_count,
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
    }))
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
    let start = Instant::now();
    let project = req.project;
    let task = req.task;

    if task.trim().is_empty() {
        return Err(invalid_arg("task is required"));
    }

    let buffer_id = buffer_id_for(state, &project)
        .await?
        .ok_or_else(|| not_found("project not found"))?;

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
    let candidates = hybrid_search(state, buffer_id, &fts_query, HybridTier::Vector, 50)
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

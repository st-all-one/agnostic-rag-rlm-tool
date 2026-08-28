//! `Search` RPC handler plus the unified query pipeline that fuses RLM summaries
//! and exploration maps into the chunk answer.

use std::time::Instant;
use tracing::{debug, warn};

use arags_search::SearchTier as HybridTier;
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg, not_found};
use crate::grpc::search::context::apply_chunk_as_of;
use crate::grpc::search::hybrid::{buffer_id_for, hybrid_search};
use crate::grpc::search::summary::{SummaryCandidate, summary_search};
use crate::grpc::util::{sanitize_fts, to_proto_results};
use crate::state::AppState;

use arags_proto::proto::{SearchRequest, SearchResponse, SearchResult, SearchTier, SummaryHit};

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
    // honored as sent. TIER_SUMMARY bypasses the chunk pipeline entirely and
    // searches only the approved RLM summary dataset.
    if matches!(SearchTier::try_from(req.tier), Ok(SearchTier::TierSummary)) {
        let fts_query = sanitize_fts(&query);
        let summaries = summary_search(
            state,
            buffer_id,
            &project,
            &fts_query,
            max_results,
            req.as_of_epoch,
        )
        .await
        .map_err(internal)?;
        let results = summaries_to_results(&summaries);
        let total_count = i32::try_from(results.len()).unwrap_or(i32::MAX);
        return Ok(Response::new(SearchResponse {
            results,
            total_count,
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
            summaries: summaries.iter().map(summary_hit_from).collect(),
            explorations: Vec::new(),
        }));
    }

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

    // Time-travel (plan 021): narrow chunk candidates to the revision active at
    // `as_of_epoch`. A chunk created after T is dropped; a chunk superseded
    // before T is replaced by the text of its prior (pre-supersede) revision.
    let candidates = if req.as_of_epoch > 0 {
        apply_chunk_as_of(state, req.as_of_epoch, candidates)
            .await
            .map_err(internal)?
    } else {
        candidates
    };

    // Unified query (plan 023): fuse approved RLM summaries into the answer
    // budget and attach relevant exploration maps. Both are additive fields —
    // clients unaware of them simply ignore the new data.
    let (results, summaries, explorations) = unify_query(
        state,
        &project,
        buffer_id,
        &query,
        &fts_query,
        to_proto_results(&candidates),
        max_results,
    )
    .await;

    let total_count = i32::try_from(results.len()).unwrap_or(i32::MAX);
    Ok(Response::new(SearchResponse {
        results,
        total_count,
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
        summaries,
        explorations,
    }))
}

/// Convert summary candidates into legacy-shaped `SearchResult`s, keeping
/// TIER_SUMMARY responses backward compatible (`file_path` tags the subject).
fn summaries_to_results(summaries: &[SummaryCandidate]) -> Vec<SearchResult> {
    summaries
        .iter()
        .map(|s| SearchResult {
            chunk_id: s.rowid,
            text: s.text.clone(),
            score: s.score,
            file_path: format!(
                "[summary:{level}] {subject}",
                level = s.level,
                subject = s.subject
            ),
            start_line: 0,
            end_line: 0,
            epoch: s.epoch,
            created_by: s.created_by.clone().unwrap_or_default(),
            model: s.model.clone().unwrap_or_default(),
            version: s.version,
        })
        .collect()
}

fn summary_hit_from(s: &SummaryCandidate) -> SummaryHit {
    SummaryHit {
        node_id: s.node_id.clone(),
        rowid: s.rowid,
        level: i32::try_from(s.level).unwrap_or(0),
        subject: s.subject.clone(),
        summary_text: s.text.clone(),
        score: s.score,
        epoch: s.epoch,
        created_by: s.created_by.clone().unwrap_or_default(),
        model: s.model.clone().unwrap_or_default(),
        version: s.version,
    }
}

/// Unified query pipeline (plan 023):
/// 1. Chunks always keep at least `(1 - summary_ratio)` of the result budget.
/// 2. When approved RLM summaries qualify (score >= `summary_min_score`),
///    they claim up to `summary_ratio` of the budget — the digest-once
///    workflow means a query about digested content is answered mostly by
///    synthesised summaries with real code backing the remainder. With no
///    qualifying summaries the full budget stays with chunks.
/// 3. Relevant fresh exploration maps are attached when available.
///
/// Every stage is best-effort: failures degrade to chunk-only responses.
async fn unify_query(
    state: &AppState,
    project: &str,
    buffer_id: i64,
    raw_query: &str,
    fts_query: &str,
    mut candidates: Vec<SearchResult>,
    max_results: usize,
) -> (
    Vec<SearchResult>,
    Vec<SummaryHit>,
    Vec<arags_proto::proto::ExplorationRef>,
) {
    let cfg = &state.config.search;

    // ── Summaries (space C) ─────────────────────────────────────────────
    let mut summary_hits: Vec<SummaryHit> = Vec::new();
    let ratio = cfg.summary_ratio.clamp(0.0, 1.0);
    if ratio > 0.0 && max_results > 1 {
        match summary_search(state, buffer_id, project, fts_query, max_results, 0).await {
            Ok(all) => {
                let qualifying: Vec<SummaryCandidate> = all
                    .into_iter()
                    .filter(|s| s.score >= cfg.summary_min_score as f32)
                    .collect();
                let (take, chunk_budget) =
                    split_summary_budget(max_results, ratio, qualifying.len());
                if take > 0 {
                    summary_hits = qualifying[..take].iter().map(summary_hit_from).collect();
                    candidates.truncate(chunk_budget);
                }
            }
            Err(e) => warn!(error = %e, "unified query: summary fusion failed"),
        }
    }

    // ── Explorations (space D) ──────────────────────────────────────────
    let mut exploration_refs: Vec<arags_proto::proto::ExplorationRef> = Vec::new();
    if cfg.exploration_enabled && cfg.exploration_limit > 0 {
        match super::super::exploration::search::search_explorations_core(
            state,
            project.to_string(),
            raw_query.to_string(),
            cfg.exploration_limit,
            false,
            0,
        )
        .await
        {
            Ok(resp) => {
                exploration_refs = resp
                    .hits
                    .into_iter()
                    .map(|h| arags_proto::proto::ExplorationRef {
                        exploration_id: h.exploration_id,
                        goal: h.goal,
                        summary: h.summary,
                        confidence: h.confidence,
                    })
                    .collect();
            }
            Err(status) => {
                debug!(%status, "unified query: exploration attach skipped");
            }
        }
    }

    (candidates, summary_hits, exploration_refs)
}

/// Split the result budget between RLM summaries and chunks.
///
/// Summaries claim `floor(max_results * ratio)` slots, capped by how many
/// actually qualify; chunks keep the remainder (always at least 1 when
/// `max_results > 1`, so real code never disappears entirely).
#[must_use]
pub(crate) fn split_summary_budget(
    max_results: usize,
    ratio: f64,
    qualifying: usize,
) -> (usize, usize) {
    if max_results <= 1 || ratio <= 0.0 {
        return (0, max_results);
    }
    let want = ((max_results as f64) * ratio).floor() as usize;
    let mut take = want.min(qualifying);
    // Keep at least one chunk slot for grounded, verbatim context.
    if take >= max_results {
        take = max_results - 1;
    }
    if take == 0 {
        return (0, max_results);
    }
    let chunk_budget = max_results - take;
    (take, chunk_budget)
}

//! Search and context-building RPCs: `Search`, `BuildContext`.
//!
//! Both run a unified hybrid search (`arags_search::HybridSearch`) over the
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

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::Instant;

use arags_search::hybrid::rrf::rrf_score;
use arags_search::{
    Bm25Search, EntitySearch, HybridSearch, SearchOptions, SearchTier as HybridTier,
    SemanticSearch, build_search_results,
};
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg, not_found};
use crate::grpc::util::{sanitize_fts, to_proto_results};
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{
    ContextRequest, ContextResponse, ContextStats, SearchRequest, SearchResponse, SearchResult,
    SearchTier,
};

/// Map a project reference (UUID or name) to its numeric buffer id.
pub(crate) async fn buffer_id_for(state: &AppState, project: &str) -> Result<Option<i64>, Status> {
    let project_owned = project.to_string();
    let storage = state.storage.clone();
    store::blocking(move || store::buffer_id_for_project(&storage, &project_owned))
        .await
        .map_err(internal)
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
) -> anyhow::Result<Vec<arags_search::SearchResult>> {
    let storage = state.storage.clone();
    let bm25 = Bm25Search::new(&storage).map_err(|e| anyhow::anyhow!("bm25 init: {e}"))?;
    let entity = EntitySearch::new(storage.clone()).ok();
    let semantic = state
        .vector_store
        .as_ref()
        .map(|v| SemanticSearch::new(v.clone()));
    // Serving-path decay ([search].decay_lambda > 0) re-weights fused scores
    // by chunk age; 0 keeps the default disabled-at-query-time behaviour.
    let hybrid = {
        let h = HybridSearch::new(bm25, entity, semantic);
        let lambda = state.config.search.decay_lambda;
        if lambda > 0.0 {
            h.with_decay(arags_search::DecayConfig::new(lambda))
        } else {
            h.with_decay(arags_search::DecayConfig::disabled())
        }
    };

    // Embedding inference is synchronous CPU work and would block the async
    // worker, so run it on a blocking task. Falls back to BM25-only when the
    // embed fails. The query embed runs on the **global** rayon pool (not the
    // capped index pool), so a concurrent `arags index` cannot starve it (issue
    // `agnostic-rlm-rs-6690`). We surface contention for observability only.
    if state.index_embed_in_flight() > 0 {
        tracing::debug!(
            active_index_embeds = state.index_embed_in_flight(),
            "query embed shares cores with an active index embed; served on global pool"
        );
    }
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

    // Serving-path salience decay ([search].decay_lambda > 0): re-weight the
    // fused scores by chunk age and re-rank. Best-effort — an age lookup
    // failure must not kill the search.
    if state.config.search.decay_lambda > 0.0 && !fused.is_empty() {
        let ids: Vec<i64> = fused.iter().map(|r| r.chunk_id).collect();
        let storage = state.storage.clone();
        match store::blocking(move || storage.chunk_ages_hours(&ids)).await {
            Ok(ages) => fused = hybrid.apply_decay(fused, &ages),
            Err(e) => tracing::warn!(error = %e, "chunk age lookup failed; skipping decay"),
        }
    }

    let mut results = build_search_results(&state.storage, &fused, None)?;
    normalize_scores(&mut results);
    Ok(results)
}

/// Min-max normalise RRF fusion scores to `[0, 1]` (higher = better) so that
/// `--min-score` thresholds and downstream ranking remain meaningful.
fn normalize_scores(results: &mut [arags_search::SearchResult]) {
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

/// Time-travel metadata captured for an RLM summary revision: `(summary_text,
/// subject, epoch, created_by, model, version)`.
type RlmAsOf = (String, String, i64, Option<String>, Option<String>, i64);

/// A hydrated RLM summary candidate with its fused score.
#[derive(Debug, Clone)]
pub(crate) struct SummaryCandidate {
    pub node_id: String,
    pub rowid: i64,
    pub level: i64,
    pub subject: String,
    pub text: String,
    pub score: f32,
    pub epoch: i64,
    pub created_by: Option<String>,
    pub model: Option<String>,
    pub version: i64,
}

/// Search the RLM recursive summary dataset. Lexical candidates come from
/// `rlm_fts`; semantic candidates from the dedicated summary vector space.
/// The two rankings are fused with Reciprocal Rank Fusion (same family as the
/// chunk hybrid tiers) and min-max normalised to `[0, 1]`.
pub(crate) async fn summary_search(
    state: &AppState,
    buffer_id: i64,
    project: &str,
    fts_query: &str,
    top_k: usize,
    as_of_epoch: i64,
) -> anyhow::Result<Vec<SummaryCandidate>> {
    const RRF_K: f32 = 60.0;
    let start = std::time::Instant::now();
    let storage = state.storage.clone();

    // Lexical pass (blocking SQLite work): ranked list of rowids.
    let query_owned = fts_query.to_string();
    let lexical: Vec<(i64, arags_storage::sqlite::rlm::RlmNode)> =
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut nodes = storage.search_rlm_fts(buffer_id, &query_owned, top_k)?;
            if nodes.is_empty() && query_owned.split_whitespace().count() > 1 {
                // Natural-language fix: AND yields nothing, retry with OR.
                let or_query = query_owned
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" OR ");
                nodes = storage.search_rlm_fts(buffer_id, &or_query, top_k)?;
            }
            Ok(nodes.into_iter().map(|n| (n.id, n)).collect())
        })
        .await
        .map_err(internal)??;

    // Semantic pass over the dedicated summary vector space: ranked list
    // ordered by cosine similarity (approved + scoped hydration only).
    let mut semantic: Vec<(i64, arags_storage::sqlite::rlm::RlmNode)> = Vec::new();
    if let Some(vectors) = state.rlm_vector_store.as_ref() {
        let embedder = state.embedder.clone();
        let q = fts_query.to_string();
        // Query embed runs on the global rayon pool; the capped index pool keeps
        // it from being starved during a concurrent `arags index` (issue
        // `agnostic-rlm-rs-6690`).
        if state.index_embed_in_flight() > 0 {
            tracing::debug!(
                active_index_embeds = state.index_embed_in_flight(),
                "rlm summary query embed contends with active index embed; served on global pool"
            );
        }
        let query_vector = tokio::task::spawn_blocking(move || embedder.embed(&q))
            .await
            .ok()
            .and_then(Result::ok);
        if let Some(vec) = query_vector {
            let mut neighbors = vectors
                .search(&vec, top_k)
                .map_err(|e| anyhow::anyhow!(e))?;
            neighbors.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
            let ids: Vec<u64> = neighbors.iter().map(|n| n.id).collect();
            let storage = state.storage.clone();
            let approved = store::blocking(move || storage.get_approved_rlm_nodes(&ids, buffer_id))
                .await
                .map_err(internal)?;
            let by_id: std::collections::HashMap<i64, _> =
                approved.into_iter().map(|n| (n.id, n)).collect();
            for nb in neighbors {
                #[allow(clippy::cast_possible_wrap)] // rowids fit i64
                if let Ok(rowid) = i64::try_from(nb.id) {
                    if let Some(node) = by_id.get(&rowid) {
                        semantic.push((rowid, node.clone()));
                    }
                }
            }
        }
    }

    // RRF fusion over the two rankings; hydrate from whichever list carries
    // the node (lexical rows are complete; semantic rows were hydrated too).
    let mut scores: HashMap<i64, f32> = HashMap::with_capacity(top_k * 2);
    for (rank, (rowid, _)) in lexical.iter().enumerate() {
        *scores.entry(*rowid).or_insert(0.0) += rrf_score(rank, RRF_K);
    }
    for (rank, (rowid, _)) in semantic.iter().enumerate() {
        *scores.entry(*rowid).or_insert(0.0) += rrf_score(rank, RRF_K);
    }
    let node_by_id: HashMap<i64, &arags_storage::sqlite::rlm::RlmNode> = lexical
        .iter()
        .chain(semantic.iter())
        .map(|(id, n)| (*id, n))
        .collect();

    let mut fused: Vec<SummaryCandidate> = scores
        .into_iter()
        .filter_map(|(rowid, score)| {
            node_by_id.get(&rowid).map(|n| SummaryCandidate {
                node_id: n.node_id.clone(),
                rowid,
                level: n.level,
                subject: n.subject.clone(),
                text: n.summary_text.clone(),
                score,
                epoch: n.epoch,
                created_by: n.created_by.clone(),
                model: n.model.clone(),
                version: n.version,
            })
        })
        .collect();

    // Time-travel (plan 021): for each summary candidate, serve the revision of
    // its `(project, level, subject)` node that was active at `as_of_epoch`.
    if as_of_epoch > 0 {
        let storage = state.storage.clone();
        let project_owned = project.to_string();
        let keys: Vec<(i64, i64, String)> = fused
            .iter()
            .map(|s| (s.rowid, s.level, s.subject.clone()))
            .collect();
        match store::blocking(move || {
            let mut revised: std::collections::HashMap<i64, RlmAsOf> =
                std::collections::HashMap::with_capacity(keys.len());
            for (rowid, level, subject) in keys {
                if let Some(node) =
                    storage.get_rlm_node_as_of(&project_owned, level, &subject, as_of_epoch)?
                {
                    revised.insert(
                        rowid,
                        (
                            node.summary_text.clone(),
                            node.subject.clone(),
                            node.epoch,
                            node.created_by.clone(),
                            node.model.clone(),
                            node.version,
                        ),
                    );
                }
            }
            Ok::<_, anyhow::Error>(revised)
        })
        .await
        {
            Ok(revised) => {
                for s in &mut fused {
                    if let Some((text, subject, epoch, created_by, model, version)) =
                        revised.get(&s.rowid)
                    {
                        s.text = text.clone();
                        s.subject = subject.clone();
                        s.epoch = *epoch;
                        s.created_by = created_by.clone();
                        s.model = model.clone();
                        s.version = *version;
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "summary as_of rewrite failed; serving live"),
        }
    }

    // Deterministic order: score desc, then rowid asc (RRF ties).
    fused.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.rowid.cmp(&b.rowid))
    });
    normalize_summaries(&mut fused);
    fused.truncate(top_k);
    tracing::debug!(
        elapsed_ms = start.elapsed().as_millis(),
        lexical = lexical.len(),
        semantic = semantic.len(),
        fused = fused.len(),
        "summary_search completed"
    );
    Ok(fused)
}

/// Min-max normalise RRF-fused summary scores to `[0, 1]`.
fn normalize_summaries(summaries: &mut [SummaryCandidate]) {
    if summaries.len() < 2 {
        for s in summaries.iter_mut() {
            s.score = 1.0;
        }
        return;
    }
    let min = summaries
        .iter()
        .map(|s| s.score)
        .fold(f32::INFINITY, f32::min);
    let max = summaries
        .iter()
        .map(|s| s.score)
        .fold(f32::NEG_INFINITY, f32::max);
    if (max - min).abs() < f32::EPSILON {
        for s in summaries.iter_mut() {
            s.score = 1.0;
        }
        return;
    }
    for s in summaries.iter_mut() {
        s.score = (s.score - min) / (max - min);
    }
}

/// Time-travel (plan 021): rewrite each chunk candidate to the revision active
/// at `as_of_epoch`. A candidate whose live revision did not yet exist at T is
/// dropped; a candidate superseded before T is replaced by the text of its
/// prior (pre-supersede) revision. Scores are preserved from the original hit.
pub(crate) async fn apply_chunk_as_of(
    state: &AppState,
    as_of_epoch: i64,
    candidates: Vec<arags_search::SearchResult>,
) -> anyhow::Result<Vec<arags_search::SearchResult>> {
    if as_of_epoch <= 0 {
        return Ok(candidates);
    }
    let storage = state.storage.clone();
    let ids: Vec<i64> = candidates.iter().map(|c| c.chunk_id).collect();
    let scores: std::collections::HashMap<i64, f32> =
        candidates.iter().map(|c| (c.chunk_id, c.score)).collect();
    store::blocking(move || {
        let mut out: Vec<arags_search::SearchResult> = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(ch) = storage.get_chunk_as_of(id, as_of_epoch)? {
                let content = storage.get_chunk_content(ch.id)?.unwrap_or_default();
                out.push(arags_search::SearchResult {
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

fn summary_hit_from(s: &SummaryCandidate) -> arags_proto::proto::SummaryHit {
    arags_proto::proto::SummaryHit {
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
    Vec<arags_proto::proto::SummaryHit>,
    Vec<arags_proto::proto::ExplorationRef>,
) {
    let cfg = &state.config.search;

    // ── Summaries (space C) ─────────────────────────────────────────────
    let mut summary_hits: Vec<arags_proto::proto::SummaryHit> = Vec::new();
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
            Err(e) => tracing::warn!(error = %e, "unified query: summary fusion failed"),
        }
    }

    // ── Explorations (space D) ──────────────────────────────────────────
    let mut exploration_refs: Vec<arags_proto::proto::ExplorationRef> = Vec::new();
    if cfg.exploration_enabled && cfg.exploration_limit > 0 {
        match super::exploration::search::search_explorations_core(
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
                tracing::debug!(%status, "unified query: exploration attach skipped");
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
fn split_summary_budget(max_results: usize, ratio: f64, qualifying: usize) -> (usize, usize) {
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

#[cfg(test)]
mod tests;

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

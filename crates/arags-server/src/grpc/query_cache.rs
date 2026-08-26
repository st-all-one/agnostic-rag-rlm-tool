//! Query-Answer Cache RPCs (plan 017) + admin-gated `InvalidateCache`.
//!
//! The server is **deterministic**: it embeds, searches (BM25 + vector) and
//! stores — it never runs an LLM. Digestion happens client-side (plan 017
//! "digest-once"). `InvalidateCache` is admin-gated by plan 018: non-admins
//! receive `PERMISSION_DENIED`.
//!
//! `InvalidateCache` is backward-compatible with plan 018: when `cache_id` is
//! empty it purges the legacy `result_cache` by project (empty = all). When
//! `cache_id` is set it invalidates the semantic `qa_cache` (Stale/Delete, with
//! an optional similarity radius over `question_vectors`).

use arags_core::qa_cache::{QaThresholds, resolve_plan};
use arags_search::qa_cache::jaccard_similarity;
use arags_storage::qa_cache as qa_store;
use arags_storage::qa_cache::StoreAnswerInput;
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg};
use crate::grpc::search::{buffer_id_for, hybrid_search};
use crate::grpc::util::{sanitize_fts, to_proto_results};
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{
    GetAnswerByIdRequest, GetAnswerByIdResponse, InvalidateCacheRequest, InvalidateCacheResponse,
    InvalidateMode, QueryWithCacheRequest, QueryWithCacheResponse, SearchResult,
    StoreAnswerRequest, StoreAnswerResponse,
};

/// Task prefix applied to question embeddings (separate vector space B).
const QUESTION_PREFIX: &str = "search_query: ";

/// Candidate pool fetched from the question-vector store for the near-hit
/// similarity check.
const NEAR_HIT_CANDIDATES: usize = 10;

/// Embed a question in the dedicated question space (blocking).
async fn embed_query(state: &AppState, question: &str) -> Option<Vec<f32>> {
    let embedder = state.embedder.clone();
    let text = format!("{QUESTION_PREFIX}{question}");
    tokio::task::spawn_blocking(move || embedder.embed(&text))
        .await
        .ok()
        .and_then(Result::ok)
}

/// Build the adaptive thresholds from server config.
fn thresholds(state: &AppState) -> QaThresholds {
    let c = &state.qa_config;
    QaThresholds {
        novel_k: c.novel_k,
        provenance_k: c.provenance_k,
        sim_high: c.sim_high,
        sim_floor: c.sim_floor,
        tier_steps: c.tier_steps.clone(),
        jaccard_min: c.jaccard_min,
    }
}

/// Resolve a project to its buffer id (explicit or by name).
async fn resolve_buffer(state: &AppState, project: &str, explicit: i64) -> Option<i64> {
    if explicit > 0 {
        return Some(explicit);
    }
    buffer_id_for(state, project).await.ok().flatten()
}

/// Verify that the cached answer's provenance chunks still carry the same
/// content hashes (trust pipeline borrowed from explorations, plan 023).
///
/// Entries without provenance (`source_chunk_ids` empty) are unverifiable and
/// pass through. On drift the entry is marked stale so later queries miss fast.
async fn provenance_intact(state: &AppState, row: &qa_store::QaCacheRow) -> bool {
    if row.source_chunk_ids.is_empty() || row.source_hashes.is_empty() {
        return true;
    }
    let pairs: Vec<(i64, String)> = row
        .source_chunk_ids
        .iter()
        .zip(row.source_hashes.iter())
        .filter_map(|(id, hash)| id.parse::<i64>().ok().map(|i| (i, hash.clone())))
        .collect();
    if pairs.is_empty() {
        return true;
    }
    let storage = state.storage.clone();
    match store::blocking(move || storage.chunk_hashes_match(&pairs)).await {
        Ok(true) => true,
        Ok(false) => {
            let id = row.id;
            let stale_storage = state.storage.clone();
            let _ = store::blocking(move || {
                stale_storage.mark_qa_stale(id, "system", "provenance drift")
            })
            .await;
            tracing::info!(cache_id = %row.cache_id, "qa provenance drifted; entry marked stale");
            false
        }
        Err(e) => {
            // Fail open: verification trouble must not break serving.
            tracing::warn!(error = %e, "qa provenance check failed; serving anyway");
            true
        }
    }
}

/// Query the semantic cache; decides hit/miss/tier deterministically.
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` without a session, or `internal` on storage failure.
pub async fn handle_query_with_cache(
    state: &AppState,
    request: Request<QueryWithCacheRequest>,
) -> Result<Response<QueryWithCacheResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.query_with_cache");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();
    crate::grpc::memory::record_query_history(state, &ctx, &req.project, "query", &req.question)
        .await;

    let project = req.project;
    if project.trim().is_empty() {
        return Err(invalid_arg("project is required"));
    }
    if req.question.trim().is_empty() {
        return Err(invalid_arg("question is required"));
    }

    let buffer_id = resolve_buffer(state, &project, req.buffer_id).await;
    let qh = qa_store::question_hash(&req.question);
    let th = thresholds(state);

    // 1) Exact hit (same question, same project).
    let qh_owned = qh.clone();
    let project_owned = project.clone();
    if let Some(row) = store::blocking({
        let storage = state.storage.clone();
        move || storage.get_cached_answer(&project_owned, &qh_owned)
    })
    .await
    .map_err(internal)?
    {
        let id = row.id;
        let cache_id = row.cache_id.clone();
        // Trust pipeline: verify provenance before serving (drift → stale,
        // fall through to near-hit/miss below).
        if provenance_intact(state, &row).await {
            // Touch for eviction weighting.
            let storage = state.storage.clone();
            let _ = store::blocking(move || storage.touch_qa(id)).await;
            let provenance =
                provenance_chunks(&state, &row.source_chunk_ids, th.provenance_k).await;
            return Ok(Response::new(QueryWithCacheResponse {
                cache_id,
                hit: true,
                tier: 0,
                similarity: 1.0,
                answer_text: row.answer_text,
                provenance,
                candidates: Vec::new(),
                digest_k: 0,
                provenance_k: th.provenance_k as i32,
            }));
        }
    }

    // 2) Near hit: embed question, search question vectors, secondary check.
    let query_vec = embed_query(state, &req.question).await;
    let mut best_sim = 0.0_f32;
    if let (Some(vec), Some(qv_store)) = (&query_vec, state.question_vector_store.as_ref()) {
        if let Ok(neighbors) = qv_store.search(vec, NEAR_HIT_CANDIDATES) {
            if let Some(best) = neighbors.first() {
                best_sim = best.similarity;
                if best.similarity >= th.sim_floor {
                    if let Ok(Some(cand)) = store::blocking({
                        let storage = state.storage.clone();
                        let id = best.id;
                        move || storage.get_qa_by_rowid(id as i64)
                    })
                    .await
                    {
                        // Project + staleness gate: the question-vector space
                        // is global, so a near-hit from another project (or a
                        // stale entry) must never be served — fall to MISS.
                        if cand.project == project && !cand.stale {
                            // Trust pipeline: provenance drift forces MISS.
                            if !provenance_intact(state, &cand).await {
                                return miss_response(
                                    state,
                                    buffer_id,
                                    &req.question,
                                    &th,
                                    best_sim,
                                )
                                .await;
                            }
                            // New query's top-K chunk ids for the Jaccard check.
                            let new_ids =
                                top_chunk_ids(state, &project, &req.question, th.novel_k).await;
                            let jac = jaccard_similarity(&new_ids, &cand.source_chunk_ids);
                            let plan = resolve_plan(best.similarity, jac, &th);
                            if !plan.is_miss {
                                let cid = cand.cache_id.clone();
                                let storage = state.storage.clone();
                                let _ = store::blocking(move || storage.touch_qa(cand.id)).await;
                                let provenance = provenance_chunks(
                                    state,
                                    &cand.source_chunk_ids,
                                    th.provenance_k,
                                )
                                .await;
                                return Ok(Response::new(QueryWithCacheResponse {
                                    cache_id: cid,
                                    hit: true,
                                    tier: plan.tier,
                                    similarity: best.similarity,
                                    answer_text: cand.answer_text,
                                    provenance,
                                    candidates: Vec::new(),
                                    digest_k: plan.digest_k as i32,
                                    provenance_k: plan.provenance_k as i32,
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    // 3) Miss: return top-K raw chunks to digest client-side.
    miss_response(state, buffer_id, &req.question, &th, best_sim).await
}

/// Build the MISS response: top-K raw chunks for the client-side digest.
async fn miss_response(
    state: &AppState,
    buffer_id: Option<i64>,
    question: &str,
    th: &QaThresholds,
    best_sim: f32,
) -> Result<Response<QueryWithCacheResponse>, Status> {
    let candidates = hybrid_search(
        state,
        buffer_id.unwrap_or(0),
        &sanitize_fts(question),
        arags_search::SearchTier::Vector,
        th.novel_k,
    )
    .await
    .map_err(internal)?;

    Ok(Response::new(QueryWithCacheResponse {
        cache_id: String::new(),
        hit: false,
        tier: -1,
        similarity: best_sim,
        answer_text: String::new(),
        provenance: Vec::new(),
        candidates: to_proto_results(&candidates),
        digest_k: th.novel_k as i32,
        provenance_k: th.provenance_k as i32,
    }))
}

/// Store a client-digested answer (idempotent per project+question_hash).
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` without a session, or `internal` on failure.
pub async fn handle_store_answer(
    state: &AppState,
    request: Request<StoreAnswerRequest>,
) -> Result<Response<StoreAnswerResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.store_answer");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();
    crate::grpc::memory::record_query_history(state, &ctx, &req.project, "store", &req.question)
        .await;

    let project = req.project;
    if project.trim().is_empty() {
        return Err(invalid_arg("project is required"));
    }
    if req.question.trim().is_empty() {
        return Err(invalid_arg("question is required"));
    }
    if req.answer.trim().is_empty() {
        return Err(invalid_arg("answer is required"));
    }

    let buffer_id = resolve_buffer(state, &project, req.buffer_id).await;
    let qh = qa_store::question_hash(&req.question);

    let input = StoreAnswerInput {
        buffer_id,
        project: project.clone(),
        question_text: req.question.clone(),
        question_hash: qh,
        answer_text: req.answer,
        source_chunk_ids: req.source_chunk_ids,
        source_hashes: req.source_hashes,
        model: if req.model.is_empty() {
            None
        } else {
            Some(req.model)
        },
        tier_snapshot: Some(
            serde_json::to_string(&state.qa_config).unwrap_or_else(|_| "{}".into()),
        ),
        token_count: req.token_count,
    };

    let stored = store::blocking({
        let storage = state.storage.clone();
        move || storage.store_answer(&input)
    })
    .await
    .map_err(internal)?;

    // Embed the question and persist it in the dedicated question space.
    if let Some(vec) = embed_query(state, &req.question).await {
        if let Some(qv_store) = state.question_vector_store.as_ref() {
            if let Err(e) = qv_store.insert(stored.id as u64, &vec) {
                tracing::warn!(error = %e, "failed to persist question vector");
            }
        }
    }

    Ok(Response::new(StoreAnswerResponse {
        cache_id: stored.cache_id,
    }))
}

/// Direct, deterministic lookup of a served answer by stable id (anti-drift).
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` without a session, or `internal` on failure.
pub async fn handle_get_answer_by_id(
    state: &AppState,
    request: Request<GetAnswerByIdRequest>,
) -> Result<Response<GetAnswerByIdResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.get_answer_by_id");
    crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();

    if req.cache_id.trim().is_empty() {
        return Err(invalid_arg("cache_id is required"));
    }

    let storage = state.storage.clone();
    let cache_id = req.cache_id.clone();
    let project = req.project.clone();
    let row = store::blocking(move || storage.get_qa_by_id(&cache_id, &project))
        .await
        .map_err(internal)?;

    match row {
        Some(r) => Ok(Response::new(GetAnswerByIdResponse {
            found: true,
            cache_id: r.cache_id,
            project: r.project,
            answer_text: r.answer_text,
            source_chunk_ids: r.source_chunk_ids,
            source_hashes: r.source_hashes,
        })),
        None => Ok(Response::new(GetAnswerByIdResponse {
            found: false,
            cache_id: req.cache_id,
            project: req.project,
            answer_text: String::new(),
            source_chunk_ids: Vec::new(),
            source_hashes: Vec::new(),
        })),
    }
}

/// Admin-gated invalidation of cached answers (plan 018 + plan 017).
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` with no session, `PERMISSION_DENIED` for
/// non-admins, or `internal` on storage failure.
pub async fn handle_invalidate_cache(
    state: &AppState,
    request: Request<InvalidateCacheRequest>,
) -> Result<Response<InvalidateCacheResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.invalidate_cache");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    crate::auth::require_admin(&ctx)?;

    let req = request.into_inner();

    // Plan 017 path: invalidate a specific semantic cache entry (and radius).
    if !req.cache_id.is_empty() {
        return invalidate_qa_entry(state, &req, &ctx.username).await;
    }

    // Legacy plan 018 path: purge the `result_cache` by project.
    let project = req.project;
    let project_opt = if project.is_empty() {
        None
    } else {
        Some(project)
    };
    let invalidated = store::blocking({
        let storage = state.storage.clone();
        let p = project_opt.clone();
        move || arags_storage::cache::invalidate_cache(&storage, p.as_deref())
    })
    .await
    .map_err(internal)?;

    Ok(Response::new(InvalidateCacheResponse {
        invalidated: invalidated as i64,
        invalidated_by: ctx.username,
    }))
}

/// Invalidate a single `qa_cache` entry, optionally with a similarity radius
/// over `question_vectors` (error-chain invalidation).
async fn invalidate_qa_entry(
    state: &AppState,
    req: &InvalidateCacheRequest,
    username: &str,
) -> Result<Response<InvalidateCacheResponse>, Status> {
    let cache_id = req.cache_id.clone();
    let row = store::blocking({
        let storage = state.storage.clone();
        move || storage.get_qa_by_cache_id(&cache_id)
    })
    .await
    .map_err(internal)?;

    let Some(row) = row else {
        return Ok(Response::new(InvalidateCacheResponse {
            invalidated: 0,
            invalidated_by: username.to_string(),
        }));
    };

    let delete = req.mode == InvalidateMode::Delete as i32;
    let mut count: u64 = 0;

    // Radius neighbors first (they reference the target's question vector).
    if req.similarity_radius > 0.0 {
        if let Some(vec) = embed_query(state, &row.question_text).await {
            if let Some(qv_store) = state.question_vector_store.as_ref() {
                if let Ok(neighbors) = qv_store.search(&vec, 1000) {
                    for n in neighbors {
                        if n.id as i64 == row.id {
                            continue;
                        }
                        if n.similarity < req.similarity_radius {
                            continue;
                        }
                        let nid = n.id as i64;
                        if delete {
                            if store::blocking({
                                let storage = state.storage.clone();
                                move || storage.delete_qa(nid)
                            })
                            .await
                            .map_err(internal)?
                                > 0
                            {
                                let _ = qv_store.delete(n.id);
                                count += 1;
                            }
                        } else if store::blocking({
                            let storage = state.storage.clone();
                            let u = username.to_string();
                            move || storage.mark_qa_stale(nid, &u, "radius")
                        })
                        .await
                        .map_err(internal)?
                        {
                            count += 1;
                        }
                    }
                }
            }
        }
    }

    // The target entry itself.
    if delete {
        if store::blocking({
            let storage = state.storage.clone();
            move || storage.delete_qa(row.id)
        })
        .await
        .map_err(internal)?
            > 0
        {
            if let Some(qv_store) = state.question_vector_store.as_ref() {
                let _ = qv_store.delete(row.id as u64);
            }
            count += 1;
        }
    } else if store::blocking({
        let storage = state.storage.clone();
        let u = username.to_string();
        let id = row.id;
        move || storage.mark_qa_stale(id, &u, "admin")
    })
    .await
    .map_err(internal)?
    {
        count += 1;
    }

    Ok(Response::new(InvalidateCacheResponse {
        invalidated: count as i64,
        invalidated_by: username.to_string(),
    }))
}

/// Fetch provenance chunks for a cached answer (top `k` by stored order).
async fn provenance_chunks(state: &AppState, ids: &[String], k: usize) -> Vec<SearchResult> {
    let ids: Vec<i64> = ids.iter().filter_map(|s| s.parse::<i64>().ok()).collect();
    if ids.is_empty() {
        return Vec::new();
    }
    let taken: Vec<i64> = ids.iter().copied().take(k.max(1)).collect();
    let storage = state.storage.clone();
    let chunks = store::blocking(move || storage.get_chunks_with_content(&taken))
        .await
        .ok()
        .unwrap_or_default();
    chunks
        .into_iter()
        .map(|(c, content)| SearchResult {
            chunk_id: c.id,
            text: content.unwrap_or_default(),
            score: 1.0,
            file_path: c.file_path,
            start_line: c.line_start as i32,
            end_line: c.line_end as i32,
        })
        .collect()
}

/// Top-K chunk ids from a hybrid search (for the secondary Jaccard check).
async fn top_chunk_ids(state: &AppState, _project: &str, question: &str, k: usize) -> Vec<String> {
    // Best-effort; if search fails, an empty list yields a failing Jaccard and
    // forces a MISS (safe default).
    match hybrid_search(
        state,
        0,
        &sanitize_fts(question),
        arags_search::SearchTier::Vector,
        k,
    )
    .await
    {
        Ok(results) => results
            .iter()
            .map(|r| r.chunk_id.to_string())
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_trust;

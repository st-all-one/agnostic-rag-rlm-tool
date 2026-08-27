//! `QueryWithCache` RPC: deterministic hit/miss/tier decision over the semantic
//! QA cache.

use arags_core::qa_cache::resolve_plan;
use arags_search::qa_cache::jaccard_similarity;
use arags_storage::qa_cache as qa_store;
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg};
use crate::grpc::search::hybrid_search;
use crate::grpc::util::{sanitize_fts, to_proto_results};
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{QueryWithCacheRequest, QueryWithCacheResponse};

use super::helpers::{
    embed_query, provenance_chunks, provenance_intact, resolve_buffer, thresholds, top_chunk_ids,
};

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
    let as_of = req.as_of_epoch;

    // 1) Exact hit (same question, same project). With time-travel
    // (`as_of_epoch`), serve the revision that was active at that epoch.
    let qh_owned = qh.clone();
    let project_owned = project.clone();
    if let Some(row) = store::blocking({
        let storage = state.storage.clone();
        let p = project_owned.clone();
        let q = qh_owned.clone();
        move || {
            if as_of > 0 {
                storage.get_cached_answer_as_of(&p, &q, buffer_id, as_of)
            } else {
                storage.get_cached_answer(&p, &q)
            }
        }
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
                provenance_chunks(&state, &row.source_chunk_ids, th.provenance_k, as_of).await;
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
                answer_epoch: row.epoch,
                answer_created_by: row.created_by.clone().unwrap_or_default(),
                answer_model: row.model.clone().unwrap_or_default(),
                answer_version: row.version,
            }));
        }
    }

    // 2) Near hit: embed question, search question vectors, secondary check.
    let query_vec = embed_query(state, &req.question).await;
    let mut best_sim = 0.0_f32;
    if let (Some(vec), Some(qv_store)) = (&query_vec, state.question_vector_store.as_ref()) {
        if let Ok(neighbors) = qv_store.search(vec, super::helpers::NEAR_HIT_CANDIDATES) {
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
                                    as_of,
                                )
                                .await;
                            }
                            // Time-travel (plan 021): serve the revision active
                            // at `as_of_epoch` for this subject.
                            let answer_row = if as_of > 0 {
                                let s = state.storage.clone();
                                store::blocking(move || {
                                    s.get_cached_answer_as_of(
                                        &cand.project,
                                        &cand.question_hash,
                                        cand.buffer_id,
                                        as_of,
                                    )
                                })
                                .await
                                .map_err(internal)?
                            } else {
                                Some(cand.clone())
                            };
                            let Some(answer_row) = answer_row else {
                                return miss_response(
                                    state,
                                    buffer_id,
                                    &req.question,
                                    &th,
                                    best_sim,
                                    as_of,
                                )
                                .await;
                            };
                            // New query's top-K chunk ids for the Jaccard check.
                            let new_ids =
                                top_chunk_ids(state, &project, &req.question, th.novel_k).await;
                            let jac = jaccard_similarity(&new_ids, &answer_row.source_chunk_ids);
                            let plan = resolve_plan(best.similarity, jac, &th);
                            if !plan.is_miss {
                                let cid = answer_row.cache_id.clone();
                                let storage = state.storage.clone();
                                let _ =
                                    store::blocking(move || storage.touch_qa(answer_row.id)).await;
                                let provenance = provenance_chunks(
                                    state,
                                    &answer_row.source_chunk_ids,
                                    th.provenance_k,
                                    as_of,
                                )
                                .await;
                                return Ok(Response::new(QueryWithCacheResponse {
                                    cache_id: cid,
                                    hit: true,
                                    tier: plan.tier,
                                    similarity: best.similarity,
                                    answer_text: answer_row.answer_text,
                                    provenance,
                                    candidates: Vec::new(),
                                    digest_k: plan.digest_k as i32,
                                    provenance_k: plan.provenance_k as i32,
                                    answer_epoch: answer_row.epoch,
                                    answer_created_by: answer_row
                                        .created_by
                                        .clone()
                                        .unwrap_or_default(),
                                    answer_model: answer_row.model.clone().unwrap_or_default(),
                                    answer_version: answer_row.version,
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    // 3) Miss: return top-K raw chunks to digest client-side.
    miss_response(state, buffer_id, &req.question, &th, best_sim, as_of).await
}

/// Build the MISS response: top-K raw chunks for the client-side digest. When
/// `as_of_epoch > 0` (plan 021) the candidate chunks are time-traveled.
async fn miss_response(
    state: &AppState,
    buffer_id: Option<i64>,
    question: &str,
    th: &arags_core::qa_cache::QaThresholds,
    best_sim: f32,
    as_of_epoch: i64,
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

    let candidates = if as_of_epoch > 0 {
        crate::grpc::search::apply_chunk_as_of(state, as_of_epoch, candidates)
            .await
            .map_err(internal)?
    } else {
        candidates
    };

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
        answer_epoch: 0,
        answer_created_by: String::new(),
        answer_model: String::new(),
        answer_version: 0,
    }))
}

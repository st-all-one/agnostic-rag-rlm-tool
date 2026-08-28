//! `StoreAnswer` and `GetAnswerById` RPCs: deterministic persistence and lookup
//! of client-digested answers.

use arags_storage::qa_cache::StoreAnswerInput;
use tracing::warn;

use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{
    GetAnswerByIdRequest, GetAnswerByIdResponse, StoreAnswerRequest, StoreAnswerResponse,
};
use tonic::{Request, Response, Status};

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

    let buffer_id = super::helpers::resolve_buffer(state, &project, req.buffer_id).await;
    let qh = arags_storage::qa_cache::question_hash(&req.question);

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
        created_by: Some(ctx.username.clone()),
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
    if let Some(vec) = super::helpers::embed_query(state, &req.question).await {
        if let Some(qv_store) = state.question_vector_store.as_ref() {
            if let Err(e) = qv_store.insert(stored.id as u64, &vec) {
                warn!(error = %e, cache_id = %stored.cache_id, "failed to persist question vector; marking qa_cache pending_vector");
                if let Err(m) = state.storage.mark_qa_cache_pending_vector(&[stored.id]) {
                    warn!(error = %m, "failed to mark qa_cache pending_vector");
                }
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

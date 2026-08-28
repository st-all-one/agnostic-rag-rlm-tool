//! QA re-digest queue RPCs (issue `agnostic-rag-rlm-tool-d172`): claim and complete
//! volunteer re-digestion jobs. The server is LLM-free — the volunteer digests
//! locally and stores via `StoreAnswer`.

use crate::grpc::error::internal;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{
    ClaimPendingQaRequest, ClaimPendingQaResponse, CompletePendingQaRequest,
    CompletePendingQaResponse,
};
use tonic::{Request, Response, Status};

/// Claim the next pending QA re-digest job for this authenticated volunteer.
/// Prefers a job whose `preferred_user` matches the worker; otherwise the oldest
/// pending job is taken.
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` without a session, or `internal` on failure.
pub async fn handle_claim_pending_qa(
    state: &AppState,
    request: Request<ClaimPendingQaRequest>,
) -> Result<Response<ClaimPendingQaResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.claim_pending_qa");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();

    let worker = if req.worker_user.trim().is_empty() {
        ctx.username.clone()
    } else {
        req.worker_user
    };
    let lease_secs = if req.lease_secs > 0 {
        req.lease_secs
    } else {
        arags_storage::pending_qa::DEFAULT_PENDING_QA_LEASE_SECS
    };

    let storage = state.storage.clone();
    let claimed = store::blocking(move || storage.claim_pending_qa(&worker, lease_secs))
        .await
        .map_err(internal)?;

    Ok(Response::new(match claimed {
        Some(job) => ClaimPendingQaResponse {
            found: true,
            job_id: job.id,
            cache_id: job.cache_id,
            project: job.project,
            preferred_user: job.preferred_user.unwrap_or_default(),
            leased_until: job.leased_until.unwrap_or(0),
        },
        None => ClaimPendingQaResponse {
            found: false,
            ..ClaimPendingQaResponse::default()
        },
    }))
}

/// Complete a leased QA re-digest job. Called after the volunteer persisted the
/// fresh answer through `StoreAnswer`. Returns `ok = false` when the job was not
/// `leased` (e.g. the lease expired).
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` without a session, or `internal` on failure.
pub async fn handle_complete_pending_qa(
    state: &AppState,
    request: Request<CompletePendingQaRequest>,
) -> Result<Response<CompletePendingQaResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.complete_pending_qa");
    crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();

    let storage = state.storage.clone();
    let ok = store::blocking(move || storage.complete_pending_qa(req.job_id))
        .await
        .map_err(internal)?;

    Ok(Response::new(CompletePendingQaResponse { ok }))
}

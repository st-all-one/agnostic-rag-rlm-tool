//! Admin invalidation + admin review gate for explorations (plan 022).
//!
//! NOTE: the public consumer feedback RPC (`FeedbackExploration`,
//! `handle_feedback_exploration`) was HARD-REMOVED in issue
//! `agnostic-rlm-rs-f5f3` to eliminate a sybil-by-AI risk: an attacker could
//! flood confirm/contradict votes to manipulate map rankings. The internal
//! storage layer still exposes `record_feedback`/`FeedbackKind`, but nothing in
//! the public RPC surface writes it anymore. Admin invalidation mirrors the
//! qa-cache semantics (Stale keeps history, Delete hard-removes row + vector).

use arags_proto::proto::{
    InvalidateExplorationRequest, InvalidateExplorationResponse, InvalidateMode,
    ReviewExplorationRequest, ReviewExplorationResponse,
};
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

/// Admin-gated invalidation. `Stale` keeps the map as auditable history;
/// `Delete` hard-removes the row and its vector.
pub(crate) async fn handle_invalidate_exploration(
    state: &AppState,
    request: Request<InvalidateExplorationRequest>,
) -> Result<Response<InvalidateExplorationResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.invalidate_exploration");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    crate::auth::require_admin(&ctx)?;
    let req = request.into_inner();

    if req.exploration_id.trim().is_empty() {
        return Err(invalid_arg("exploration_id is required"));
    }

    // Resolve the row first so Delete can also drop its vector.
    let storage = state.storage.clone();
    let id = req.exploration_id.clone();
    let row = store::blocking(move || storage.get_exploration_by_uuid(&id))
        .await
        .map_err(internal)?
        .ok_or_else(|| crate::grpc::error::not_found("unknown exploration_id"))?;

    let mode = InvalidateMode::try_from(req.mode)
        .map_err(|e| invalid_arg(&format!("unknown invalidate mode: {e}")))?;
    match mode {
        InvalidateMode::Stale => {
            let reason = if req.reason.is_empty() {
                "admin".to_string()
            } else {
                req.reason.clone()
            };
            let storage = state.storage.clone();
            let id = req.exploration_id.clone();
            let applied = store::blocking(move || {
                storage
                    .invalidate_exploration_stale(&id, &reason_admin(&ctx), &reason)
                    .map_err(anyhow::Error::from)
            })
            .await
            .map_err(internal)?;
            Ok(Response::new(InvalidateExplorationResponse { applied }))
        }
        InvalidateMode::Delete => {
            let storage = state.storage.clone();
            let applied = store::blocking(move || {
                storage
                    .delete_exploration(row.id)
                    .map(|deleted| deleted > 0)
                    .map_err(anyhow::Error::from)
            })
            .await
            .map_err(internal)?;
            if applied {
                if let Some(vectors) = state.exploration_vector_store.as_ref() {
                    #[allow(clippy::cast_possible_truncation)] // rowids fit u64 here
                    if let Err(e) = vectors.delete(u64::try_from(row.id).unwrap_or(u64::MAX)) {
                        tracing::warn!(error = %e, exploration_id = %req.exploration_id, "vector delete failed");
                    }
                }
                tracing::info!(exploration_id = %req.exploration_id, "exploration deleted");
            }
            Ok(Response::new(InvalidateExplorationResponse { applied }))
        }
    }
}

fn reason_admin(ctx: &crate::auth::AuthContext) -> String {
    ctx.username.clone()
}

/// Admin quality gate (plan 023, borrowed from the RLM review gate): approve
/// flips a `pending_review` map to `fresh`; reject retires it. Admin-gated.
pub(crate) async fn handle_review_exploration(
    state: &AppState,
    request: Request<ReviewExplorationRequest>,
) -> Result<Response<ReviewExplorationResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.review_exploration");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    if !ctx.is_admin() {
        return Err(Status::permission_denied("admin role required"));
    }
    let req = request.into_inner();

    if req.exploration_id.trim().is_empty() {
        return Err(invalid_arg("exploration_id is required"));
    }

    let storage = state.storage.clone();
    let applied = store::blocking(move || {
        storage
            .review_exploration(&req.exploration_id, req.approved, &ctx.username)
            .map_err(anyhow::Error::from)
    })
    .await
    .map_err(internal)?;

    Ok(Response::new(ReviewExplorationResponse { applied }))
}

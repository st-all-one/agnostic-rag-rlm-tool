//! Consumer feedback + admin invalidation for explorations (plan 022).
//!
//! The cheapest verifier of a map is the agent that just used it: confirms
//! raise future rankings, contradictions lower confidence and auto-retire the
//! map at the configured limit. Admin invalidation mirrors the qa-cache
//! semantics (Stale keeps history, Delete hard-removes row + vector).

use arags_proto::proto::{
    FeedbackExplorationRequest, FeedbackExplorationResponse, FeedbackKind,
    InvalidateExplorationRequest, InvalidateExplorationResponse, InvalidateMode,
};
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

/// Record consumer feedback on a served map.
pub(crate) async fn handle_feedback_exploration(
    state: &AppState,
    request: Request<FeedbackExplorationRequest>,
) -> Result<Response<FeedbackExplorationResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.feedback_exploration");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();

    if req.exploration_id.trim().is_empty() {
        return Err(invalid_arg("exploration_id is required"));
    }
    let proto_kind = FeedbackKind::try_from(req.kind)
        .map_err(|e| invalid_arg(&format!("unknown feedback kind: {e}")))?;
    let kind = match proto_kind {
        FeedbackKind::Confirm => arags_storage::explorations::FeedbackKind::Confirm,
        FeedbackKind::Contradict => arags_storage::explorations::FeedbackKind::Contradict,
    };
    let limit = state.config.exploration.contradiction_limit;
    let username = ctx.username.clone();

    let storage = state.storage.clone();
    let exploration_id_for_log = req.exploration_id.clone();
    let outcome = store::blocking(move || {
        storage
            .record_feedback(&req.exploration_id, kind, limit)
            .map_err(anyhow::Error::from)
    })
    .await
    .map_err(internal)?
    .ok_or_else(|| crate::grpc::error::not_found("unknown exploration_id"))?;

    let auto_retired = matches!(
        &outcome,
        arags_storage::explorations::FeedbackOutcome::Contradicted {
            auto_retired: true,
            ..
        }
    );
    tracing::info!(
        exploration_id = %exploration_id_for_log,
        confirmed_by = %username,
        ?kind,
        auto_retired,
        "exploration feedback"
    );

    Ok(Response::new(match outcome {
        arags_storage::explorations::FeedbackOutcome::Confirmed { confirmed } => {
            FeedbackExplorationResponse {
                applied: true,
                confirmed,
                contradicted: 0,
                auto_retired: false,
            }
        }
        arags_storage::explorations::FeedbackOutcome::Contradicted {
            contradicted,
            auto_retired,
        } => FeedbackExplorationResponse {
            applied: true,
            confirmed: 0,
            contradicted,
            auto_retired,
        },
    }))
}

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

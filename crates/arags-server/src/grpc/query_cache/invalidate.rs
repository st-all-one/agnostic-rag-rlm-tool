//! `InvalidateCache` RPC: admin-gated invalidation of cached answers (plan 018 /
//! plan 017), including similarity-radius error-chain invalidation.

use crate::grpc::error::internal;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{InvalidateCacheRequest, InvalidateCacheResponse, InvalidateMode};
use tonic::{Request, Response, Status};

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
        if let Some(vec) = super::helpers::embed_query(state, &row.question_text).await {
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

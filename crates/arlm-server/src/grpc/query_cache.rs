//! `InvalidateCache` RPC (plan 017, admin-gated by plan 018).
//!
//! Purges the query/result-answer cache. Requires an authenticated **admin**
//! session — non-admins receive `PERMISSION_DENIED`. Unlike `AuthRefresh`
//! (the login RPC) and the health RPCs, this privileged operation is fully
//! gated: it must carry a valid session *and* an admin role.

use arlm_proto::proto::*;
use tonic::{Request, Response, Status};

use crate::state::AppState;
use crate::store;

/// Handle `InvalidateCache`: authenticate, enforce admin, purge cache.
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` if no valid session is presented,
/// `PERMISSION_DENIED` if the caller is not an admin, or `internal` on a
/// storage failure.
pub async fn handle_invalidate_cache(
    state: &AppState,
    request: Request<InvalidateCacheRequest>,
) -> Result<Response<InvalidateCacheResponse>, Status> {
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    crate::auth::require_admin(&ctx)?;

    let project = request.into_inner().project;
    let project_opt = if project.is_empty() {
        None
    } else {
        Some(project)
    };

    let invalidated = store::blocking({
        let storage = state.storage.clone();
        let project_opt = project_opt.clone();
        move || arlm_storage::cache::invalidate_cache(&storage, project_opt.as_deref())
    })
    .await
    .map_err(|e| Status::internal(format!("cache invalidation failed: {e}")))?;

    Ok(Response::new(InvalidateCacheResponse {
        invalidated: invalidated as i64,
        invalidated_by: ctx.username,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use tonic::metadata::{MetadataMap, MetadataValue};

    use crate::config::ServerConfig;
    use crate::state::AppState;
    use arlm_storage::{Storage, tokens::NewToken, tokens::Role};

    fn bearer(token: &str) -> MetadataMap {
        let mut md = MetadataMap::new();
        let value = MetadataValue::<tonic::metadata::Ascii>::from_str(&format!("Bearer {token}"))
            .expect("valid metadata value");
        md.insert("authorization", value);
        md
    }

    fn temp_storage() -> Storage {
        let dir = tempfile::tempdir().expect("tempdir");
        Storage::open(dir.path()).expect("open storage")
    }

    fn authed_state(storage: &Storage) -> AppState {
        AppState::new(
            storage.clone(),
            ServerConfig::default(),
            AppState::build_llm(&ServerConfig::default()).expect("build llm"),
            None,
        )
        .expect("app state")
    }

    #[tokio::test]
    async fn non_admin_cannot_invalidate() {
        let storage = temp_storage();
        storage
            .put_cached_result("h1", "proj", "answer")
            .expect("seed cache");

        let (_, refresh) = arlm_storage::tokens::create_token(
            &storage,
            &NewToken {
                username: "dev1".into(),
                role: Role::NonAdmin,
                created_by: "system".into(),
            },
        )
        .expect("create token");
        let (session, _, _, _) =
            arlm_storage::tokens::create_session(&storage, &refresh).expect("create session");

        let state = authed_state(&storage);
        let mut req = Request::new(InvalidateCacheRequest {
            project: "proj".into(),
        });
        *req.metadata_mut() = bearer(&session);

        let err = handle_invalidate_cache(&state, req)
            .await
            .expect_err("non-admin must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);

        // Cache must remain intact after the rejected call.
        assert!(
            storage
                .get_cached_result("h1", "proj")
                .expect("get")
                .is_some(),
            "cache should be untouched after denied invalidation"
        );
    }

    #[tokio::test]
    async fn admin_can_invalidate_and_audit() {
        let storage = temp_storage();
        storage
            .put_cached_result("h1", "proj", "answer")
            .expect("seed cache");
        storage
            .put_cached_result("h2", "other", "answer2")
            .expect("seed cache");

        let (_, refresh) = arlm_storage::tokens::create_token(
            &storage,
            &NewToken {
                username: "admin1".into(),
                role: Role::Admin,
                created_by: "system".into(),
            },
        )
        .expect("create token");
        let (session, _, _, _) =
            arlm_storage::tokens::create_session(&storage, &refresh).expect("create session");

        let state = authed_state(&storage);

        // Scoped purge (single project).
        let mut req = Request::new(InvalidateCacheRequest {
            project: "proj".into(),
        });
        *req.metadata_mut() = bearer(&session);
        let resp = handle_invalidate_cache(&state, req)
            .await
            .expect("admin invalidation")
            .into_inner();
        assert_eq!(resp.invalidated, 1);
        assert_eq!(resp.invalidated_by, "admin1");
        assert!(
            storage.get_cached_result("h1", "proj").expect("get").is_none(),
            "proj entry should be gone"
        );
        assert!(
            storage.get_cached_result("h2", "other").expect("get").is_some(),
            "other project untouched"
        );

        // Full purge (empty project).
        let mut req2 = Request::new(InvalidateCacheRequest {
            project: String::new(),
        });
        *req2.metadata_mut() = bearer(&session);
        let resp2 = handle_invalidate_cache(&state, req2)
            .await
            .expect("admin full purge")
            .into_inner();
        assert_eq!(resp2.invalidated, 1);
        assert!(
            storage.get_cached_result("h2", "other").expect("get").is_none(),
            "all entries purged"
        );
    }

    #[tokio::test]
    async fn missing_session_is_unauthenticated() {
        let storage = temp_storage();
        let state = authed_state(&storage);
        let req = Request::new(InvalidateCacheRequest {
            project: String::new(),
        });
        let err = handle_invalidate_cache(&state, req)
            .await
            .expect_err("no bearer must be unauthenticated");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
}

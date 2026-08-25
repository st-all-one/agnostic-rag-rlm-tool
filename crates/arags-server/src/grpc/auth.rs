//! `AuthRefresh` RPC: exchange a refresh token for a 5-minute session token.

use tonic::{Response, Status};

use crate::state::AppState;
use crate::store;

use arags_proto::proto::{AuthRefreshRequest, AuthRefreshResponse};

/// Handle `AuthRefresh`: validate the refresh token and mint a session token.
///
/// This RPC is intentionally **exempt** from `auth::authenticate` — it is the
/// login endpoint that mints the session used everywhere else.
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` if the refresh token is unknown, revoked or
/// expired; returns `internal` on a storage failure.
pub(crate) async fn handle_auth_refresh(
    state: &AppState,
    req: AuthRefreshRequest,
) -> Result<Response<AuthRefreshResponse>, Status> {
    let storage = state.storage.clone();
    let refresh = req.refresh_token.clone();

    let (session_token, username, role, expires_at) =
        store::blocking(move || arags_storage::tokens::create_session(&storage, &refresh))
            .await
            .map_err(|e| Status::unauthenticated(format!("AuthRefresh failed: {e}")))?;

    Ok(Response::new(AuthRefreshResponse {
        session_token,
        expires_at,
        role: role.as_str().to_string(),
        username,
    }))
}

//! Auth core for the gRPC server (plan 018).
//!
//! - [`Role`] is re-exported from `arlm-storage` (single source of truth).
//! - [`AuthContext`] is the caller identity resolved from a session token.
//! - [`authenticate`] enforces a valid session on a request's `Authorization`
//!   metadata; call it at the top of every RPC handler except `AuthRefresh`
//!   (the login RPC) and the health RPCs.
//! - [`require_admin`] enforces the admin gate used by privileged RPCs.
//!
//! Note: tonic 0.13's `Interceptor` cannot see the RPC path, so per-RPC
//! exemption (notably `AuthRefresh`) is done at the handler level rather than
//! via a global interceptor.

pub use arlm_storage::Role;

use arlm_storage::Storage;
use tonic::metadata::MetadataMap;
use tonic::{Request, Status};

/// Identity of the caller, resolved from a valid session token.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Auditing username (from the refresh token).
    pub username: String,
    /// Authorization role.
    pub role: Role,
}

impl AuthContext {
    /// Whether this context carries the `admin` role.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}

/// Authenticate a request from its `Authorization: Bearer` metadata.
///
/// Returns the caller's identity, or `UNAUTHENTICATED` / `internal`.
///
/// # Errors
///
/// Returns a `Status` when the session is missing, invalid, expired, or when
/// validation fails.
pub fn authenticate(md: &MetadataMap, storage: &Storage) -> Result<AuthContext, Status> {
    let token = bearer(md)?;
    match arlm_storage::tokens::validate_session(storage, &token) {
        Ok(Some((username, role))) => Ok(AuthContext { username, role }),
        Ok(None) => Err(Status::unauthenticated("invalid or expired session")),
        Err(e) => {
            tracing::error!(error = %e, "session validation failed");
            Err(Status::internal("session validation failed"))
        }
    }
}

/// Enforce the `admin` role, returning `PERMISSION_DENIED` otherwise.
///
/// # Errors
///
/// Returns a `Status` when the caller is not an admin.
pub fn require_admin(ctx: &AuthContext) -> Result<(), Status> {
    if ctx.is_admin() {
        Ok(())
    } else {
        Err(Status::permission_denied(
            "admin role required for this operation",
        ))
    }
}

fn bearer(md: &MetadataMap) -> Result<String, Status> {
    let value = md
        .get("authorization")
        .ok_or_else(|| Status::unauthenticated("missing authorization header"))?
        .to_str()
        .map_err(|_| Status::unauthenticated("authorization header is not valid UTF-8"))?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .ok_or_else(|| Status::unauthenticated("authorization must be a Bearer token"))?;
    if token.is_empty() {
        return Err(Status::unauthenticated("empty bearer token"));
    }
    Ok(token.to_string())
}

/// Helper to authenticate a typed request (carries `metadata()`).
///
/// # Errors
///
/// See [`authenticate`].
pub fn authenticate_request<T>(req: &Request<T>, storage: &Storage) -> Result<AuthContext, Status> {
    authenticate(req.metadata(), storage)
}

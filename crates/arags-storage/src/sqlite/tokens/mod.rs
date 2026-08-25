//! Refresh-token and session-token persistence for auth (plan 018).
//!
//! Two tables:
//! - `auth_tokens`: long-lived (1 year) refresh tokens, stored **hashed**.
//! - `auth_sessions`: short-lived (5 min) session tokens derived from a refresh
//!   token via `AuthRefresh`.
//!
//! `Role` is the source of truth for authorization; it round-trips to the
//! `admin` / `non_admin` strings stored in `auth_tokens.role`.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::str::FromStr;

use super::conn::Storage;

/// Authorization role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// May invalidate cache and manage tokens.
    Admin,
    /// May only query/store cache.
    NonAdmin,
}

impl Role {
    /// Stable wire/storage string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::NonAdmin => "non_admin",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "admin" => Ok(Role::Admin),
            "non_admin" => Ok(Role::NonAdmin),
            other => anyhow::bail!("unknown role: {other}"),
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Duration of a refresh token: 1 year, in milliseconds.
pub const REFRESH_TOKEN_TTL_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// Duration of a session token: 5 minutes, in milliseconds.
pub const SESSION_TOKEN_TTL_MS: i64 = 5 * 60 * 1000;

/// Length in bytes of the generated refresh token.
pub const REFRESH_TOKEN_BYTES: usize = 128;

/// Current epoch milliseconds (UTC).
#[must_use]
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Hash a refresh token with SHA-256, optionally salted by `ARAGS_TOKEN_PEPPER`.
///
/// Only the hash is persisted; the plaintext refresh token lives solely on the
/// client (`config.toml`) and at creation time.
#[must_use]
pub fn hash_refresh(refresh: &str) -> String {
    let pepper = std::env::var("ARAGS_TOKEN_PEPPER").unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(refresh.as_bytes());
    hasher.update(pepper.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a cryptographically random hex token of `n_bytes` entropy.
///
/// # Errors
///
/// Returns an error if the system CSPRNG is unavailable.
pub fn generate_secure_token(n_bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; n_bytes];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow::anyhow!("CSPRNG unavailable: {e}"))?;
    Ok(hex::encode(buf))
}

/// A refresh-token row as stored.
#[derive(Debug, Clone)]
pub struct AuthTokenRow {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub token_hash: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub created_by: String,
    pub revoked: bool,
    pub revoked_at: Option<i64>,
    pub revoked_by: Option<String>,
}

/// Input for creating a refresh token.
#[derive(Debug, Clone)]
pub struct NewToken {
    pub username: String,
    pub role: Role,
    pub created_by: String,
}

fn token_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AuthTokenRow> {
    Ok(AuthTokenRow {
        id: r.get(0)?,
        username: r.get(1)?,
        role: Role::from_str(&r.get::<_, String>(2)?).unwrap_or(Role::NonAdmin),
        token_hash: r.get(3)?,
        created_at: r.get(4)?,
        expires_at: r.get(5)?,
        created_by: r.get(6)?,
        revoked: r.get::<_, i64>(7)? != 0,
        revoked_at: r.get(8)?,
        revoked_by: r.get(9)?,
    })
}

/// Create a refresh token, returning its `(id, plaintext_refresh)`.
///
/// The plaintext is **only** available here; the DB stores `SHA-256(refresh)`.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn create_token(storage: &Storage, new: &NewToken) -> Result<(String, String)> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let id = uuid::Uuid::now_v7().to_string();
    let refresh = generate_secure_token(REFRESH_TOKEN_BYTES)?;
    let token_hash = hash_refresh(&refresh);
    let now = now_ms();
    let expires = now + REFRESH_TOKEN_TTL_MS;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO auth_tokens \
             (id, username, role, token_hash, created_at, expires_at, created_by, revoked) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![
                id,
                new.username,
                new.role.as_str(),
                token_hash,
                now,
                expires,
                new.created_by
            ],
        )?;
        Ok(())
    })
    .context("failed to insert auth token")?;

    Ok((id, refresh))
}

/// Revoke a single refresh token by id. Returns `true` if a row was affected.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn revoke_token_by_id(storage: &Storage, id: &str, revoked_by: &str) -> Result<bool> {
    revoke_tokens(storage, RevokeBy::Id(id.to_string()), revoked_by)
}

/// Revoke all refresh tokens for a username. Returns `true` if any row changed.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn revoke_token_by_username(
    storage: &Storage,
    username: &str,
    revoked_by: &str,
) -> Result<bool> {
    revoke_tokens(
        storage,
        RevokeBy::Username(username.to_string()),
        revoked_by,
    )
}

/// Target selector for revocation. Each variant maps to a **compile-time
/// fixed** SQL clause — user input is always bound, never interpolated.
enum RevokeBy {
    Id(String),
    Username(String),
}

fn revoke_tokens(storage: &Storage, by: RevokeBy, revoked_by: &str) -> Result<bool> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let now = now_ms();
    let (sql, where_val): (&'static str, String) = match by {
        RevokeBy::Id(v) => (
            "UPDATE auth_tokens SET revoked = 1, revoked_at = ?1, revoked_by = ?2 \
             WHERE id = ?3 AND revoked = 0",
            v,
        ),
        RevokeBy::Username(v) => (
            "UPDATE auth_tokens SET revoked = 1, revoked_at = ?1, revoked_by = ?2 \
             WHERE username = ?3 AND revoked = 0",
            v,
        ),
    };
    let changed = conn
        .execute(|conn| {
            let mut stmt = conn.prepare(sql)?;
            stmt.execute(params![now, revoked_by, where_val])?;
            Ok(conn.changes())
        })
        .context("failed to revoke auth tokens")?;

    if changed > 0 {
        conn.execute(|conn| {
            conn.execute(
                "DELETE FROM auth_sessions WHERE token_id IN \
                 (SELECT id FROM auth_tokens WHERE revoked = 1)",
                [],
            )?;
            Ok(())
        })
        .context("failed to purge sessions for revoked tokens")?;
    }
    Ok(changed > 0)
}

/// Revoke **every** refresh token (emergency `prune-tokens`). Returns the count.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn revoke_all_tokens(storage: &Storage, revoked_by: &str) -> Result<u64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let now = now_ms();
    let count: u64 = conn.execute(|conn| {
        conn.execute(
            "UPDATE auth_tokens SET revoked = 1, revoked_at = ?1, revoked_by = ?2 WHERE revoked = 0",
            params![now, revoked_by],
        )?;
        let n = conn.changes();
        conn.execute("DELETE FROM auth_sessions", [])?;
        Ok(n)
    })?;
    Ok(count)
}

/// List all refresh tokens (hashes only — plaintext never stored).
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn list_tokens(storage: &Storage) -> Result<Vec<AuthTokenRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    conn.execute(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, username, role, token_hash, created_at, expires_at, \
             created_by, revoked, revoked_at, revoked_by FROM auth_tokens \
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], token_row)?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    })
    .context("failed to list auth tokens")
}

pub mod session;
pub use session::{create_session, validate_session};

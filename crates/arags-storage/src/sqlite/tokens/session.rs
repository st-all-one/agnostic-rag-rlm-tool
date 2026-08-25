//! Short-lived session tokens derived from refresh tokens (plan 018).

use anyhow::{Context, Result};
use rusqlite::params;
use std::str::FromStr;

use super::{Role, SESSION_TOKEN_TTL_MS, hash_refresh, now_ms};
use crate::sqlite::conn::Storage;

/// Create a 5-minute session token from a refresh token.
///
/// Returns `(session_id, username, role, expires_at_ms)`. Errors if the refresh
/// token is unknown, revoked, or expired.
///
/// # Errors
///
/// Returns an error if the token is invalid or the insert fails.
pub fn create_session(
    storage: &Storage,
    refresh_token: &str,
) -> Result<(String, String, Role, i64)> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let token_hash = hash_refresh(refresh_token);
    let now = now_ms();

    let (token_id, username, role): (String, String, Role) = conn
        .execute(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, username, role FROM auth_tokens \
                 WHERE token_hash = ?1 AND revoked = 0 AND expires_at > ?2",
            )?;
            let mut rows = stmt.query_map(params![token_hash, now], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    Role::from_str(&r.get::<_, String>(2)?).unwrap_or(Role::NonAdmin),
                ))
            })?;
            match rows.next() {
                Some(row) => row.map_err(|e| anyhow::anyhow!(e)),
                None => anyhow::bail!("invalid or expired refresh token"),
            }
        })
        .map_err(|e| anyhow::anyhow!("AuthRefresh failed: {e}"))?;

    let session_id = uuid::Uuid::now_v7().to_string();
    let expires = now + SESSION_TOKEN_TTL_MS;
    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO auth_sessions (id, token_id, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, token_id, now, expires],
        )?;
        Ok(())
    })
    .context("failed to insert auth session")?;

    Ok((session_id, username, role, expires))
}

/// Validate a session token, returning the owning `(username, role)` if the
/// session is live **and** its refresh token is neither revoked nor expired.
///
/// # Errors
///
/// Returns an error on a database failure (an *invalid* session is `Ok(None)`).
pub fn validate_session(storage: &Storage, session_id: &str) -> Result<Option<(String, Role)>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let now = now_ms();
    conn.execute(|conn| {
        let mut stmt = conn.prepare(
            "SELECT t.username, t.role \
             FROM auth_sessions s \
             JOIN auth_tokens t ON t.id = s.token_id \
             WHERE s.id = ?1 \
               AND s.expires_at > ?2 \
               AND t.revoked = 0 \
               AND t.expires_at > ?2",
        )?;
        let mut rows = stmt.query_map(params![session_id, now], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Role::from_str(&r.get::<_, String>(1)?).unwrap_or(Role::NonAdmin),
            ))
        })?;
        match rows.next() {
            Some(row) => row.map_err(|e| anyhow::anyhow!(e)).map(Some),
            None => Ok(None),
        }
    })
    .context("failed to validate auth session")
}

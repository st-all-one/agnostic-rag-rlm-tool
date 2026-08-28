//! Append-only audit log of key data-plane actions (issue
//! `agnostic-rag-rlm-tool-7222`).
//!
//! Every mutating server RPC records a cheap, parameterized row so operators
//! can reconstruct *who* did *what* to *which* project/target and *when*.
//! Writes are best-effort on the caller side: a failure to log MUST never fail
//! the request it is auditing. All access goes through
//! [`super::conn::Storage::connection`], safe in both single (CLI) and pooled
//! (server) modes.

use anyhow::{Context, Result};
use rusqlite::params;

use super::conn::Storage;

/// A single audit-log row.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// Numeric rowid.
    pub id: i64,
    /// Project the action targeted (may be empty for global actions).
    pub project: String,
    /// Authenticated username that performed the action.
    pub username: String,
    /// Action verb (e.g. `index`, `persist_exploration`, `complete_rlm_job`).
    pub action: String,
    /// Optional target identifier (e.g. node id, buffer id, subject key).
    pub target: Option<String>,
    /// Optional human-readable detail.
    pub detail: Option<String>,
    /// Unix epoch seconds the row was written.
    pub created_at: i64,
}

impl Storage {
    /// Append an audit-log entry.
    ///
    /// 100% parameterized. Callers MUST treat the result as best-effort: a
    /// failure to record the audit row must NOT fail the request being
    /// audited. Returns the new row's `id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails (callers should `tracing::warn!`
    /// and continue).
    pub fn write_audit_log(
        &self,
        project: &str,
        username: &str,
        action: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) -> Result<i64> {
        let start = std::time::Instant::now();
        let id = self.connection()?.execute(|conn| {
            conn.execute(
                "INSERT INTO audit_log (project, username, action, target, detail) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![project, username, action, target, detail,],
            )
            .with_context(|| format!("failed to insert audit_log entry for action {action}"))?;
            Ok(conn.last_insert_rowid())
        })?;
        tracing::debug!(
            id,
            action,
            username,
            duration_ms = %start.elapsed().as_millis(),
            "audit log written"
        );
        Ok(id)
    }

    /// List audit-log entries, optionally filtered by `project` and `username`.
    ///
    /// An empty `project` or `username` disables the corresponding filter.
    /// Results are ordered newest-first and capped at `limit`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_audit_log(
        &self,
        project: &str,
        username: &str,
        limit: usize,
    ) -> Result<Vec<AuditEntry>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX).max(1);
        self.connection()?.execute(|conn| {
            let mut query = String::from(
                "SELECT id, project, username, action, target, detail, created_at \
                 FROM audit_log WHERE 1=1",
            );
            if !project.is_empty() {
                query.push_str(" AND project = ?1");
            }
            if !username.is_empty() {
                query.push_str(" AND username = ?2");
            }
            query.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?3");

            let mut stmt = conn.prepare(&query)?;
            let rows = stmt.query_map(params![project, username, limit], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    project: row.get(1)?,
                    username: row.get(2)?,
                    action: row.get(3)?,
                    target: row.get(4)?,
                    detail: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }
}

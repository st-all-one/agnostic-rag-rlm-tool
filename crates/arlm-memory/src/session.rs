use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use arlm_storage::Storage;

use crate::ScopedTimer;

/// A multi-turn session record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub project_name: String,
    pub title: String,
    pub created_at: i64,
}

/// A context entry in a session.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub version: u32,
    pub payload: String,
    pub created_at: i64,
}

/// Manages multi-turn sessions with versioned contexts and history.
pub struct SessionManager {
    storage: Storage,
}

impl SessionManager {
    /// Create a new `SessionManager`.
    ///
    /// Ensures the sessions schema exists.
    ///
    /// # Errors
    ///
    /// Returns an error if schema creation fails.
    pub fn new(storage: Storage) -> Result<Self> {
        Self::ensure_schema(&storage)?;
        Ok(Self { storage })
    }

    /// Get a reference to the underlying storage.
    #[must_use]
    pub fn get_storage(&self) -> &Storage {
        &self.storage
    }

    fn ensure_schema(storage: &Storage) -> Result<()> {
        let conn = storage.conn();
        let conn = conn.lock();

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                project_name TEXT NOT NULL,
                title TEXT NOT NULL,
                created_at INTEGER DEFAULT (unixepoch())
            ) STRICT;

            CREATE TABLE IF NOT EXISTS session_contexts (
                session_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at INTEGER DEFAULT (unixepoch()),
                PRIMARY KEY (session_id, version),
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            ) STRICT;

            CREATE TABLE IF NOT EXISTS session_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                query TEXT NOT NULL,
                result TEXT,
                created_at INTEGER DEFAULT (unixepoch()),
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            ) STRICT;
            ",
        )
        .context("failed to create session schema")?;

        Ok(())
    }

    /// Create a new session for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn create(&self, project_name: &str, title: &str) -> Result<String> {
        let _timer = ScopedTimer::new("session_create");

        let id = format!("s_{}", uuid::Uuid::now_v7());

        let conn = self.storage.conn();
        let conn = conn.lock();

        conn.execute(
            "INSERT INTO sessions (id, project_name, title) VALUES (?1, ?2, ?3)",
            params![id, project_name, title],
        )
        .context("failed to insert session")?;

        tracing::info!(session_id = %id, project = project_name, title, "session created");

        Ok(id)
    }

    /// Add a context version to a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn add_context(&self, session_id: &str, payload: &str) -> Result<u32> {
        let _timer = ScopedTimer::new("session_add_context");

        let conn = self.storage.conn();
        let conn = conn.lock();

        // Get current max version
        let max_version: u32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM session_contexts WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .context("failed to get max context version")?;

        let new_version = max_version + 1;

        conn.execute(
            "INSERT INTO session_contexts (session_id, version, payload) VALUES (?1, ?2, ?3)",
            params![session_id, new_version, payload],
        )
        .context("failed to insert session context")?;

        tracing::info!(session_id, version = new_version, "session context added");

        Ok(new_version)
    }

    /// Get the latest context for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_latest_context(&self, session_id: &str) -> Result<Option<SessionContext>> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT version, payload, created_at FROM session_contexts
                 WHERE session_id = ?1 ORDER BY version DESC LIMIT 1",
            )
            .context("failed to prepare get_latest_context")?;

        let mut rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionContext {
                version: row.get(0)?,
                payload: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;

        rows.next()
            .transpose()
            .context("failed to get latest context")
    }

    /// Get all contexts for a session, ordered by version.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_contexts(&self, session_id: &str) -> Result<Vec<SessionContext>> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT version, payload, created_at FROM session_contexts
                 WHERE session_id = ?1 ORDER BY version",
            )
            .context("failed to prepare get_contexts")?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(SessionContext {
                    version: row.get(0)?,
                    payload: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Record a query in the session history.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn record_query(&self, session_id: &str, query: &str, result: Option<&str>) -> Result<i64> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        conn.execute(
            "INSERT INTO session_history (session_id, query, result) VALUES (?1, ?2, ?3)",
            params![session_id, query, result],
        )
        .context("failed to record session query")?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    /// Get the history for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_history(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<(String, Option<String>, i64)>> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT query, result, created_at FROM session_history
                 WHERE session_id = ?1 ORDER BY created_at DESC LIMIT ?2",
            )
            .context("failed to prepare get_session_history")?;

        let rows = stmt
            .query_map(params![session_id, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Get a session by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get(&self, session_id: &str) -> Result<Option<SessionRecord>> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare("SELECT id, project_name, title, created_at FROM sessions WHERE id = ?1")
            .context("failed to prepare get_session")?;

        let mut rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                project_name: row.get(1)?,
                title: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;

        rows.next().transpose().context("failed to get session")
    }
}

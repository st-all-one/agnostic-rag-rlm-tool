//! Connection-handle operations: `conn()`-based helpers, pooled-mode access,
//! lifecycle maintenance (backup/verify/analyze), and the [`StorageConnection`]
//! handle used by the dual single/pooled modes.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::Connection;

use super::Storage;
use super::StorageMode;

impl Storage {
    /// Run a passive WAL checkpoint (best-effort background "flush",
    /// plan 020 `flush_interval_ms`). No-op when the WAL is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if the pragma execution fails.
    pub fn wal_checkpoint(&self) -> Result<()> {
        let conn = self.conn();
        let guard = conn.lock();
        guard
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .context("failed to run WAL checkpoint")?;
        Ok(())
    }

    /// Get a connection handle that works for both single and pooled modes.
    ///
    /// # Errors
    ///
    /// Returns an error if the pool is exhausted.
    pub fn connection(&self) -> Result<StorageConnection> {
        match self.mode {
            StorageMode::Single => {
                let arc = self
                    .sqlite
                    .as_ref()
                    .context("single connection not initialized")?;
                Ok(StorageConnection::Single(arc.clone()))
            }
            StorageMode::Pooled => {
                let pool = self.pool.as_ref().context("pool not initialized")?;
                let conn = pool.get().context("connection pool exhausted")?;
                Ok(StorageConnection::Pooled(conn))
            }
        }
    }

    /// Get the storage path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the storage mode.
    #[must_use]
    pub fn mode(&self) -> StorageMode {
        self.mode
    }

    /// Get pool statistics (only meaningful for pooled mode).
    #[must_use]
    pub fn pool_stats(&self) -> Option<PoolStats> {
        let pool = self.pool.as_ref()?;
        Some(PoolStats {
            max_size: pool.max_size(),
            min_idle: pool.min_idle().unwrap_or(0),
            idle_connections: pool.state().idle_connections,
            connections: pool.state().connections,
        })
    }

    /// Create a point-in-time backup of the database via `VACUUM INTO`.
    ///
    /// # Errors
    ///
    /// Returns an error if the destination path is not valid UTF-8 or the
    /// backup fails (the destination file must not already exist).
    pub fn backup(&self, dest: &Path) -> Result<()> {
        let dest_str = dest.to_str().context("non-utf8 backup path")?;
        self.connection()?.execute(|conn| {
            conn.execute_batch(&format!("VACUUM INTO '{dest_str}'"))
                .with_context(|| format!("failed to backup database to {}", dest.display()))
        })
    }

    /// Verify database integrity via `PRAGMA integrity_check`.
    ///
    /// # Errors
    ///
    /// Returns an error if the check cannot run or reports anything other than
    /// `"ok"`.
    pub fn verify(&self) -> Result<()> {
        self.connection()?.execute(|conn| {
            let mut stmt = conn
                .prepare("PRAGMA integrity_check(1)")
                .context("failed to prepare integrity_check")?;
            let result: String = stmt
                .query_row([], |row| row.get(0))
                .context("failed to run integrity_check")?;
            if result != "ok" {
                anyhow::bail!("database integrity check failed: {result}");
            }
            Ok(())
        })
    }

    /// Ensure the FTS5 extension is available in this SQLite build.
    ///
    /// # Errors
    ///
    /// Returns an error if creating an FTS5 virtual table fails.
    pub fn ensure_fts5_available(&self) -> Result<()> {
        self.connection()?.execute(|conn| {
            conn.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS _arags_fts5_probe USING fts5(content); \
                 DROP TABLE _arags_fts5_probe;",
            )
            .context("FTS5 extension is not available in this SQLite build")
        })
    }

    /// Run `ANALYZE` to refresh query planner statistics.
    ///
    /// # Errors
    ///
    /// Returns an error if the statement fails.
    pub fn analyze(&self) -> Result<()> {
        self.connection()?.execute(|conn| {
            conn.execute_batch("ANALYZE;")
                .context("failed to run ANALYZE")
        })
    }
}

/// A connection handle that can be either single or pooled.
pub enum StorageConnection {
    Single(Arc<Mutex<Connection>>),
    Pooled(r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>),
}

impl StorageConnection {
    /// Execute a closure with the underlying connection.
    ///
    /// # Errors
    ///
    /// Returns any error from the closure.
    pub fn execute<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        match self {
            Self::Single(arc) => {
                let conn = arc.lock();
                f(&conn)
            }
            Self::Pooled(conn) => f(conn),
        }
    }
}

/// Pool statistics.
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub max_size: u32,
    pub min_idle: u32,
    pub idle_connections: u32,
    pub connections: u32,
}

impl Clone for Storage {
    fn clone(&self) -> Self {
        Self {
            sqlite: self.sqlite.clone(),
            pool: self.pool.clone(),
            path: self.path.clone(),
            mode: self.mode,
        }
    }
}

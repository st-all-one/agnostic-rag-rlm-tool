use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::Connection;

use super::schema;

/// Connection mode for the storage backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageMode {
    /// Single connection mode (CLI). Uses exclusive locking.
    Single,
    /// Pooled connection mode (Server). Uses WAL with concurrent readers.
    Pooled,
}

/// `SQLite` storage with WAL mode and optimized pragmas.
///
/// Supports two modes:
/// - **Single** (CLI): One connection with exclusive locking. Fast, no contention.
/// - **Pooled** (Server): r2d2 connection pool with WAL. Concurrent readers, one writer.
pub struct Storage {
    /// Single connection mode (CLI) - kept for backward compatibility
    sqlite: Option<Arc<Mutex<Connection>>>,
    /// Pooled connection mode (Server)
    pool: Option<r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>>,
    path: PathBuf,
    mode: StorageMode,
}

impl Storage {
    /// Open in single-connection mode (CLI, backward compatible).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the database cannot
    /// be opened, pragmas cannot be applied, or migrations fail.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_single(path, false)
    }

    /// Open in single-connection mode with exclusive locking (CLI, no -shm file).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the database cannot
    /// be opened, pragmas cannot be applied, or migrations fail.
    pub fn open_exclusive(path: &Path) -> Result<Self> {
        Self::open_single(path, true)
    }

    /// Open in pooled connection mode (Server).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the database cannot
    /// be opened, pragmas cannot be applied, or migrations fail.
    pub fn open_pooled(path: &Path, max_size: u32) -> Result<Self> {
        std::fs::create_dir_all(path).context("failed to create storage directory")?;

        let db_path = path.join("knowledge.db");

        // Run migrations on a temporary connection before creating the pool
        {
            let temp_conn =
                Connection::open(&db_path).context("failed to open SQLite for migrations")?;
            Self::apply_pragmas(&temp_conn, false)?;
            schema::run_migrations(&temp_conn)?;
        }

        // Create the connection manager with pragma application
        let manager = r2d2_sqlite::SqliteConnectionManager::file(&db_path).with_init(|conn| {
            Self::apply_pragmas(conn, false).map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))
        });

        let pool = r2d2::Pool::builder()
            .max_size(max_size)
            .min_idle(Some(1))
            .build(manager)
            .context("failed to create connection pool")?;

        tracing::info!(path = %db_path.display(), max_size, "SQLite storage opened (pooled)");

        Ok(Self {
            sqlite: None,
            pool: Some(pool),
            path: path.to_path_buf(),
            mode: StorageMode::Pooled,
        })
    }

    /// Open in single-connection mode (internal).
    fn open_single(path: &Path, exclusive: bool) -> Result<Self> {
        std::fs::create_dir_all(path).context("failed to create storage directory")?;

        let db_path = path.join("knowledge.db");
        let conn = Connection::open(&db_path).context("failed to open SQLite database")?;

        Self::apply_pragmas(&conn, exclusive)?;

        // Run migrations
        schema::run_migrations(&conn)?;

        tracing::info!(path = %db_path.display(), exclusive, "SQLite storage opened");

        Ok(Self {
            sqlite: Some(Arc::new(Mutex::new(conn))),
            pool: None,
            path: path.to_path_buf(),
            mode: StorageMode::Single,
        })
    }

    /// Apply optimized pragmas to a connection.
    fn apply_pragmas(conn: &Connection, exclusive: bool) -> Result<()> {
        conn.execute_batch(
            "
            PRAGMA page_size=8192;
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA mmap_size=268435456;
            PRAGMA cache_size=-65536;
            PRAGMA temp_store=MEMORY;
            PRAGMA busy_timeout=5000;
            PRAGMA wal_autocheckpoint=2000;
            PRAGMA journal_size_limit=33554432;
            PRAGMA hard_heap_limit=104857600;
            PRAGMA threads=4;
            PRAGMA automatic_index=ON;
            PRAGMA analysis_limit=1000;
            PRAGMA optimize;
            ",
        )
        .context("failed to apply SQLite pragmas")?;

        if exclusive {
            conn.execute_batch("PRAGMA locking_mode=EXCLUSIVE;")
                .context("failed to set exclusive locking")?;
        }

        Ok(())
    }

    /// Get a reference to the underlying `SQLite` connection (single mode only).
    ///
    /// # Panics
    ///
    /// Panics if called in pooled mode.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        self.sqlite
            .as_ref()
            .expect("conn() called in pooled mode; use connection() instead")
            .clone()
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
                "CREATE VIRTUAL TABLE IF NOT EXISTS _arlm_fts5_probe USING fts5(content); \
                 DROP TABLE _arlm_fts5_probe;",
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

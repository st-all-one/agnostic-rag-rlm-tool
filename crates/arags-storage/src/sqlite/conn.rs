use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::Connection;

use super::schema;

pub(crate) mod ops;

pub use ops::{PoolStats, StorageConnection};

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
            Self::apply_pragmas(conn, false)
                .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))
        });

        let pool = r2d2::Pool::builder()
            .max_size(max_size)
            .min_idle(Some(1))
            .build(manager)
            .context("failed to create connection pool")?;

        // Hybrid mode: the pool serves `connection()` (concurrent writers),
        // while a dedicated shared connection keeps `conn()`-based read
        // helpers valid (they serialize on its mutex). WAL allows concurrent
        // readers alongside pool writers.
        let shared = Connection::open(&db_path).context("failed to open shared read connection")?;
        Self::apply_pragmas(&shared, false)?;

        tracing::info!(path = %db_path.display(), max_size, "SQLite storage opened (pooled)");

        Ok(Self {
            sqlite: Some(Arc::new(Mutex::new(shared))),
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

    /// Get a reference to the underlying shared `SQLite` connection.
    ///
    /// Available in **both** modes: single mode holds the only connection;
    /// pooled (hybrid) mode keeps a dedicated shared read connection so the
    /// `conn()`-based read helpers remain valid.
    ///
    /// # Panics
    ///
    /// Panics if storage was constructed without a shared connection, which
    /// cannot happen through the public constructors.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        self.sqlite
            .as_ref()
            .expect("storage has no shared connection")
            .clone()
    }
}

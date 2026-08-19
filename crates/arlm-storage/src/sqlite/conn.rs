use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::Connection;

use super::schema;

/// `SQLite` storage with WAL mode and optimized pragmas.
pub struct Storage {
    sqlite: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl Storage {
    /// Open or create a `SQLite` database at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the database cannot
    /// be opened, pragmas cannot be applied, or migrations fail.
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path).context("failed to create storage directory")?;

        let db_path = path.join("knowledge.db");
        let conn = Connection::open(&db_path).context("failed to open SQLite database")?;

        // Apply optimized pragmas (order matters: page_size BEFORE any write)
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
            PRAGMA optimize;
            ",
        )
        .context("failed to apply SQLite pragmas")?;

        // Run migrations
        schema::run_migrations(&conn)?;

        tracing::info!(path = %db_path.display(), "SQLite storage opened");

        Ok(Self {
            sqlite: Arc::new(Mutex::new(conn)),
            path: path.to_path_buf(),
        })
    }

    /// Get a reference to the underlying `SQLite` connection.
    #[must_use]
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        self.sqlite.clone()
    }

    /// Get the storage path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Clone for Storage {
    fn clone(&self) -> Self {
        Self {
            sqlite: self.sqlite.clone(),
            path: self.path.clone(),
        }
    }
}

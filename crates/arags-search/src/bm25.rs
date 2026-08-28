use std::time::Instant;

use anyhow::{Context, Result};
use arags_storage::Storage;
use tracing::info;

use crate::types::Bm25Result;

pub struct Bm25Search {
    storage: Storage,
}

impl Bm25Search {
    /// Create a new BM25 search instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 table cannot be created.
    pub fn new(storage: &Storage) -> Result<Self> {
        let s = Self {
            storage: storage.clone(),
        };
        s.ensure_fts_table()?;
        Ok(s)
    }

    fn ensure_fts_table(&self) -> Result<()> {
        self.storage.connection()?.execute(|conn| {
            // detail='column': stores column but not position (~40% smaller than full)
            // Supports: OR, AND, NOT, column-specific queries.
            // Does NOT support: phrases, NEAR queries (not needed for BM25).
            conn.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(\
                    content, \
                    tokenize='porter unicode61', \
                    detail='column'\
                );",
            )
            .context("failed to create chunks_fts table")?;

            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM chunks_fts", [], |row| row.get(0))
                .unwrap_or(0);
            info!(count, "chunks_fts table ready");

            Ok(())
        })
    }

    /// Rebuild the FTS index from `chunk_texts`.
    ///
    /// # Errors
    ///
    /// Returns an error if clearing or repopulating the FTS table fails.
    pub fn populate_fts(&self) -> Result<usize> {
        let start = Instant::now();
        let count = self.storage.connection()?.execute(|conn| {
            conn.execute("DELETE FROM chunks_fts", [])
                .context("failed to clear chunks_fts")?;

            conn.execute(
                "INSERT INTO chunks_fts(rowid, content) \
                 SELECT chunk_id, content FROM chunk_texts",
                [],
            )
            .context("failed to populate chunks_fts")
        })?;

        info!(
            count,
            duration_ms = %start.elapsed().as_millis(),
            "populated chunks_fts"
        );

        Ok(count)
    }

    /// Insert a single document into the FTS index.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS insert fails.
    pub fn insert_into_fts(&self, chunk_id: i64, content: &str) -> Result<()> {
        self.storage.connection()?.execute(|conn| {
            conn.execute(
                "INSERT INTO chunks_fts(rowid, content) VALUES(?1, ?2)",
                rusqlite::params![chunk_id, content],
            )
            .context("failed to insert into chunks_fts")?;
            Ok(())
        })
    }

    /// Search BM25 with `buffer_id` filter.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 query fails.
    pub fn search(&self, query: &str, buffer_id: i64, top_k: usize) -> Result<Vec<Bm25Result>> {
        let start = Instant::now();
        let limit = i64::try_from(top_k).context("top_k overflow")?;

        let safe_query = arags_storage::fts::sanitize_query(query);
        if safe_query.split_whitespace().next().is_none() {
            return Ok(Vec::new());
        }

        let results = self.storage.connection()?.execute(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT chunks_fts.rowid, bm25(chunks_fts) as score \
                     FROM chunks_fts \
                     JOIN chunks ON chunks.id = chunks_fts.rowid \
                     WHERE chunks_fts.content MATCH ?1 \
                       AND chunks.buffer_id = ?2 \
                     ORDER BY score \
                     LIMIT ?3",
                )
                .context("failed to prepare BM25 search query")?;

            let results = stmt
                .query_map(rusqlite::params![safe_query, buffer_id, limit], |row| {
                    Ok(Bm25Result {
                        chunk_id: row.get(0)?,
                        score: row.get(1)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to collect BM25 results")?;

            Ok(results)
        })?;

        info!(
            query,
            buffer_id,
            results_count = results.len(),
            duration_ms = %start.elapsed().as_millis(),
            "bm25 search completed"
        );

        Ok(results)
    }

    /// Search BM25 across all buffers.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 query fails.
    pub fn search_all(&self, query: &str, top_k: usize) -> Result<Vec<Bm25Result>> {
        let start = Instant::now();
        let limit = i64::try_from(top_k).context("top_k overflow")?;

        let safe_query = arags_storage::fts::sanitize_query(query);
        if safe_query.split_whitespace().next().is_none() {
            return Ok(Vec::new());
        }

        let results = self.storage.connection()?.execute(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT rowid, bm25(chunks_fts) as score \
                     FROM chunks_fts \
                     WHERE content MATCH ?1 \
                     ORDER BY score \
                     LIMIT ?2",
                )
                .context("failed to prepare BM25 search_all query")?;

            let results = stmt
                .query_map(rusqlite::params![safe_query, limit], |row| {
                    Ok(Bm25Result {
                        chunk_id: row.get(0)?,
                        score: row.get(1)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to collect BM25 results")?;

            Ok(results)
        })?;

        info!(
            query,
            results_count = results.len(),
            duration_ms = %start.elapsed().as_millis(),
            "bm25 search_all completed"
        );

        Ok(results)
    }
}

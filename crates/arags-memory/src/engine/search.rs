//! FTS5 BM25 search across indexed knowledge for [`MemoryEngine`](crate::engine::MemoryEngine).

use anyhow::{Context, Result};
use rusqlite::params;
use tracing::info;

use crate::ScopedTimer;
use crate::engine::{MemoryEngine, SearchOptions, SearchResult};

impl MemoryEngine {
    /// Search across indexed knowledge using FTS5 BM25.
    ///
    /// # Errors
    ///
    /// Returns an error if the search query fails.
    pub fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
        let _timer = ScopedTimer::new("memory_search");

        let conn = self.storage.conn();
        let conn = conn.lock();

        let limit = i64::try_from(options.limit).unwrap_or(i64::MAX);
        let safe_query = arags_storage::fts::sanitize_query(query);
        if safe_query.split_whitespace().next().is_none() {
            return Ok(Vec::new());
        }
        let sql = "SELECT c.id, c.file_path, c.content, bm25(chunks_fts) AS rank
                   FROM chunks_fts
                   JOIN chunks c ON c.rowid = chunks_fts.rowid
                   WHERE chunks_fts.content MATCH ?1
                   ORDER BY rank
                   LIMIT ?2";

        let mut stmt = conn.prepare(sql).context("failed to prepare FTS search")?;

        let rows: Vec<SearchResult> = stmt
            .query_map(params![safe_query, limit], |row| {
                Ok(SearchResult {
                    chunk_id: row.get(0)?,
                    file_path: row.get(1)?,
                    content: row.get(2)?,
                    score: row.get::<_, f32>(3)?.abs(),
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        info!(query, results = rows.len(), "search completed");

        Ok(rows)
    }
}

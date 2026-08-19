use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use arlm_storage::Storage;
use parking_lot::Mutex;
use rusqlite::Connection;

use crate::types::Bm25Result;

pub struct Bm25Search {
    conn: Arc<Mutex<Connection>>,
    _storage: Storage,
}

impl Bm25Search {
    /// Create a new BM25 search instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 table cannot be created.
    pub fn new(storage: &Storage) -> Result<Self> {
        let conn = storage.conn();
        let s = Self {
            conn,
            _storage: storage.clone(),
        };
        s.ensure_fts_table()?;
        Ok(s)
    }

    fn ensure_fts_table(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(\
                content, \
                tokenize='porter unicode61'\
            );",
        )
        .context("failed to create chunks_fts table")?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks_fts", [], |row| row.get(0))
            .unwrap_or(0);
        tracing::info!(count, "chunks_fts table ready");

        Ok(())
    }

    /// Rebuild the FTS index from `chunk_texts`.
    ///
    /// # Errors
    ///
    /// Returns an error if clearing or repopulating the FTS table fails.
    pub fn populate_fts(&self) -> Result<usize> {
        let start = Instant::now();
        let conn = self.conn.lock();

        conn.execute("DELETE FROM chunks_fts", [])
            .context("failed to clear chunks_fts")?;

        let count = conn
            .execute(
                "INSERT INTO chunks_fts(rowid, content) \
                 SELECT chunk_id, content FROM chunk_texts",
                [],
            )
            .context("failed to populate chunks_fts")?;

        tracing::info!(
            count,
            elapsed_ms = start.elapsed().as_millis(),
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
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO chunks_fts(rowid, content) VALUES(?1, ?2)",
            rusqlite::params![chunk_id, content],
        )
        .context("failed to insert into chunks_fts")?;
        Ok(())
    }

    /// Search BM25 with `buffer_id` filter.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 query fails.
    pub fn search(&self, query: &str, buffer_id: i64, top_k: usize) -> Result<Vec<Bm25Result>> {
        let start = Instant::now();
        let conn = self.conn.lock();
        let limit = i64::try_from(top_k).context("top_k overflow")?;

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
            .query_map(rusqlite::params![query, buffer_id, limit], |row| {
                Ok(Bm25Result {
                    chunk_id: row.get(0)?,
                    score: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to collect BM25 results")?;

        tracing::info!(
            query,
            buffer_id,
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
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
        let conn = self.conn.lock();
        let limit = i64::try_from(top_k).context("top_k overflow")?;

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
            .query_map(rusqlite::params![query, limit], |row| {
                Ok(Bm25Result {
                    chunk_id: row.get(0)?,
                    score: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to collect BM25 results")?;

        tracing::info!(
            query,
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "bm25 search_all completed"
        );

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlm_storage::sqlite::buffers::NewBuffer;
    use arlm_storage::sqlite::chunks::NewChunk;

    fn setup() -> (Bm25Search, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        let search = Bm25Search::new(&storage).unwrap();
        (search, tmp)
    }

    fn create_buffer(storage: &Storage, idx: u32) -> i64 {
        storage
            .insert_buffer(&NewBuffer {
                name: format!("test-{idx}"),
                path: format!("/test-{idx}"),
            })
            .unwrap()
    }

    fn create_chunk(storage: &Storage, buffer_id: i64, file_path: &str) -> i64 {
        storage
            .insert_chunk(&NewChunk {
                buffer_id,
                file_path: file_path.to_string(),
                offset_start: 0,
                offset_end: 100,
                line_start: 1,
                line_end: 10,
                hash: vec![0u8],
                language: Some("rust".to_string()),
                chunk_type: None,
                token_count: Some(50),
            })
            .unwrap()
    }

    #[test]
    fn test_ensure_fts_table() {
        let (search, _tmp) = setup();
        let conn = search.conn.lock();
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='chunks_fts'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(exists);
    }

    #[test]
    fn test_fts_direct() {
        let (search, _tmp) = setup();
        let conn = search.conn.lock();

        conn.execute(
            "INSERT INTO chunks_fts(rowid, content) VALUES(1, 'hello world')",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO chunks_fts(rowid, content) VALUES(2, 'hello rust')",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare("SELECT rowid FROM chunks_fts WHERE content MATCH 'hello'")
            .unwrap();
        let ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .collect();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn test_populate_and_search() {
        let (search, _tmp) = setup();
        let storage = search._storage.clone();

        let buffer_id = create_buffer(&storage, 0);
        let chunk_id = create_chunk(&storage, buffer_id, "src/main.rs");

        search
            .insert_into_fts(chunk_id, "fn main() { println!(\"hello\"); }")
            .unwrap();

        let results = search.search("hello", buffer_id, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, chunk_id);
    }

    #[test]
    fn test_search_no_match() {
        let (search, _tmp) = setup();
        let results = search.search("nonexistent", 1, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_buffer_filter() {
        let (search, _tmp) = setup();
        let storage = search._storage.clone();

        let buf1 = create_buffer(&storage, 0);
        let buf2 = create_buffer(&storage, 1);

        let c1 = create_chunk(&storage, buf1, "a.rs");
        let c2 = create_chunk(&storage, buf2, "b.rs");

        search.insert_into_fts(c1, "alpha bravo").unwrap();
        search.insert_into_fts(c2, "alpha charlie").unwrap();

        let results = search.search("alpha", buf1, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, c1);
    }

    #[test]
    fn test_search_all() {
        let (search, _tmp) = setup();
        let storage = search._storage.clone();

        let buf = create_buffer(&storage, 0);
        let c1 = create_chunk(&storage, buf, "a.rs");
        let c2 = create_chunk(&storage, buf, "b.rs");

        search.insert_into_fts(c1, "hello world").unwrap();
        search.insert_into_fts(c2, "hello rust").unwrap();

        let results = search.search_all("hello", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_populate_fts() {
        let (search, _tmp) = setup();
        let storage = search._storage.clone();

        let buf = create_buffer(&storage, 0);
        let c1 = create_chunk(&storage, buf, "a.rs");
        let c2 = create_chunk(&storage, buf, "b.rs");

        storage.insert_chunk_content(c1, "foo bar").unwrap();
        storage.insert_chunk_content(c2, "baz qux").unwrap();

        let count = search.populate_fts().unwrap();
        assert_eq!(count, 2);

        let results = search.search("foo", buf, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, c1);
    }
}

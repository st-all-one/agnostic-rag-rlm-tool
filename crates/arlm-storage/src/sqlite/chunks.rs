use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params, params_from_iter};

use super::conn::Storage;

/// Metadata for a chunk (without content).
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: i64,
    pub buffer_id: i64,
    pub file_path: String,
    pub offset_start: i64,
    pub offset_end: i64,
    pub line_start: i64,
    pub line_end: i64,
    pub hash: Vec<u8>,
    pub language: Option<String>,
    pub chunk_type: Option<String>,
    pub token_count: Option<i64>,
    pub status: String,
    pub created_at: i64,
    pub last_accessed_at: i64,
}

/// New chunk to insert.
#[derive(Debug)]
pub struct NewChunk {
    pub buffer_id: i64,
    pub file_path: String,
    pub offset_start: i64,
    pub offset_end: i64,
    pub line_start: i64,
    pub line_end: i64,
    pub hash: Vec<u8>,
    pub language: Option<String>,
    pub chunk_type: Option<String>,
    pub token_count: Option<i64>,
}

impl Storage {
    /// Insert a new chunk and return its ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_chunk(&self, chunk: &NewChunk) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
                "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    chunk.buffer_id,
                    chunk.file_path,
                    chunk.offset_start,
                    chunk.offset_end,
                    chunk.line_start,
                    chunk.line_end,
                    chunk.hash,
                    chunk.language,
                    chunk.chunk_type,
                    chunk.token_count,
                ],
            )
            .context("failed to insert chunk")?;

        let chunk_id = conn.last_insert_rowid();
        tracing::info!(chunk_id, buffer_id = chunk.buffer_id, file = %chunk.file_path, "inserted chunk");

        Ok(chunk_id)
    }

    /// Get a chunk by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunk(&self, id: i64) -> Result<Option<Chunk>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count, status, created_at, last_accessed_at
                 FROM chunks WHERE id = ?1",
            )
            .context("failed to prepare get_chunk query")?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Chunk {
                id: row.get(0)?,
                buffer_id: row.get(1)?,
                file_path: row.get(2)?,
                offset_start: row.get(3)?,
                offset_end: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                hash: row.get(7)?,
                language: row.get(8)?,
                chunk_type: row.get(9)?,
                token_count: row.get(10)?,
                status: row.get(11)?,
                created_at: row.get(12)?,
                last_accessed_at: row.get(13)?,
            })
        })?;

        rows.next().transpose().context("failed to get chunk")
    }

    /// Get chunk content by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunk_content(&self, id: i64) -> Result<Option<String>> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.query_row(
            "SELECT content FROM chunk_texts WHERE chunk_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .context("failed to get chunk content")
    }

    /// Insert chunk content.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_chunk_content(&self, chunk_id: i64, content: &str) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
            "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, ?2)",
            params![chunk_id, content],
        )
        .context("failed to insert chunk content")?;

        Ok(())
    }

    /// List all chunks for a buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_chunks(&self, buffer_id: i64) -> Result<Vec<Chunk>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count, status, created_at, last_accessed_at
                 FROM chunks WHERE buffer_id = ?1 ORDER BY id",
            )
            .context("failed to prepare list_chunks query")?;

        let rows = stmt
            .query_map(params![buffer_id], |row| {
                Ok(Chunk {
                    id: row.get(0)?,
                    buffer_id: row.get(1)?,
                    file_path: row.get(2)?,
                    offset_start: row.get(3)?,
                    offset_end: row.get(4)?,
                    line_start: row.get(5)?,
                    line_end: row.get(6)?,
                    hash: row.get(7)?,
                    language: row.get(8)?,
                    chunk_type: row.get(9)?,
                    token_count: row.get(10)?,
                    status: row.get(11)?,
                    created_at: row.get(12)?,
                    last_accessed_at: row.get(13)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Count chunks for a buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn count_chunks(&self, buffer_id: i64) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE buffer_id = ?1",
            params![buffer_id],
            |row| row.get(0),
        )
        .context("failed to count chunks")
    }

    /// Refresh `last_accessed_at` for the given chunk IDs to `unixepoch()`.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn refresh_last_accessed(&self, chunk_ids: &[i64]) -> Result<()> {
        if chunk_ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn();
        let conn = conn.lock();

        let placeholders: Vec<String> = chunk_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "UPDATE chunks SET last_accessed_at = unixepoch() WHERE id IN ({})",
            placeholders.join(", ")
        );

        conn.execute(&sql, params_from_iter(chunk_ids.iter()))
            .context("failed to refresh last_accessed_at")?;

        Ok(())
    }

    /// Check if a chunk with the same hash exists for a file.
    #[must_use]
    pub fn chunk_exists_by_hash(&self, file_path: &str, hash: &[u8]) -> bool {
        let conn = self.conn();
        let conn = conn.lock();

        conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path = ?1 AND hash = ?2",
            params![file_path, hash],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .unwrap_or(false)
    }

    /// Delete all chunks for a file path.
    pub fn delete_chunks_for_file(&self, file_path: &str) -> Result<usize> {
        let conn = self.conn();
        let conn = conn.lock();

        let deleted = conn
            .execute("DELETE FROM chunks WHERE file_path = ?1", params![file_path])
            .context("failed to delete chunks")?;

        Ok(deleted)
    }

    /// Get `last_accessed_at` for multiple chunks by ID.
    ///
    /// Returns a map of `chunk_id` -> `last_accessed_at` (unix seconds).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunks_last_accessed(
        &self,
        chunk_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, i64>> {
        if chunk_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self.conn();
        let conn = conn.lock();

        let placeholders: Vec<String> = chunk_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "SELECT id, last_accessed_at FROM chunks WHERE id IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare get_chunks_last_accessed query")?;
        let rows = stmt.query_map(params_from_iter(chunk_ids.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;

        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (id, ts) = row.context("failed to read last_accessed_at row")?;
            map.insert(id, ts);
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_storage() -> (Storage, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (storage, tmp)
    }

    fn create_test_buffer(storage: &Storage) -> i64 {
        let conn = storage.conn();
        let conn = conn.lock();
        conn.execute(
            "INSERT INTO buffers (name, path) VALUES ('test', '/test')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn test_insert_and_get_chunk() {
        let (storage, _tmp) = setup_storage();
        let buffer_id = create_test_buffer(&storage);

        let chunk = NewChunk {
            buffer_id,
            file_path: "src/main.rs".to_string(),
            offset_start: 0,
            offset_end: 100,
            line_start: 1,
            line_end: 10,
            hash: vec![0x01, 0x02, 0x03],
            language: Some("rust".to_string()),
            chunk_type: Some("function".to_string()),
            token_count: Some(50),
        };

        let id = storage.insert_chunk(&chunk).unwrap();
        assert!(id > 0);

        let retrieved = storage.get_chunk(id).unwrap().unwrap();
        assert_eq!(retrieved.file_path, "src/main.rs");
        assert_eq!(retrieved.language, Some("rust".to_string()));
    }

    #[test]
    fn test_chunk_content() {
        let (storage, _tmp) = setup_storage();
        let buffer_id = create_test_buffer(&storage);

        let chunk = NewChunk {
            buffer_id,
            file_path: "src/main.rs".to_string(),
            offset_start: 0,
            offset_end: 100,
            line_start: 1,
            line_end: 10,
            hash: vec![0x01, 0x02, 0x03],
            language: None,
            chunk_type: None,
            token_count: None,
        };

        let id = storage.insert_chunk(&chunk).unwrap();
        storage.insert_chunk_content(id, "fn main() {}").unwrap();

        let content = storage.get_chunk_content(id).unwrap().unwrap();
        assert_eq!(content, "fn main() {}");
    }

    #[test]
    fn test_list_chunks() {
        let (storage, _tmp) = setup_storage();
        let buffer_id = create_test_buffer(&storage);

        for i in 0..3 {
            let chunk = NewChunk {
                buffer_id,
                file_path: format!("src/file{i}.rs"),
                offset_start: 0,
                offset_end: 100,
                line_start: 1,
                line_end: 10,
                hash: vec![i as u8],
                language: None,
                chunk_type: None,
                token_count: None,
            };
            storage.insert_chunk(&chunk).unwrap();
        }

        let chunks = storage.list_chunks(buffer_id).unwrap();
        assert_eq!(chunks.len(), 3);
    }
}

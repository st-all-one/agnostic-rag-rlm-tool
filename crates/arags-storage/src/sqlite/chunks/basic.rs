//! Basic chunk CRUD: insert, fetch, list, count, delete.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::{CHUNK_COLS, Chunk, NewChunk, chunk_mapper};
use crate::sqlite::conn::Storage;

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
            .prepare(&format!("SELECT {CHUNK_COLS} FROM chunks WHERE id = ?1"))
            .context("failed to prepare get_chunk query")?;

        let mut rows = stmt.query_map(params![id], chunk_mapper)?;

        rows.next().transpose().context("failed to get chunk")
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

    /// List all chunks for a buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_chunks(&self, buffer_id: i64) -> Result<Vec<Chunk>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {CHUNK_COLS} FROM chunks WHERE buffer_id = ?1 ORDER BY id"
            ))
            .context("failed to prepare list_chunks query")?;

        let rows = stmt
            .query_map(params![buffer_id], chunk_mapper)?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Count every chunk across all buffers (used by integration tests to
    /// assert end-to-end indexing persisted data without knowing the buffer id).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn count_all_chunks(&self) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .context("failed to count all chunks")
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

    /// Delete all chunks for a file path.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn delete_chunks_for_file(&self, file_path: &str) -> Result<usize> {
        let conn = self.conn();
        let conn = conn.lock();

        let deleted = conn
            .execute(
                "DELETE FROM chunks WHERE file_path = ?1",
                params![file_path],
            )
            .context("failed to delete chunks")?;

        Ok(deleted)
    }
}

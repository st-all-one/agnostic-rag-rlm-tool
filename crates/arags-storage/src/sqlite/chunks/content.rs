//! Chunk content hydration and existence checks.

use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::{CHUNK_COLS, Chunk, chunk_mapper};
use crate::sqlite::conn::Storage;

impl Storage {
    /// Fetch chunks by id (in any order) together with their content, for
    /// building cache provenance payloads.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunks_with_content(&self, ids: &[i64]) -> Result<Vec<(Chunk, Option<String>)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c.prepare(&format!("SELECT {CHUNK_COLS} FROM chunks WHERE id = ?1"))?;
            let mut content_stmt = c
                .prepare("SELECT content FROM chunk_texts WHERE chunk_id = ?1")
                .context("prepare chunk content lookup")?;
            let mut out = Vec::with_capacity(ids.len());
            for id in ids {
                if let Ok(chunk) = stmt.query_row(params![id], chunk_mapper) {
                    // Content is fetched through the ALREADY-LOCKED connection:
                    // routing via `get_chunk_content` would re-lock the same
                    // non-reentrant mutex and deadlock (Single mode).
                    let content = content_stmt
                        .query_row(params![chunk.id], |row| row.get::<_, String>(0))
                        .optional()
                        .unwrap_or_default();
                    out.push((chunk, content));
                }
            }
            Ok(out)
        })
        .context("failed to fetch chunks with content")
    }

    /// Current chunk-content hashes (sha256 hex) for a buffer, used by the
    /// query-answer cache staleness hook.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn chunk_hashes_for_buffer(&self, buffer_id: i64) -> Result<HashSet<String>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c.prepare("SELECT hash FROM chunks WHERE buffer_id = ?1")?;
            let rows = stmt
                .query_map(params![buffer_id], |r| {
                    let bytes: Vec<u8> = r.get(0)?;
                    Ok(String::from_utf8_lossy(&bytes).into_owned())
                })?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(rows)
        })
        .context("failed to list chunk hashes for buffer")
    }

    /// Check if a chunk with the same hash exists for a file.
    #[must_use]
    pub fn chunk_exists_by_hash(&self, file_path: &str, hash: &[u8]) -> bool {
        let conn = self.conn();
        let conn = conn.lock();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE file_path = ?1 AND hash = ?2",
                params![file_path, hash],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        count > 0
    }
}

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

    /// Whether every `(chunk_id, expected_hash)` pair still matches the
    /// stored content hash. Missing ids count as drift (vanished provenance).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn chunk_hashes_match(&self, pairs: &[(i64, String)]) -> Result<bool> {
        if pairs.is_empty() {
            return Ok(true);
        }
        let conn = self.connection().context("failed to acquire connection")?;
        let ids: Vec<i64> = pairs.iter().map(|(id, _)| *id).collect();
        let ids_json = serde_json::to_string(&ids).context("serialize chunk ids")?;
        let current: std::collections::HashMap<i64, String> = conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, LOWER(HEX(hash)) FROM chunks \
                     WHERE id IN (SELECT value FROM json_each(?1))",
                )
                .context("prepare chunk_hashes_match")?;
            let rows = stmt
                .query_map(params![ids_json], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .context("query chunk_hashes_match")?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows.into_iter().collect())
        })?;

        for (id, expected) in pairs {
            match current.get(id) {
                Some(actual) if actual.eq_ignore_ascii_case(expected) => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Age in hours (`unixepoch() - last_accessed_at`) per chunk id, used by
    /// the serving-path salience decay after RRF fusion.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn chunk_ages_hours(&self, ids: &[i64]) -> Result<std::collections::HashMap<i64, f32>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, (unixepoch() - last_accessed_at) / 3600.0 \
                     FROM chunks WHERE id IN (SELECT value FROM json_each(?1))",
                )
                .context("prepare chunk_ages_hours")?;
            let ids_json = serde_json::to_string(ids).context("serialize chunk ids")?;
            let rows = stmt
                .query_map(params![ids_json], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
                })
                .context("query chunk_ages_hours")?;
            let mut map = std::collections::HashMap::with_capacity(ids.len());
            for row in rows {
                let (id, hours) = row.context("read chunk age")?;
                #[allow(clippy::cast_possible_truncation)] // ages fit f32 here
                map.insert(id, hours as f32);
            }
            Ok(map)
        })
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
            let mut stmt = c.prepare(
                "SELECT id, buffer_id, file_path, offset_start, offset_end, line_start, \
                 line_end, hash, language, chunk_type, token_count, status, created_at, \
                 last_accessed_at FROM chunks WHERE id = ?1",
            )?;
            let mut content_stmt = c
                .prepare("SELECT content FROM chunk_texts WHERE chunk_id = ?1")
                .context("prepare chunk content lookup")?;
            let mut out = Vec::with_capacity(ids.len());
            for id in ids {
                if let Ok(chunk) = stmt.query_row(params![id], |r| {
                    Ok(Chunk {
                        id: r.get(0)?,
                        buffer_id: r.get(1)?,
                        file_path: r.get(2)?,
                        offset_start: r.get(3)?,
                        offset_end: r.get(4)?,
                        line_start: r.get(5)?,
                        line_end: r.get(6)?,
                        hash: r.get(7)?,
                        language: r.get(8)?,
                        chunk_type: r.get(9)?,
                        token_count: r.get(10)?,
                        status: r.get(11)?,
                        created_at: r.get(12)?,
                        last_accessed_at: r.get(13)?,
                    })
                }) {
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
    pub fn chunk_hashes_for_buffer(
        &self,
        buffer_id: i64,
    ) -> Result<std::collections::HashSet<String>> {
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

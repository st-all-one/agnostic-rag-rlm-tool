//! Chunk and indexing persistence (chunks, texts, FTS5, entities, buffers).

use anyhow::{Context, Result};
use arlm_storage::Storage;
use rusqlite::params;

/// Insert a chunk row using the real `chunks` schema and return its id.
///
/// # Errors
///
/// Returns an error if the insert fails.
#[allow(clippy::too_many_arguments)]
pub fn insert_chunk(
    storage: &Storage,
    buffer_id: i64,
    file_path: &str,
    line_start: i32,
    line_end: i32,
    hash_bytes: &[u8],
    language: Option<&str>,
    chunk_type: Option<&str>,
    token_count: Option<i64>,
) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count) \
             VALUES (?1, ?2, 0, 0, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                buffer_id,
                file_path,
                line_start,
                line_end,
                hash_bytes,
                language,
                chunk_type,
                token_count,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .context("failed to insert chunk")
}

/// Insert chunk text into `chunk_texts`.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_chunk_text(storage: &Storage, chunk_id: i64, content: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, ?2)",
            params![chunk_id, content],
        )?;
        Ok(())
    })
    .context("failed to insert chunk text")
}

/// Index a chunk in the FTS5 table (`rowid` links back to `chunks.id`).
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_fts_row(storage: &Storage, chunk_id: i64, content: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO chunks_fts(rowid, content) VALUES (?1, ?2)",
            params![chunk_id, content],
        )?;
        Ok(())
    })
    .context("failed to index chunk in FTS")
}

/// Store extracted entities for a chunk.
///
/// # Errors
///
/// Returns an error if any of the inserts fail.
pub fn insert_entities(storage: &Storage, chunk_id: i64, entities: &[String]) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    for entity in entities {
        conn.execute(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO chunk_entities (chunk_id, entity) VALUES (?1, ?2)",
                params![chunk_id, entity],
            )?;
            conn.execute(
                "INSERT INTO entities_fts (entity) VALUES (?1)",
                params![entity],
            )?;
            Ok(())
        })?;
    }

    Ok(())
}

/// Update the aggregate counts on a buffer after an indexing pass.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn update_buffer_counts(
    storage: &Storage,
    buffer_id: i64,
    total_chunks: i64,
    total_files: i64,
    embedding_model: &str,
    embedding_dims: i64,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE buffers SET total_chunks = ?1, total_files = ?2, embedding_model = ?3, embedding_dims = ?4, last_indexed_at = unixepoch() \
             WHERE id = ?5",
            params![
                total_chunks,
                total_files,
                embedding_model,
                embedding_dims,
                buffer_id,
            ],
        )?;
        Ok(())
    })
    .context("failed to update buffer counts")
}

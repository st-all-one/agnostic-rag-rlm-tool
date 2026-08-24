//! Chunk and indexing persistence (chunks, texts, FTS5, entities, buffers).

use anyhow::{Context, Result};
use arags_storage::Storage;
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

/// Return the distinct `file_path`s for the given chunk ids (provenance
/// expansion for `GetCache`).
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn chunk_file_paths(storage: &Storage, ids: &[i64]) -> Result<Vec<String>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT DISTINCT file_path FROM chunks WHERE id IN ({})",
        placeholders.join(", ")
    );
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare chunk_file_paths query")?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| row.get(0))?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    })
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

/// Add to the aggregate counts on a buffer (used when multiple concurrent
/// index streams each contribute a disjoint file set).
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn increment_buffer_counts(
    storage: &Storage,
    buffer_id: i64,
    delta_chunks: i64,
    delta_files: i64,
    embedding_model: &str,
    embedding_dims: i64,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE buffers SET total_chunks = total_chunks + ?1, total_files = total_files + ?2, \
             embedding_model = ?3, embedding_dims = ?4, last_indexed_at = unixepoch() \
             WHERE id = ?5",
            params![
                delta_chunks,
                delta_files,
                embedding_model,
                embedding_dims,
                buffer_id,
            ],
        )?;
        Ok(())
    })
    .context("failed to increment buffer counts")
}

/// Insert flattened `(file_path, chunk)` pairs in transactional batches of at
/// most `max_batch` rows (`server.toml max_batch_size`, plan 020), returning
/// the persisted `(chunk_id, content)` pairs. One connection is held for the
/// whole call; each batch commits atomically.
///
/// # Errors
///
/// Returns an error if a connection cannot be acquired or any insert fails.
pub fn insert_chunks_batched(
    storage: &Storage,
    buffer_id: i64,
    items: &[(&str, &crate::indexing::IndexedChunk)],
    max_batch: usize,
) -> Result<Vec<(i64, String)>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    let mut out = Vec::with_capacity(items.len());
    conn.execute(|conn| {
        for group in items.chunks(max_batch.max(1)) {
            let tx = conn
                .unchecked_transaction()
                .context("failed to begin batch transaction")?;
            for (file_path, c) in group {
                let hash_bytes = hex::decode(&c.hash).unwrap_or_default();
                tx.execute(
                    "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count) \
                     VALUES (?1, ?2, 0, 0, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        buffer_id,
                        file_path,
                        c.line_start,
                        c.line_end,
                        hash_bytes,
                        c.language.as_deref(),
                        Some(c.chunk_type.as_str()),
                        Some(0),
                    ],
                )
                .context("failed to insert chunk")?;
                let chunk_id = tx.last_insert_rowid();

                tx.execute(
                    "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, ?2)",
                    params![chunk_id, c.content],
                )
                .context("failed to insert chunk text")?;
                tx.execute(
                    "INSERT INTO chunks_fts(rowid, content) VALUES (?1, ?2)",
                    params![chunk_id, c.content],
                )
                .context("failed to index chunk in FTS")?;

                for entity in Storage::extract_entities(&c.content, file_path) {
                    tx.execute(
                        "INSERT OR IGNORE INTO chunk_entities (chunk_id, entity) VALUES (?1, ?2)",
                        params![chunk_id, entity],
                    )
                    .context("failed to insert chunk entity")?;
                    tx.execute(
                        "INSERT INTO entities_fts (entity) VALUES (?1)",
                        params![entity],
                    )
                    .context("failed to index entity in FTS")?;
                }

                out.push((chunk_id, c.content.clone()));
            }
            tx.commit().context("failed to commit batch")?;
        }
        Ok(())
    })?;

    Ok(out)
}

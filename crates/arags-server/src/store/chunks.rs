//! Chunk and indexing persistence (chunks, texts, FTS5, entities, buffers).

use std::collections::HashMap;

use anyhow::{Context, Result};
use arags_storage::Storage;
use rusqlite::{params, params_from_iter};

/// Composite key that identifies a chunk's *position* within a buffer: the
/// chunk's provenance file plus its line span. Two index runs that touch the
/// same physical location share a key, which is how re-indexing supersedes the
/// previous active version of that chunk (issue `agnostic-rag-rlm-tool-8dcc`) instead
/// of deleting it.
pub type ChunkKey = (String, i64, i64); // (file_path, line_start, line_end)

/// Result of a batched chunk insert under immutable-versioning semantics.
pub struct BatchedInsert {
    /// `(chunk_id, content)` for every persisted chunk (new rows and unchanged
    /// rows that were reused in place), for the caller to embed + vectorize.
    pub persisted: Vec<(i64, String)>,
    /// Snapshot keys (`ChunkKey`) that matched an existing active row this
    /// batch, so the caller can drop them from its "remaining active" set.
    pub handled_keys: Vec<ChunkKey>,
    /// Ids of previous active rows that were retired this batch (superseded by
    /// a new version), so the caller can purge their vectors.
    pub retired_ids: Vec<i64>,
    /// Number of *new* rows actually inserted (excludes unchanged rows that
    /// were reused), for buffer count accounting.
    pub inserted: usize,
}

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
        "SELECT DISTINCT file_path FROM chunks WHERE id IN ({}) AND is_active = 1",
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
/// most `max_batch` rows (`server.toml max_batch_size`, plan 020), applying
/// immutable-versioning semantics (issue `agnostic-rag-rlm-tool-8dcc`):
///
/// * An active chunk already exists at the same key `(file_path, line_start,
///   line_end)` **and same hash** → unchanged; the existing id is reused and no
///   duplicate row is written.
/// * An active chunk exists at the same key but with a **different hash** → a
///   NEW active row is inserted (`version = old + 1`) and the previous active
///   row is *retired* (`is_active = 0`, `superseded_by = new_id`, FTS row
///   dropped). Its id is reported in [`BatchedInsert::retired_ids`] so the
///   caller can purge the now-orphaned vector.
/// * No active chunk exists at that key → a brand-new active row (`version = 1`)
///   is inserted.
///
/// `active` is the start-of-stream snapshot of currently-active keys (see
/// [`snapshot_active_chunks`]); it gates the per-chunk lookup and is matched
/// against the live `is_active = 1` row so the versioning decision stays
/// correct even if state drifted. The caller removes every returned
/// [`BatchedInsert::handled_keys`] entry from its "remaining active" set; any
/// of those still present at end-of-stream are orphaned (file removed / chunk
/// moved) and retired then.
///
/// One connection is held for the whole call; each batch commits atomically.
///
/// # Errors
///
/// Returns an error if a connection cannot be acquired or any insert fails.
pub fn insert_chunks_batched<S>(
    storage: &Storage,
    buffer_id: i64,
    items: &[(&str, &crate::indexing::IndexedChunk)],
    max_batch: usize,
    active: &HashMap<ChunkKey, i64, S>,
    created_by: Option<&str>,
    model: Option<&str>,
) -> Result<BatchedInsert>
where
    S: std::hash::BuildHasher,
{
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    let mut persisted = Vec::with_capacity(items.len());
    let mut handled_keys = Vec::new();
    let mut retired_ids = Vec::new();
    let mut inserted = 0usize;
    conn.execute(|conn| {
        for group in items.chunks(max_batch.max(1)) {
            let tx = conn
                .unchecked_transaction()
                .context("failed to begin batch transaction")?;
            for (file_path, c) in group {
                let key: ChunkKey = (
                    (*file_path).to_string(),
                    i64::from(c.line_start),
                    i64::from(c.line_end),
                );
                let hash_bytes = hex::decode(&c.hash).unwrap_or_default();

                // Versioning decision: only inspect the live active row when the
                // snapshot claims one exists at this key (the snapshot is the
                // authoritative start-of-stream active set).
                let existing = if active.contains_key(&key) {
                    existing_active_row(&tx, buffer_id, file_path, c.line_start, c.line_end)
                        .context("failed to read existing active chunk")?
                } else {
                    None
                };

                match existing {
                    Some((old_id, old_hash, _old_version)) if old_hash == hash_bytes => {
                        // Unchanged: reuse the id, do not insert a duplicate.
                        persisted.push((old_id, c.content.clone()));
                        handled_keys.push(key);
                    }
                    Some((old_id, _old_hash, old_version)) => {
                        // Changed: insert the new active version, then retire
                        // the previous one (its vector is purged by the caller).
                        let new_id = insert_active_chunk(
                            &tx,
                            buffer_id,
                            file_path,
                            c,
                            &hash_bytes,
                            old_version + 1,
                            created_by,
                            model,
                        )
                        .context("failed to insert superseding chunk")?;
                        retire_row(&tx, old_id, Some(new_id))
                            .context("failed to retire superseded chunk")?;
                        retired_ids.push(old_id);
                        persisted.push((new_id, c.content.clone()));
                        handled_keys.push(key);
                        inserted += 1;
                    }
                    None => {
                        // New key (or snapshot drifted): fresh active row.
                        let new_id = insert_active_chunk(
                            &tx,
                            buffer_id,
                            file_path,
                            c,
                            &hash_bytes,
                            1,
                            created_by,
                            model,
                        )
                        .context("failed to insert new chunk")?;
                        if active.contains_key(&key) {
                            handled_keys.push(key);
                        }
                        persisted.push((new_id, c.content.clone()));
                        inserted += 1;
                    }
                }
            }
            tx.commit().context("failed to commit batch")?;
        }
        Ok(())
    })?;

    Ok(BatchedInsert {
        persisted,
        handled_keys,
        retired_ids,
        inserted,
    })
}

/// Insert a single active chunk row (with text, FTS and entities) and return
/// its new id. Helper for [`insert_chunks_batched`] so the three code paths
/// share identical write logic.
///
/// # Errors
///
/// Returns an error if any of the inserts fail.
fn insert_active_chunk(
    tx: &rusqlite::Transaction<'_>,
    buffer_id: i64,
    file_path: &str,
    c: &crate::indexing::IndexedChunk,
    hash_bytes: &[u8],
    version: i64,
    created_by: Option<&str>,
    model: Option<&str>,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count, version, is_active, created_by, model) \
         VALUES (?1, ?2, 0, 0, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?11)",
        params![
            buffer_id,
            file_path,
            c.line_start,
            c.line_end,
            hash_bytes,
            c.language.as_deref(),
            Some(c.chunk_type.as_str()),
            Some(0),
            version,
            created_by,
            model,
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

    Ok(chunk_id)
}

/// Return `(id, hash, version)` of the single active chunk at
/// `(buffer_id, file_path, line_start, line_end)`, or `None` if no active row
/// exists there.
///
/// # Errors
///
/// Returns an error if the query fails.
fn existing_active_row(
    conn: &rusqlite::Connection,
    buffer_id: i64,
    file_path: &str,
    line_start: i32,
    line_end: i32,
) -> Result<Option<(i64, Vec<u8>, i64)>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, hash, version FROM chunks WHERE buffer_id = ?1 AND file_path = ?2 \
             AND line_start = ?3 AND line_end = ?4 AND is_active = 1 LIMIT 1",
        )
        .context("failed to prepare existing-active query")?;
    let mut rows = stmt
        .query_map(params![buffer_id, file_path, line_start, line_end], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .context("failed to query existing active chunk")?;
    rows.next().transpose().context("failed to read active row")
}

/// Retire a chunk row inside an open transaction: soft-delete (`is_active = 0`),
/// link the superseding row, stamp `retired_at`, and drop its FTS5 row so hybrid
/// search never surfaces it again (the caller purges the usearch vector). Used
/// by [`insert_chunks_batched`] for supersede and by the index loop for orphans.
///
/// # Errors
///
/// Returns an error if the update or FTS delete fails.
fn retire_row(
    conn: &rusqlite::Connection,
    chunk_id: i64,
    superseded_by: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE chunks SET is_active = 0, superseded_by = ?2, retired_at = unixepoch() \
         WHERE id = ?1 AND is_active = 1",
        params![chunk_id, superseded_by],
    )
    .context("failed to retire chunk")?;
    conn.execute("DELETE FROM chunks_fts WHERE rowid = ?1", params![chunk_id])
        .context("failed to delete retired chunk FTS")?;
    Ok(())
}

/// Snapshot the currently-active chunk keys → ids for a buffer (issue
/// `agnostic-rag-rlm-tool-8dcc`). Used by the index loop to (a) drive per-chunk
/// supersede decisions and (b) compute the orphan set at end-of-stream.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn snapshot_active_chunks(storage: &Storage, buffer_id: i64) -> Result<HashMap<ChunkKey, i64>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT file_path, line_start, line_end, id FROM chunks \
                 WHERE buffer_id = ?1 AND is_active = 1",
            )
            .context("failed to prepare active snapshot")?;
        let rows = stmt
            .query_map(params![buffer_id], |r| {
                Ok((
                    (
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ),
                    r.get::<_, i64>(3)?,
                ))
            })
            .context("failed to query active snapshot")?;
        let mut map = HashMap::new();
        for row in rows {
            let (key, id) = row.context("failed to read active snapshot row")?;
            map.insert(key, id);
        }
        Ok(map)
    })
}

/// Retire a single active chunk (soft-delete). `superseded_by` links the row
/// that replaced it, or `None` for an orphan (file removed / chunk moved). The
/// FTS5 row is dropped so hybrid search never returns it; the caller is
/// responsible for purging the usearch vector (issue `agnostic-rag-rlm-tool-8dcc`).
///
/// # Errors
///
/// Returns an error if the transaction fails.
pub fn retire_chunk(storage: &Storage, chunk_id: i64, superseded_by: Option<i64>) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    conn.execute(|conn| retire_row(conn, chunk_id, superseded_by))
        .context("failed to retire chunk")
}

/// Permanently purge retired (`is_active = 0`) chunks whose `retired_at` is
/// older than `retention_days` days, cascading to `chunks_fts`,
/// `chunk_texts` and `chunk_entities`. Their usearch vectors were already
/// removed at retire time (see [`retire_chunk`]), so this only reclaims DB
/// history. `retention_days = 0` purges everything retired.
///
/// Pool-safe: runs through [`arags_storage::Storage::connection`].
///
/// # Errors
///
/// Returns an error if the transaction fails.
pub fn purge_inactive_chunks(storage: &Storage, retention_days: u64) -> Result<usize> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    conn.execute(|conn| {
        let cutoff: i64 = (retention_days as i64) * 86_400;
        let mut stmt = conn
            .prepare(
                "SELECT id FROM chunks WHERE is_active = 0 AND retired_at IS NOT NULL \
             AND retired_at <= unixepoch() - ?1",
            )
            .context("failed to prepare purge select")?;
        let ids: Vec<i64> = stmt
            .query_map(params![cutoff], |r| r.get(0))
            .context("failed to query retired ids")?
            .filter_map(std::result::Result::ok)
            .collect();

        if ids.is_empty() {
            return Ok(0);
        }

        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        conn.execute(
            &format!("DELETE FROM chunks_fts WHERE rowid IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )
        .context("delete chunks_fts for purge")?;
        conn.execute(
            &format!("DELETE FROM chunk_texts WHERE chunk_id IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )
        .context("delete chunk_texts for purge")?;
        conn.execute(
            &format!("DELETE FROM chunk_entities WHERE chunk_id IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )
        .context("delete chunk_entities for purge")?;
        conn.execute(
            &format!("DELETE FROM chunks WHERE id IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )
        .context("delete chunks for purge")?;

        Ok(ids.len())
    })
    .context("failed to purge inactive chunks")
}

/// Delete every chunk belonging to `buffer_id`, cascading to `chunks_fts`,
/// `chunk_texts` and `chunk_entities`, and return the deleted chunk ids together
/// with the number of distinct files they covered.
///
/// This is the re-index stopgap for `agnostic-rag-rlm-tool-20cd`: calling it before
/// [`insert_chunks_batched`] makes a repeated `IndexProject` *replace* rather
/// than *append*, keeping chunk/FTS/vector counts stable. The durable fix is
/// immutable versioned writes (`agnostic-rag-rlm-tool-8dcc`).
///
/// Pool-safe: runs through [`arags_storage::Storage::connection`], so it works in
/// both single and pooled (server) modes.
///
/// # Errors
///
/// Returns an error if the transaction fails.
pub fn delete_chunks_for_buffer(storage: &Storage, buffer_id: i64) -> Result<(Vec<i64>, usize)> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        // Snapshot ids + files before deleting so we can report them and the
        // caller can purge the matching vectors.
        let (ids, distinct_files) = {
            let mut stmt = conn
                .prepare("SELECT id, file_path FROM chunks WHERE buffer_id = ?1")
                .context("prepare select chunks for buffer")?;
            let rows = stmt
                .query_map(params![buffer_id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .context("query chunks for buffer")?
                .filter_map(std::result::Result::ok)
                .collect::<Vec<_>>();
            let mut files = std::collections::HashSet::new();
            for (_, fp) in &rows {
                files.insert(fp.clone());
            }
            let ids: Vec<i64> = rows.into_iter().map(|(id, _)| id).collect();
            (ids, files.len())
        };

        if ids.is_empty() {
            return Ok((ids, distinct_files));
        }

        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        conn.execute(
            &format!("DELETE FROM chunks_fts WHERE rowid IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )
        .context("delete chunks_fts for buffer")?;
        conn.execute(
            &format!("DELETE FROM chunk_texts WHERE chunk_id IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )
        .context("delete chunk_texts for buffer")?;
        conn.execute(
            &format!("DELETE FROM chunk_entities WHERE chunk_id IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )
        .context("delete chunk_entities for buffer")?;
        conn.execute(
            "DELETE FROM chunks WHERE buffer_id = ?1",
            params![buffer_id],
        )
        .context("delete chunks for buffer")?;

        // NOTE: `entities_fts` may retain orphan entity rows for entities that
        // only appeared in this buffer; they are benign (entity search joins
        // through `chunk_entities`, which is now empty for them) and are
        // reconciled by the maintenance sweep (plan 49d6).

        Ok((ids, distinct_files))
    })
    .context("failed to delete chunks for buffer (cascade)")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use arags_storage::Storage;

    /// Insert a minimal active chunk row (no FTS/texts) directly for tests.
    /// Takes the already-open `rusqlite` connection so it never acquires a
    /// second one (which would deadlock single-connection mode).
    fn seed_chunk(conn: &rusqlite::Connection, id: i64, path: &str, hash: &[u8]) {
        conn.execute(
            "INSERT INTO chunks (id, buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, version, is_active) \
             VALUES (?1, 1, ?2, 0, 0, 1, 1, ?3, 1, 1)",
            rusqlite::params![id, path, hash],
        )
        .unwrap();
    }

    #[test]
    fn purge_inactive_chunks_respects_retention_window() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();

        let conn = storage.connection().unwrap();
        conn.execute(|c| {
            c.execute(
                "INSERT OR IGNORE INTO buffers (id, name, path) VALUES (1, 'b', '/tmp/b')",
                [],
            )?;
            seed_chunk(c, 1, "a.rs", &[0x00]);
            // Old retired chunk (100 days ago).
            c.execute(
                "UPDATE chunks SET is_active = 0, retired_at = unixepoch() - 100*86400 WHERE id = 1",
                [],
            )?;
            seed_chunk(c, 2, "b.rs", &[0x01]);
            // Recently retired chunk (now).
            c.execute(
                "UPDATE chunks SET is_active = 0, retired_at = unixepoch() WHERE id = 2",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        // retention 7 days: only the 100-day-old chunk is purged.
        let purged = super::purge_inactive_chunks(&storage, 7).unwrap();
        assert_eq!(purged, 1, "only the old retired chunk should be purged");

        let remaining: i64 = storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM chunks WHERE is_active = 0", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(remaining, 1, "recently retired chunk kept within window");

        // retention 0: purge everything retired (cascade to texts too).
        let conn = storage.connection().unwrap();
        conn.execute(|c| {
            c.execute(
                "INSERT INTO chunk_texts (chunk_id, content) VALUES (2, 'recent')",
                [],
            )?;
            c.execute(
                "INSERT INTO chunks_fts (rowid, content) VALUES (2, 'recent')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let purged = super::purge_inactive_chunks(&storage, 0).unwrap();
        assert_eq!(purged, 1, "retention 0 purges remaining retired chunk");
        let fts: i64 = storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM chunks_fts WHERE rowid = 2", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(fts, 0, "chunk_texts FTS cascaded on purge");
    }

    #[test]
    fn retire_chunk_drops_fts_marks_inactive_links_superseder() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        let conn = storage.connection().unwrap();
        conn.execute(|c| {
            c.execute(
                "INSERT OR IGNORE INTO buffers (id, name, path) VALUES (1, 'b', '/tmp/b')",
                [],
            )?;
            seed_chunk(c, 1, "a.rs", &[0x00]);
            c.execute(
                "INSERT INTO chunk_texts (chunk_id, content) VALUES (1, 'hello')",
                [],
            )?;
            c.execute(
                "INSERT INTO chunks_fts (rowid, content) VALUES (1, 'hello')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        super::retire_chunk(&storage, 1, Some(2)).unwrap();

        let active: i64 = storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM chunks WHERE is_active = 1", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(active, 0, "retired chunk is inactive");

        let fts: i64 = storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM chunks_fts WHERE rowid = 1", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(fts, 0, "FTS row dropped on retire");

        let sup: Option<i64> = storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(
                    c.query_row("SELECT superseded_by FROM chunks WHERE id = 1", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(sup, Some(2), "superseded_by links the new version");
    }
}

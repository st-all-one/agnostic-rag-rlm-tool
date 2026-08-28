//! Row mapping, column projection, and the supersede-aware insert for the QA cache.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::types::{QaCacheRow, StoreAnswerInput};

/// Projection shared by all `qa_cache` row queries (order fixed; see
/// [`row_mapper`]). The trailing `is_active` / `superseded_by` / `epoch` /
/// `created_by` / `version` are what the supersede chain walks for time-travel
/// (issue `agnostic-rag-rlm-tool-e210` / plan 021).
pub(crate) const QA_COLS: &str = "id, cache_id, buffer_id, project, question_text, question_hash, \
     answer_text, source_chunk_ids, source_hashes, model, confidence, tier_snapshot, \
     token_count, access_count, created_at, last_accessed_at, stale, invalidated_at, \
     invalidated_by, invalidated_reason, is_active, superseded_by, epoch, created_by, version";

/// Parse a JSON array column into a `Vec<String>`.
pub(crate) fn parse_json_array(text: Option<String>) -> Vec<String> {
    match text {
        Some(s) if !s.is_empty() => serde_json::from_str::<Vec<String>>(&s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub(crate) fn row_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<QaCacheRow> {
    Ok(QaCacheRow {
        id: r.get(0)?,
        cache_id: r.get(1)?,
        buffer_id: r.get(2)?,
        project: r.get(3)?,
        question_text: r.get(4)?,
        question_hash: r.get(5)?,
        answer_text: r.get(6)?,
        source_chunk_ids: parse_json_array(r.get::<_, Option<String>>(7)?),
        source_hashes: parse_json_array(r.get::<_, Option<String>>(8)?),
        model: r.get(9)?,
        confidence: r.get(10)?,
        tier_snapshot: r.get(11)?,
        token_count: r.get(12)?,
        access_count: r.get(13)?,
        created_at: r.get(14)?,
        last_accessed_at: r.get(15)?,
        stale: r.get::<_, i64>(16)? != 0,
        invalidated_at: r.get(17)?,
        invalidated_by: r.get(18)?,
        invalidated_reason: r.get(19)?,
        is_active: r.get::<_, i64>(20)? != 0,
        superseded_by: r.get(21)?,
        epoch: r.get(22)?,
        created_by: r.get(23)?,
        version: r.get(24)?,
    })
}

/// Supersede-aware insert for a QA answer: retire any active row at
/// `(project, buffer_id, question_hash)` and insert a fresh active revision
/// (issue `agnostic-rag-rlm-tool-e210`). Runs inside an open transaction so the
/// retire/insert pair commits atomically.
pub(crate) fn store_answer_inner(
    c: &rusqlite::Connection,
    input: &StoreAnswerInput,
    now: i64,
) -> Result<(String, i64)> {
    let tx = c.unchecked_transaction().context("begin store_answer tx")?;

    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT id, version FROM qa_cache \
             WHERE project = ?1 AND buffer_id IS ?2 AND question_hash = ?3 \
               AND is_active = 1 LIMIT 1",
            params![input.project, input.buffer_id, input.question_hash],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()
        .context("failed to probe existing qa_cache entry")?;

    let cache_id = uuid::Uuid::now_v7().to_string();
    let chunk_ids_json = serde_json::to_string(&input.source_chunk_ids)
        .context("failed to serialize source_chunk_ids")?;
    let hashes_json =
        serde_json::to_string(&input.source_hashes).context("failed to serialize source_hashes")?;
    let snapshot = input
        .tier_snapshot
        .clone()
        .unwrap_or_else(|| "{}".to_string());

    let new_id: i64 = match existing {
        Some((old_id, old_version)) => {
            // Retire the previous active row first so the partial unique index
            // (one active per subject) is never violated by the insert.
            tx.execute(
                "UPDATE qa_cache SET is_active = 0 WHERE id = ?1 AND is_active = 1",
                params![old_id],
            )
            .context("failed to retire superseded qa_cache row")?;
            let rowid: i64 = tx
                .query_row(
                    "INSERT INTO qa_cache \
                      (cache_id, buffer_id, project, question_text, question_hash, \
                       answer_text, source_chunk_ids, source_hashes, model, created_by, \
                       version, is_active, confidence, tier_snapshot, token_count, \
                       access_count, created_at, last_accessed_at, stale) \
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 1.0, ?12, \
                              ?13, 0, ?14, ?14, 0) \
                      RETURNING id",
                    params![
                        cache_id,
                        input.buffer_id,
                        input.project,
                        input.question_text,
                        input.question_hash,
                        input.answer_text,
                        chunk_ids_json,
                        hashes_json,
                        input.model,
                        input.created_by,
                        old_version + 1,
                        snapshot,
                        input.token_count,
                        now,
                    ],
                    |r| r.get(0),
                )
                .context("failed to insert superseding qa_cache row")?;
            tx.execute(
                "UPDATE qa_cache SET superseded_by = ?1 WHERE id = ?2",
                params![rowid, old_id],
            )
            .context("failed to link superseded qa_cache row")?;
            rowid
        }
        None => tx
            .query_row(
                "INSERT INTO qa_cache \
                  (cache_id, buffer_id, project, question_text, question_hash, \
                   answer_text, source_chunk_ids, source_hashes, model, created_by, \
                   version, is_active, confidence, tier_snapshot, token_count, \
                   access_count, created_at, last_accessed_at, stale) \
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, 1, 1.0, ?11, ?12, 0, \
                          ?13, ?13, 0) \
                  RETURNING id",
                params![
                    cache_id,
                    input.buffer_id,
                    input.project,
                    input.question_text,
                    input.question_hash,
                    input.answer_text,
                    chunk_ids_json,
                    hashes_json,
                    input.model,
                    input.created_by,
                    snapshot,
                    input.token_count,
                    now,
                ],
                |r| r.get(0),
            )
            .context("failed to insert qa_cache entry")?,
    };

    tx.commit().context("commit store_answer tx")?;
    Ok((cache_id, new_id))
}

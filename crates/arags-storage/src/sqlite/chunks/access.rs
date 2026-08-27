//! Access-bookkeeping and staleness helpers for chunks: last-accessed refresh,
//! hash/provenance verification, and age queries.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter};

use crate::sqlite::conn::Storage;

impl Storage {
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
        let current: HashMap<i64, String> = conn.execute(|c| {
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
    pub fn chunk_ages_hours(&self, ids: &[i64]) -> Result<HashMap<i64, f32>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, (unixepoch() - last_accessed_at) / 3600.0 \
                     FROM chunks WHERE id IN (SELECT value FROM json_each(?1)) AND is_active = 1",
                )
                .context("prepare chunk_ages_hours")?;
            let ids_json = serde_json::to_string(ids).context("serialize chunk ids")?;
            let rows = stmt
                .query_map(params![ids_json], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
                })
                .context("query chunk_ages_hours")?;
            let mut map = HashMap::with_capacity(ids.len());
            for row in rows {
                let (id, hours) = row.context("read chunk age")?;
                #[allow(clippy::cast_possible_truncation)] // ages fit f32 here
                map.insert(id, hours as f32);
            }
            Ok(map)
        })
    }

    /// Get `last_accessed_at` for multiple chunks by ID.
    ///
    /// Returns a map of `chunk_id` -> `last_accessed_at` (unix seconds).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunks_last_accessed(&self, chunk_ids: &[i64]) -> Result<HashMap<i64, i64>> {
        if chunk_ids.is_empty() {
            return Ok(HashMap::new());
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

        let mut map = HashMap::new();
        for row in rows {
            let (id, ts) = row.context("failed to read last_accessed_at row")?;
            map.insert(id, ts);
        }
        Ok(map)
    }
}

//! Vector re-embedding lifecycle for chunks: mark/pending/clear `pending_vector`
//! and read the embed input triples for the reconcile worker.

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter};

use crate::sqlite::conn::Storage;

impl Storage {
    /// Mark the given chunks as awaiting vector re-derivation.
    ///
    /// Sets `status = 'pending_vector'` for every chunk in `chunk_ids` that
    /// belongs to `buffer_id`. The canonical text is preserved, so a reconcile
    /// worker (issue `agnostic-rlm-rs-36ae`) can re-embed later.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_chunks_pending_vector(&self, buffer_id: i64, chunk_ids: &[i64]) -> Result<()> {
        if chunk_ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn();
        let conn = conn.lock();

        let placeholders: Vec<String> = chunk_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "UPDATE chunks SET status = 'pending_vector' WHERE buffer_id = ?1 AND id IN ({})",
            placeholders.join(", ")
        );

        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&buffer_id];
        for id in chunk_ids {
            params.push(id);
        }

        conn.execute(&sql, params_from_iter(params.iter()))
            .context("failed to mark chunks pending_vector")?;
        Ok(())
    }

    /// Return the IDs of chunks in `buffer_id` awaiting vector re-derivation.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn chunks_pending_vector(&self, buffer_id: i64) -> Result<Vec<i64>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare("SELECT id FROM chunks WHERE buffer_id = ?1 AND status = 'pending_vector'")
            .context("failed to prepare chunks_pending_vector query")?;
        let rows = stmt
            .query_map(params![buffer_id], |row| row.get::<_, i64>(0))
            .context("failed to query chunks_pending_vector")?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.context("failed to read pending_vector chunk id")?);
        }
        Ok(ids)
    }

    /// Return `(chunk_id, buffer_id, content)` triples for the given chunk ids,
    /// used by the reconcile worker (`agnostic-rlm-rs-36ae`) to re-embed from
    /// canonical text. Chunks whose `chunk_texts` row has been purged are
    /// skipped (they cannot be reconciled and remain pending).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunk_embed_inputs(&self, ids: &[i64]) -> Result<Vec<(i64, i64, String)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn();
        let conn = conn.lock();
        let placeholders: Vec<String> = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "SELECT ct.chunk_id, c.buffer_id, ct.content \
             FROM chunk_texts ct JOIN chunks c ON c.id = ct.chunk_id \
             WHERE ct.chunk_id IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare chunk embed inputs query")?;
        let rows = stmt
            .query_map(params_from_iter(ids.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("failed to query chunk embed inputs")?;
        let mut out = Vec::with_capacity(ids.len());
        for row in rows {
            out.push(row.context("failed to read chunk embed input")?);
        }
        Ok(out)
    }

    /// Return `(chunk_id, buffer_id, content)` triples for **every** chunk that
    /// has canonical text (all `chunk_texts` rows joined to `chunks`). Used by
    /// the server bootstrap rebuild (`agnostic-rlm-rs-620d`) to reconstruct the
    /// chunk vector space from SQLite when it diverges from the store.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn all_chunk_embed_inputs(&self) -> Result<Vec<(i64, i64, String)>> {
        let conn = self.conn();
        let conn = conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT ct.chunk_id, c.buffer_id, ct.content \
                 FROM chunk_texts ct JOIN chunks c ON c.id = ct.chunk_id",
            )
            .context("failed to prepare all chunk embed inputs query")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("failed to query all chunk embed inputs")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to read all chunk embed input")?);
        }
        Ok(out)
    }

    /// Clear the `pending_vector` marker for the given chunks after a successful
    /// re-embed, restoring the normal `active` status.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn clear_chunks_pending_vector(&self, buffer_id: i64, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn();
        let conn = conn.lock();
        let placeholders: Vec<String> = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "UPDATE chunks SET status = 'active' WHERE buffer_id = ?1 AND id IN ({})",
            placeholders.join(", ")
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&buffer_id];
        for id in ids {
            params.push(id);
        }
        conn.execute(&sql, params_from_iter(params.iter()))
            .context("failed to clear chunks pending_vector")?;
        Ok(())
    }
}

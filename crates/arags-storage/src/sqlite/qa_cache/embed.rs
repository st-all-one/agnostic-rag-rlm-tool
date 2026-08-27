//! QA-cache embed-input extraction for the reconcile worker and server bootstrap
//! rebuild.

use anyhow::{Context, Result};
use rusqlite::params_from_iter;

use crate::sqlite::conn::Storage;

impl Storage {
    /// Return `(id, question_text)` pairs for the given QA cache rows, used by
    /// the reconcile worker (`agnostic-rlm-rs-36ae`) to re-embed the canonical
    /// question text from SQLite. Missing rows are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_embed_inputs(&self, ids: &[i64]) -> Result<Vec<(i64, String)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let placeholders: Vec<String> = ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect();
            let sql = format!(
                "SELECT id, question_text FROM qa_cache WHERE id IN ({})",
                placeholders.join(", ")
            );
            let mut stmt = c.prepare(&sql).context("prepare qa embed inputs query")?;
            let rows = stmt
                .query_map(params_from_iter(ids.iter()), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .context("query qa embed inputs")?;
            let mut out = Vec::with_capacity(ids.len());
            for row in rows {
                out.push(row.context("read qa embed input")?);
            }
            Ok(out)
        })
    }

    /// Return `(id, question_text)` pairs for **every** QA cache row, used by
    /// the server bootstrap rebuild (`agnostic-rlm-rs-620d`) to reconstruct the
    /// question vector space from SQLite when it diverges from the store.
    /// Missing rows are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn all_qa_embed_inputs(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare("SELECT id, question_text FROM qa_cache")
                .context("prepare all qa embed inputs query")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .context("query all qa embed inputs")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.context("read all qa embed input")?);
            }
            Ok(out)
        })
    }
}

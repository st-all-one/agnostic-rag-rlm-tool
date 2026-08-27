//! Time-travel chunk lookup (plan 021): walk the supersede chain to the revision
//! active at a given epoch.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::Chunk;
use crate::sqlite::conn::Storage;

impl Storage {
    /// Time-travel: return the chunk revision that was **active** at `as_of_epoch`
    /// (a unix-second epoch), starting from `chunk_id` (normally the current live
    /// revision returned by search). The active revision at time T is the one with
    /// the greatest `epoch <= T` among `chunk_id`'s supersede family (the chain of
    /// rows linked by `superseded_by`). If every revision has `epoch > T` the
    /// chunk did not yet exist, so `None` is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if any query fails.
    pub fn get_chunk_as_of(&self, chunk_id: i64, as_of_epoch: i64) -> Result<Option<Chunk>> {
        // Collect every revision in the supersede family by walking backward via
        // `superseded_by` (each row points to the NEWER revision it was replaced
        // by), newest → oldest.
        let mut family: Vec<Chunk> = Vec::new();
        let mut current = chunk_id;
        loop {
            let Some(chunk) = self.get_chunk(current)? else {
                break;
            };
            family.push(chunk);
            let predecessor: Option<i64> = {
                let conn = self.conn();
                let conn = conn.lock();
                conn.query_row(
                    "SELECT id FROM chunks WHERE superseded_by = ?1",
                    params![current],
                    |r| r.get(0),
                )
                .optional()
                .context("failed to read chunk supersede predecessor")?
            };
            match predecessor {
                Some(id) if id != current => current = id,
                _ => break,
            }
        }

        Ok(family
            .into_iter()
            .filter(|c| c.epoch <= as_of_epoch)
            .max_by(|a, b| a.epoch.cmp(&b.epoch).then_with(|| a.id.cmp(&b.id))))
    }
}

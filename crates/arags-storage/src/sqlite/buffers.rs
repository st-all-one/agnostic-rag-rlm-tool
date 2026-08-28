use anyhow::{Context, Result};
use rusqlite::params;
use tracing::info;

use super::conn::Storage;

/// Buffer (project/directory) metadata.
#[derive(Debug, Clone)]
pub struct Buffer {
    pub id: i64,
    pub uuid: Option<String>,
    pub name: String,
    pub path: String,
    pub total_chunks: i64,
    pub total_files: i64,
    pub embedding_model: Option<String>,
    pub embedding_dims: Option<i64>,
    pub last_indexed_at: Option<i64>,
    pub created_at: i64,
}

/// New buffer to insert.
#[derive(Debug)]
pub struct NewBuffer {
    pub name: String,
    pub path: String,
}

const BUFFER_COLUMNS: &str = "id, uuid, name, path, total_chunks, total_files, embedding_model, embedding_dims, last_indexed_at, created_at";

fn row_to_buffer(row: &rusqlite::Row<'_>) -> rusqlite::Result<Buffer> {
    Ok(Buffer {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        total_chunks: row.get(4)?,
        total_files: row.get(5)?,
        embedding_model: row.get(6)?,
        embedding_dims: row.get(7)?,
        last_indexed_at: row.get(8)?,
        created_at: row.get(9)?,
    })
}

impl Storage {
    /// Insert a new buffer and return its ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_buffer(&self, buffer: &NewBuffer) -> Result<i64> {
        let uuid = uuid::Uuid::now_v7().to_string();
        let conn = self.conn();
        let conn = conn.lock();

        let start = std::time::Instant::now();
        conn.execute(
            "INSERT INTO buffers (name, path, uuid) VALUES (?1, ?2, ?3)",
            params![buffer.name, buffer.path, uuid],
        )
        .context("failed to insert buffer")?;

        let buffer_id = conn.last_insert_rowid();
        info!(buffer_id, name = %buffer.name, path = %buffer.path, duration_ms = %start.elapsed().as_millis(), "inserted buffer");

        Ok(buffer_id)
    }

    /// Get a buffer by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_buffer(&self, id: i64) -> Result<Option<Buffer>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {BUFFER_COLUMNS} FROM buffers WHERE id = ?1"
            ))
            .context("failed to prepare get_buffer query")?;

        let mut rows = stmt.query_map(params![id], row_to_buffer)?;

        rows.next().transpose().context("failed to get buffer")
    }

    /// Get a buffer by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_buffer_by_name(&self, name: &str) -> Result<Option<Buffer>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {BUFFER_COLUMNS} FROM buffers WHERE name = ?1"
            ))
            .context("failed to prepare get_buffer_by_name query")?;

        let mut rows = stmt.query_map(params![name], row_to_buffer)?;

        rows.next()
            .transpose()
            .context("failed to get buffer by name")
    }

    /// Get a buffer by UUID.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_buffer_by_uuid(&self, uuid: &str) -> Result<Option<Buffer>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {BUFFER_COLUMNS} FROM buffers WHERE uuid = ?1"
            ))
            .context("failed to prepare get_buffer_by_uuid query")?;

        let mut rows = stmt.query_map(params![uuid], row_to_buffer)?;

        rows.next()
            .transpose()
            .context("failed to get buffer by uuid")
    }

    /// List all buffers.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_buffers(&self) -> Result<Vec<Buffer>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {BUFFER_COLUMNS} FROM buffers ORDER BY name"
            ))
            .context("failed to prepare list_buffers query")?;

        let rows = stmt
            .query_map([], row_to_buffer)?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Backfill UUID for existing buffers that don't have one.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn ensure_uuids(&self) -> Result<u64> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare("SELECT id FROM buffers WHERE uuid IS NULL")
            .context("failed to prepare ensure_uuids query")?;

        let ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(std::result::Result::ok)
            .collect();

        let mut updated = 0u64;
        for id in ids {
            let uuid = uuid::Uuid::now_v7().to_string();
            conn.execute(
                "UPDATE buffers SET uuid = ?1 WHERE id = ?2",
                params![uuid, id],
            )
            .context("failed to update buffer uuid")?;
            updated += 1;
        }

        if updated > 0 {
            info!(updated, "backfilled UUIDs for existing buffers");
        }

        Ok(updated)
    }

    /// Update buffer counts after indexing.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn update_buffer_counts(
        &self,
        buffer_id: i64,
        total_chunks: i64,
        total_files: i64,
    ) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
            "UPDATE buffers SET total_chunks = ?1, total_files = ?2, last_indexed_at = unixepoch() WHERE id = ?3",
            params![total_chunks, total_files, buffer_id],
        )
        .context("failed to update buffer counts")?;

        Ok(())
    }

    /// Delete a buffer and its associated chunks.
    ///
    /// # Errors
    ///
    /// Returns an error if any of the database deletes fail.
    pub fn delete_buffer(&self, buffer_id: i64) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();
        let start = std::time::Instant::now();

        let tx = conn.unchecked_transaction()?;
        // Cascade to every child table so deleting a buffer leaves no orphans in
        // FTS5 / entity indexes (the same "deletes don't cascade" gap tracked in
        // agnostic-rag-rlm-tool-20cd).
        tx.execute(
            "DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE buffer_id = ?1)",
            params![buffer_id],
        )?;
        tx.execute(
            "DELETE FROM chunk_entities WHERE chunk_id IN (SELECT id FROM chunks WHERE buffer_id = ?1)",
            params![buffer_id],
        )?;
        tx.execute(
            "DELETE FROM chunk_texts WHERE chunk_id IN (SELECT id FROM chunks WHERE buffer_id = ?1)",
            params![buffer_id],
        )?;
        tx.execute(
            "DELETE FROM chunks WHERE buffer_id = ?1",
            params![buffer_id],
        )?;
        tx.execute("DELETE FROM buffers WHERE id = ?1", params![buffer_id])?;
        tx.commit()?;

        info!(buffer_id, duration_ms = %start.elapsed().as_millis(), "deleted buffer");

        Ok(())
    }
}

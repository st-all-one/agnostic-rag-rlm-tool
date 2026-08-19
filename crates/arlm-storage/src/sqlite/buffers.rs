use anyhow::{Context, Result};
use rusqlite::params;

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

const BUFFER_COLUMNS: &str =
    "id, uuid, name, path, total_chunks, total_files, embedding_model, embedding_dims, last_indexed_at, created_at";

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

        conn.execute(
            "INSERT INTO buffers (name, path, uuid) VALUES (?1, ?2, ?3)",
            params![buffer.name, buffer.path, uuid],
        )
        .context("failed to insert buffer")?;

        let buffer_id = conn.last_insert_rowid();
        tracing::info!(buffer_id, name = %buffer.name, path = %buffer.path, "inserted buffer");

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
            tracing::info!(updated, "backfilled UUIDs for existing buffers");
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

        let tx = conn.unchecked_transaction()?;
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

        tracing::info!(buffer_id, "deleted buffer");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_storage() -> (Storage, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (storage, tmp)
    }

    #[test]
    fn test_insert_and_get_buffer() {
        let (storage, _tmp) = setup_storage();

        let buffer = NewBuffer {
            name: "my-project".to_string(),
            path: "/path/to/project".to_string(),
        };

        let id = storage.insert_buffer(&buffer).unwrap();
        assert!(id > 0);

        let retrieved = storage.get_buffer(id).unwrap().unwrap();
        assert_eq!(retrieved.name, "my-project");
        assert_eq!(retrieved.path, "/path/to/project");
        assert!(retrieved.uuid.is_some());
    }

    #[test]
    fn test_get_buffer_by_uuid() {
        let (storage, _tmp) = setup_storage();

        let buffer = NewBuffer {
            name: "my-project".to_string(),
            path: "/path/to/project".to_string(),
        };

        storage.insert_buffer(&buffer).unwrap();
        let buffers = storage.list_buffers().unwrap();
        let uuid = buffers[0].uuid.as_deref().unwrap();

        let retrieved = storage.get_buffer_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(retrieved.name, "my-project");
    }

    #[test]
    fn test_get_buffer_by_name() {
        let (storage, _tmp) = setup_storage();

        let buffer = NewBuffer {
            name: "my-project".to_string(),
            path: "/path/to/project".to_string(),
        };

        storage.insert_buffer(&buffer).unwrap();

        let retrieved = storage.get_buffer_by_name("my-project").unwrap().unwrap();
        assert_eq!(retrieved.path, "/path/to/project");
    }

    #[test]
    fn test_list_buffers() {
        let (storage, _tmp) = setup_storage();

        for i in 0..3 {
            let buffer = NewBuffer {
                name: format!("project-{i}"),
                path: format!("/path/to/project-{i}"),
            };
            storage.insert_buffer(&buffer).unwrap();
        }

        let buffers = storage.list_buffers().unwrap();
        assert_eq!(buffers.len(), 3);
    }

    #[test]
    fn test_update_buffer_counts() {
        let (storage, _tmp) = setup_storage();

        let buffer = NewBuffer {
            name: "my-project".to_string(),
            path: "/path/to/project".to_string(),
        };

        let id = storage.insert_buffer(&buffer).unwrap();
        storage.update_buffer_counts(id, 100, 10).unwrap();

        let retrieved = storage.get_buffer(id).unwrap().unwrap();
        assert_eq!(retrieved.total_chunks, 100);
        assert_eq!(retrieved.total_files, 10);
        assert!(retrieved.last_indexed_at.is_some());
    }

    #[test]
    fn test_ensure_uuids_backfill() {
        let (storage, _tmp) = setup_storage();

        // Insert a buffer normally (gets UUID)
        let buffer = NewBuffer {
            name: "project-a".to_string(),
            path: "/path/a".to_string(),
        };
        let id = storage.insert_buffer(&buffer).unwrap();

        // Manually NULL the UUID to simulate old data
        {
            let conn = storage.conn();
            let conn = conn.lock();
            conn.execute("UPDATE buffers SET uuid = NULL WHERE id = ?1", params![id])
                .unwrap();
        }

        // Verify UUID is NULL
        let b = storage.get_buffer(id).unwrap().unwrap();
        assert!(b.uuid.is_none());

        // Backfill
        let count = storage.ensure_uuids().unwrap();
        assert_eq!(count, 1);

        // Verify UUID is now set
        let b = storage.get_buffer(id).unwrap().unwrap();
        assert!(b.uuid.is_some());
    }
}

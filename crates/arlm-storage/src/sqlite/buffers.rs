use anyhow::{Context, Result};
use rusqlite::params;

use super::conn::Storage;

/// Buffer (project/directory) metadata.
#[derive(Debug, Clone)]
pub struct Buffer {
    pub id: i64,
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

impl Storage {
    /// Insert a new buffer and return its ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_buffer(&self, buffer: &NewBuffer) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
            "INSERT INTO buffers (name, path) VALUES (?1, ?2)",
            params![buffer.name, buffer.path],
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
            .prepare(
                "SELECT id, name, path, total_chunks, total_files, embedding_model, embedding_dims, last_indexed_at, created_at
                 FROM buffers WHERE id = ?1",
            )
            .context("failed to prepare get_buffer query")?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Buffer {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                total_chunks: row.get(3)?,
                total_files: row.get(4)?,
                embedding_model: row.get(5)?,
                embedding_dims: row.get(6)?,
                last_indexed_at: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

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
            .prepare(
                "SELECT id, name, path, total_chunks, total_files, embedding_model, embedding_dims, last_indexed_at, created_at
                 FROM buffers WHERE name = ?1",
            )
            .context("failed to prepare get_buffer_by_name query")?;

        let mut rows = stmt.query_map(params![name], |row| {
            Ok(Buffer {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                total_chunks: row.get(3)?,
                total_files: row.get(4)?,
                embedding_model: row.get(5)?,
                embedding_dims: row.get(6)?,
                last_indexed_at: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        rows.next()
            .transpose()
            .context("failed to get buffer by name")
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
            .prepare(
                "SELECT id, name, path, total_chunks, total_files, embedding_model, embedding_dims, last_indexed_at, created_at
                 FROM buffers ORDER BY name",
            )
            .context("failed to prepare list_buffers query")?;

        let rows = stmt
            .query_map([], |row| {
                Ok(Buffer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    total_chunks: row.get(3)?,
                    total_files: row.get(4)?,
                    embedding_model: row.get(5)?,
                    embedding_dims: row.get(6)?,
                    last_indexed_at: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
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
}

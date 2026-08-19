use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use arrow::array::{ArrayRef, Float32Array, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use arrow_array::FixedSizeListArray;
use futures::TryStreamExt;
use lancedb::connection::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::table::Table;
use tracing::info;

const VECTOR_DIMS: i32 = 1024;

/// Vector store using `LanceDB`.
pub struct VectorStore {
    _conn: Connection,
    pub(crate) table: Table,
}

/// Vector entry for insertion.
#[derive(Debug)]
pub struct VectorEntry {
    pub chunk_id: u64,
    pub buffer_id: u64,
    pub vector: Vec<f32>,
}

impl VectorStore {
    /// Open or create a vector store at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the `LanceDB` connection or table creation fails.
    pub async fn open(path: &Path) -> Result<Self> {
        let db_path = path.join("vectors.lance");
        let db_path_str = db_path.to_string_lossy();

        info!("opening LanceDB at {}", db_path.display());

        let conn = lancedb::connect(&db_path_str)
            .execute()
            .await
            .context("failed to connect to LanceDB")?;

        let table = if let Ok(t) = conn.open_table("vectors").execute().await {
            t
        } else {
            info!("creating vectors table");
            let schema = Self::arrow_schema();
            conn.create_empty_table("vectors", schema)
                .execute()
                .await
                .context("failed to create vectors table")?
        };

        Ok(Self { _conn: conn, table })
    }

    /// `LanceDB` schema for vectors.
    fn arrow_schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("chunk_id", DataType::UInt64, false),
            Field::new("buffer_id", DataType::UInt64, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    VECTOR_DIMS,
                ),
                false,
            ),
        ]))
    }

    /// Insert vectors into the store.
    ///
    /// # Errors
    ///
    /// Returns an error if the arrow batch construction or `LanceDB` insert fails.
    pub async fn insert_vectors(&self, entries: &[VectorEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        info!("inserting {} vectors", entries.len());

        let chunk_ids: UInt64Array = entries.iter().map(|e| Some(e.chunk_id)).collect();
        let buffer_ids: UInt64Array = entries.iter().map(|e| Some(e.buffer_id)).collect();

        let flat_values: Vec<f32> = entries
            .iter()
            .flat_map(|e| e.vector.iter().copied())
            .collect();

        let values_array: ArrayRef = Arc::new(Float32Array::from(flat_values));
        let vector_list = FixedSizeListArray::try_new(
            Arc::new(Field::new("item", DataType::Float32, true)),
            VECTOR_DIMS,
            values_array,
            None,
        )
        .context("failed to create FixedSizeListArray for vectors")?;

        let batch = RecordBatch::try_new(
            Self::arrow_schema(),
            vec![
                Arc::new(chunk_ids) as ArrayRef,
                Arc::new(buffer_ids) as ArrayRef,
                Arc::new(vector_list) as ArrayRef,
            ],
        )
        .context("failed to create record batch")?;

        self.table
            .add(batch)
            .execute()
            .await
            .context("failed to insert vectors")?;

        Ok(())
    }

    /// Search for similar vectors.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector search query or result collection fails.
    pub async fn search(
        &self,
        query_vector: &[f32],
        buffer_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<(u64, f32)>> {
        let mut search_query = self
            .table
            .vector_search(query_vector)
            .context("failed to build vector search query")?;

        if let Some(bid) = buffer_id {
            search_query = search_query.only_if(format!("buffer_id = {bid}"));
        }

        let results = search_query
            .limit(limit)
            .execute()
            .await
            .context("failed to execute vector search")?;

        let batches: Vec<RecordBatch> = results
            .try_collect()
            .await
            .context("failed to collect search results")?;

        let mut entries = Vec::new();
        for batch in &batches {
            let chunk_ids = batch
                .column_by_name("chunk_id")
                .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
                .context("missing chunk_id column")?;
            let distances = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
                .context("missing _distance column")?;

            for i in 0..batch.num_rows() {
                entries.push((chunk_ids.value(i), distances.value(i)));
            }
        }

        Ok(entries)
    }

    /// Return the number of vectors in the store.
    ///
    /// # Errors
    ///
    /// Returns an error if the row count query fails.
    pub async fn count(&self) -> Result<usize> {
        self.table
            .count_rows(None)
            .await
            .context("failed to count vectors")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_vector_store_open() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();
        assert!(store.count().await.unwrap() == 0);
    }

    #[tokio::test]
    async fn test_insert_vectors() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();

        let entries: Vec<VectorEntry> = (0..5)
            .map(|i| VectorEntry {
                chunk_id: i,
                buffer_id: 0,
                vector: vec![i as f32; VECTOR_DIMS as usize],
            })
            .collect();

        store.insert_vectors(&entries).await.unwrap();
        assert!(store.count().await.unwrap() == 5);
    }

    #[tokio::test]
    async fn test_insert_empty() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();
        store.insert_vectors(&[]).await.unwrap();
        assert!(store.count().await.unwrap() == 0);
    }

    #[tokio::test]
    async fn test_search_vectors() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();

        let entries: Vec<VectorEntry> = (0..3)
            .map(|i| VectorEntry {
                chunk_id: i,
                buffer_id: 0,
                vector: vec![i as f32; VECTOR_DIMS as usize],
            })
            .collect();

        store.insert_vectors(&entries).await.unwrap();

        let query = vec![0.0_f32; VECTOR_DIMS as usize];
        let results = store.search(&query, None, 10).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, 0);
    }

    #[tokio::test]
    async fn test_search_with_buffer_filter() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();

        let entries = vec![
            VectorEntry {
                chunk_id: 0,
                buffer_id: 1,
                vector: vec![1.0; VECTOR_DIMS as usize],
            },
            VectorEntry {
                chunk_id: 1,
                buffer_id: 2,
                vector: vec![2.0; VECTOR_DIMS as usize],
            },
        ];

        store.insert_vectors(&entries).await.unwrap();

        let query = vec![1.0_f32; VECTOR_DIMS as usize];
        let results = store.search(&query, Some(1), 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 0);
    }
}

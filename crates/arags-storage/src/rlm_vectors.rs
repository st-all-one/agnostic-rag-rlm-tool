//! Summary-vector index for RLM recursive summaries.
//!
//! This is a **dedicated** `usearch` index (`rlm_vectors`) in its own vector
//! space, separate from the chunk store (`vector.usearch`) and the QA question
//! index (`question_vectors.usearch`). It holds the embedding of each summary
//! node so semantic search over the hierarchy (file/theme/project summaries)
//! never mixes with chunk retrieval or cache lookup.
//!
//! Keys are the `rlm_nodes.id` rowids (`u64`), keeping a 1:1 logical mapping
//! to `rlm_nodes`. The index is persisted next to the SQLite database and
//! saved on every mutation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::sqlite::conn::Storage;

const DEFAULT_RLM_DIMS: usize = 384;
const INDEX_FILE: &str = "rlm_vectors.usearch";

/// A nearest neighbour of a summary query, with cosine similarity in `[0, 1]`.
#[derive(Debug, Clone)]
pub struct RlmNeighbor {
    /// `rlm_nodes.id` of the matching summary.
    pub id: u64,
    /// Cosine similarity to the query (clamped to `[0, 1]`).
    pub similarity: f32,
}

/// `usearch` index of RLM summary embeddings (cosine metric).
pub struct RlmVectorStore {
    index: Index,
    index_path: PathBuf,
}

impl RlmVectorStore {
    /// Open (or create) the summary index at `storage_path/rlm_vectors.usearch`.
    ///
    /// # Errors
    ///
    /// Returns an error if the index file cannot be created or restored.
    pub fn open(storage_path: &Path, dims: usize) -> Result<Self> {
        let dims = if dims == 0 { DEFAULT_RLM_DIMS } else { dims };
        std::fs::create_dir_all(storage_path).context("failed to create storage dir")?;
        let index_path = storage_path.join(INDEX_FILE);

        let index = if index_path.exists() {
            let path_str = index_path.to_str().context("non-utf8 index path")?;
            Index::restore(path_str)
                .map_err(|e| anyhow::anyhow!("failed to restore rlm vector index: {e}"))?
        } else {
            let opts = IndexOptions {
                dimensions: dims,
                metric: MetricKind::Cos,
                quantization: ScalarKind::F32,
                connectivity: 0,
                expansion_add: 0,
                expansion_search: 0,
                ..Default::default()
            };
            Index::new(&opts)
                .map_err(|e| anyhow::anyhow!("failed to create rlm vector index: {e}"))?
        };

        Ok(Self { index, index_path })
    }

    /// Build a store rooted alongside the given `Storage`'s directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot be opened.
    pub fn open_for_storage(storage: &Storage, dims: usize) -> Result<Self> {
        Self::open(storage.path(), dims)
    }

    /// The embedding dimensionality.
    #[must_use]
    pub fn dimensions(&self) -> usize {
        self.index.dimensions()
    }

    /// Number of indexed summaries.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.index.size() as u64
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.size() == 0
    }

    /// Insert or replace a summary vector keyed by `rlm_nodes.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector has the wrong dimensionality or the
    /// insertion fails.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<()> {
        let dims = self.index.dimensions();
        if vector.len() != dims {
            anyhow::bail!(
                "rlm vector dimension mismatch: expected {dims}, got {}",
                vector.len()
            );
        }
        // usearch does not upsert; remove any prior key first.
        let _ = self.index.remove(id);
        self.index
            .reserve(self.index.size() + 1)
            .map_err(|e| anyhow::anyhow!("failed to reserve rlm index capacity: {e}"))?;
        self.index
            .add(id, vector)
            .map_err(|e| anyhow::anyhow!("failed to add rlm vector {id}: {e}"))?;
        self.save()
    }

    /// Remove a summary vector by `rlm_nodes.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the deletion or save fails.
    pub fn delete(&self, id: u64) -> Result<()> {
        let _ = self.index.remove(id);
        self.save()
    }

    /// Search for the `limit` nearest summaries, returning cosine similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the search fails.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<RlmNeighbor>> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let matches = self
            .index
            .search(query, limit)
            .map_err(|e| anyhow::anyhow!("rlm vector search failed: {e}"))?;
        let out = matches
            .keys
            .iter()
            .copied()
            .zip(matches.distances.iter().copied())
            .map(|(id, dist)| RlmNeighbor {
                id,
                // `cos` metric returns cosine distance (1 - cos); clamp to [0, 1].
                similarity: (1.0_f32 - dist).clamp(0.0, 1.0),
            })
            .collect();
        Ok(out)
    }

    fn save(&self) -> Result<()> {
        let path_str = self.index_path.to_str().context("non-utf8 index path")?;
        self.index
            .save(path_str)
            .map_err(|e| anyhow::anyhow!("failed to save rlm vector index: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(dims: usize) -> (tempfile::TempDir, RlmVectorStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = RlmVectorStore::open(dir.path(), dims).expect("open");
        (dir, store)
    }

    #[test]
    fn insert_search_and_delete_roundtrip() {
        let (_dir, store) = temp_store(4);
        assert!(store.is_empty());
        assert_eq!(store.dimensions(), 4);

        store.insert(10, &[1.0, 0.0, 0.0, 0.0]).expect("insert 10");
        store.insert(20, &[0.0, 1.0, 0.0, 0.0]).expect("insert 20");
        assert_eq!(store.len(), 2);

        let hits = store.search(&[1.0, 0.0, 0.0, 0.0], 2).expect("search");
        assert_eq!(hits[0].id, 10);
        assert!(hits[0].similarity > 0.99);

        // Replace keeps one entry per key.
        store.insert(10, &[0.0, 0.0, 1.0, 0.0]).expect("replace");
        assert_eq!(store.len(), 2);
        let hits = store.search(&[0.0, 0.0, 1.0, 0.0], 2).expect("search");
        assert_eq!(hits[0].id, 10);

        store.delete(10).expect("delete");
        assert_eq!(store.len(), 1);
        let hits = store.search(&[0.0, 0.0, 1.0, 0.0], 2).expect("search");
        assert_eq!(hits[0].id, 20);
    }

    #[test]
    fn wrong_dimensionality_is_rejected() {
        let (_dir, store) = temp_store(4);
        let err = store.insert(1, &[1.0, 0.0]).expect_err("dim mismatch");
        assert!(err.to_string().contains("dimension mismatch"));
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let store = RlmVectorStore::open(dir.path(), 2).expect("open");
            store.insert(7, &[0.5, 0.5]).expect("insert");
        }
        let reopened = RlmVectorStore::open(dir.path(), 2).expect("reopen");
        assert_eq!(reopened.len(), 1);
        let hits = reopened.search(&[0.5, 0.5], 1).expect("search");
        assert_eq!(hits[0].id, 7);
    }
}

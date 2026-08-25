//! Exploration-vector index for the explorations dataset (plan 022).
//!
//! This is a **dedicated** `usearch` index (`exploration_vectors`) in its own
//! vector space, separate from chunk retrieval (`vector.usearch`) and from
//! question lookup (`question_vectors.usearch`). It holds the embedding of
//! each map's `goal + summary` under the `cos` metric.
//!
//! Keys are the `explorations.id` rowids (`u64`), keeping a 1:1 logical
//! mapping to `explorations`. The index is persisted next to the SQLite
//! database and saved on every mutation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::sqlite::conn::Storage;

const DEFAULT_EXPLORATION_DIMS: usize = 1024;
const INDEX_FILE: &str = "exploration_vectors.usearch";

/// A nearest neighbour of an exploration query, with cosine similarity in
/// `[0, 1]`.
#[derive(Debug, Clone)]
pub struct ExplorationNeighbor {
    /// `explorations.id` of the matching map.
    pub id: u64,
    /// Cosine similarity to the query (clamped to `[0, 1]`).
    pub similarity: f32,
}

/// `usearch` index of exploration-summary embeddings (cosine metric).
pub struct ExplorationVectorStore {
    index: Index,
    index_path: PathBuf,
}

impl ExplorationVectorStore {
    /// Open (or create) the index at `storage_path/exploration_vectors.usearch`.
    ///
    /// # Errors
    ///
    /// Returns an error if the index file cannot be created or restored.
    pub fn open(storage_path: &Path, dims: usize) -> Result<Self> {
        let dims = if dims == 0 {
            DEFAULT_EXPLORATION_DIMS
        } else {
            dims
        };
        std::fs::create_dir_all(storage_path).context("failed to create storage dir")?;
        let index_path = storage_path.join(INDEX_FILE);

        let index = if index_path.exists() {
            let path_str = index_path.to_str().context("non-utf8 index path")?;
            Index::restore(path_str)
                .map_err(|e| anyhow::anyhow!("failed to restore exploration index: {e}"))?
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
                .map_err(|e| anyhow::anyhow!("failed to create exploration index: {e}"))?
        };

        Ok(Self { index, index_path })
    }

    /// Build a store rooted alongside the given [`Storage`]'s directory.
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

    /// Number of indexed maps.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.index.size() as u64
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.size() == 0
    }

    /// Insert or replace a vector keyed by `explorations.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector has the wrong dimensionality or the
    /// insertion fails.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<()> {
        let dims = self.index.dimensions();
        if vector.len() != dims {
            anyhow::bail!(
                "exploration vector dimension mismatch: expected {dims}, got {}",
                vector.len()
            );
        }
        // usearch does not upsert; remove any prior key first.
        let _ = self.index.remove(id);
        self.index
            .reserve(self.index.size() + 1)
            .map_err(|e| anyhow::anyhow!("failed to reserve exploration index capacity: {e}"))?;
        self.index
            .add(id, vector)
            .map_err(|e| anyhow::anyhow!("failed to add exploration vector {id}: {e}"))?;
        self.save()
    }

    /// Remove a vector by `explorations.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the save fails.
    pub fn delete(&self, id: u64) -> Result<()> {
        let _ = self.index.remove(id);
        self.save()
    }

    /// Search for the `limit` nearest maps, returning cosine similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the search fails.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<ExplorationNeighbor>> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let matches = self
            .index
            .search(query, limit)
            .map_err(|e| anyhow::anyhow!("exploration vector search failed: {e}"))?;
        let out = matches
            .keys
            .iter()
            .copied()
            .zip(matches.distances.iter().copied())
            .map(|(id, dist)| ExplorationNeighbor {
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
            .map_err(|e| anyhow::anyhow!("failed to save exploration index: {e}"))?;
        Ok(())
    }
}

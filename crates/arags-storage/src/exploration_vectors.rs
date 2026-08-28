//! Exploration-vector index for the explorations dataset (plan 022).
//!
//! A **dedicated** vector space (`exploration_vectors.usearch`) holding the
//! embedding of each map's `goal + summary` under cosine, separate from chunk
//! retrieval (`vector.usearch`) and question lookup
//! (`question_vectors.usearch`). Keys are the `explorations.id` rowids;
//! persistence is debounced via the shared [`VectorSpaceStore`] core.

use std::path::Path;

use anyhow::Result;

use crate::sqlite::conn::Storage;
use crate::vector_space::{FlushableVectorSpace, Neighbor, VectorSpaceStore};

const INDEX_FILE: &str = "exploration_vectors.usearch";

/// A nearest neighbour of an exploration query, with cosine similarity in
/// `[0, 1]`.
pub type ExplorationNeighbor = Neighbor;

/// `usearch` index of exploration-summary embeddings (cosine metric).
///
/// Thin facade over [`VectorSpaceStore`]; see its docs for the persistence
/// policy (debounced whole-file saves, [`ExplorationVectorStore::persist`] to
/// force a flush).
pub struct ExplorationVectorStore(VectorSpaceStore);

impl ExplorationVectorStore {
    /// Open (or create) the index at `storage_path/exploration_vectors.usearch`.
    ///
    /// # Errors
    ///
    /// Returns an error if the index file cannot be created or restored.
    pub fn open(storage_path: &Path, dims: usize) -> Result<Self> {
        Ok(Self(VectorSpaceStore::open(
            storage_path,
            INDEX_FILE,
            dims,
            true,
        )?))
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
        self.0.dimensions()
    }

    /// Number of indexed maps.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.0.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Insert or replace a vector keyed by `explorations.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector has the wrong dimensionality or the
    /// insertion fails.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<()> {
        self.0.insert(id, vector)
    }

    /// Remove a vector by `explorations.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the save fails.
    pub fn delete(&self, id: u64) -> Result<()> {
        self.0.delete(id)
    }

    /// Search for the `limit` nearest maps, returning cosine similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the search fails.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<ExplorationNeighbor>> {
        self.0.search(query, limit)
    }

    /// Whether there are unsaved (debounced) mutations.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    /// Force a whole-file save when there are unsaved mutations.
    ///
    /// # Errors
    ///
    /// Returns an error if the index file cannot be written.
    pub fn persist(&self) -> Result<()> {
        self.0.persist()
    }

    /// Drop every vector and rebuild an empty index (bootstrap rebuild,
    /// `agnostic-rag-rlm-tool-620d`).
    ///
    /// # Errors
    ///
    /// Returns an error if the fresh index cannot be created or saved.
    pub fn clear(&self) -> Result<()> {
        self.0.clear()
    }
}

impl FlushableVectorSpace for ExplorationVectorStore {
    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn persist(&self) -> Result<()> {
        self.0.persist()
    }
}

//! Question-vector index for the semantic query-answer cache (plan 017).
//!
//! A **dedicated** vector space (`question_vectors.usearch`) holding the
//! embedding of each cached *question* under cosine, so cache lookup by
//! question similarity never mixes with chunk retrieval. Keys are the
//! `qa_cache.id` rowids; persistence is debounced via the shared
//! [`VectorSpaceStore`] core.

use std::path::Path;

use anyhow::Result;

use crate::sqlite::conn::Storage;
use crate::vector_space::{FlushableVectorSpace, Neighbor, VectorSpaceStore};

const INDEX_FILE: &str = "question_vectors.usearch";

/// A nearest neighbour of a question query, with cosine similarity in `[0, 1]`.
pub type QuestionNeighbor = Neighbor;

/// `usearch` index of question embeddings (cosine metric).
///
/// Thin facade over [`VectorSpaceStore`]; see its docs for the persistence
/// policy (debounced whole-file saves, [`QuestionVectorStore::persist`] to
/// force a flush).
pub struct QuestionVectorStore(VectorSpaceStore);

impl QuestionVectorStore {
    /// Open (or create) the question index at
    /// `storage_path/question_vectors.usearch`.
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

    /// Number of indexed questions.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.0.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Insert or replace a question vector keyed by `qa_cache.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector has the wrong dimensionality or the
    /// insertion fails.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<()> {
        self.0.insert(id, vector)
    }

    /// Remove a question vector by `qa_cache.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the save fails.
    pub fn delete(&self, id: u64) -> Result<()> {
        self.0.delete(id)
    }

    /// Search for the `limit` nearest questions, returning cosine similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the search fails.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<QuestionNeighbor>> {
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
}

impl FlushableVectorSpace for QuestionVectorStore {
    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn persist(&self) -> Result<()> {
        self.0.persist()
    }
}

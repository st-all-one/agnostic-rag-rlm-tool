//! Question-vector index for the semantic query-answer cache (plan 017).
//!
//! This is a **dedicated** `usearch` index (`question_vectors`) in its own
//! vector space, separate from the chunk vector store (`vector.usearch`). It
//! holds the embedding of each cached *question* under the `cos` metric so that
//! cache lookup by question similarity never mixes with chunk retrieval.
//!
//! Keys are the `qa_cache.id` rowids (`u64`), keeping a 1:1 logical mapping to
//! `qa_cache`. The index is persisted next to the SQLite database and saved on
//! every mutation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::sqlite::conn::Storage;

const DEFAULT_QUESTION_DIMS: usize = 1024;
const INDEX_FILE: &str = "question_vectors.usearch";

/// A nearest neighbour of a question query, with cosine similarity in `[0, 1]`.
#[derive(Debug, Clone)]
pub struct QuestionNeighbor {
    /// `qa_cache.id` of the matching cached question.
    pub id: u64,
    /// Cosine similarity to the query (clamped to `[0, 1]`).
    pub similarity: f32,
}

/// `usearch` index of question embeddings (cosine metric).
pub struct QuestionVectorStore {
    index: Index,
    index_path: PathBuf,
}

impl QuestionVectorStore {
    /// Open (or create) the question index at `storage_path/question_vectors.usearch`.
    ///
    /// # Errors
    ///
    /// Returns an error if the index file cannot be created or restored.
    pub fn open(storage_path: &Path, dims: usize) -> Result<Self> {
        let dims = if dims == 0 {
            DEFAULT_QUESTION_DIMS
        } else {
            dims
        };
        std::fs::create_dir_all(storage_path).context("failed to create storage dir")?;
        let index_path = storage_path.join(INDEX_FILE);

        let index = if index_path.exists() {
            let path_str = index_path.to_str().context("non-utf8 index path")?;
            Index::restore(path_str)
                .map_err(|e| anyhow::anyhow!("failed to restore question index: {e}"))?
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
                .map_err(|e| anyhow::anyhow!("failed to create question index: {e}"))?
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

    /// Number of indexed questions.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.index.size() as u64
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.size() == 0
    }

    /// Insert or replace a question vector keyed by `qa_cache.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector has the wrong dimensionality or the
    /// insertion fails.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<()> {
        let dims = self.index.dimensions();
        if vector.len() != dims {
            anyhow::bail!(
                "question vector dimension mismatch: expected {dims}, got {}",
                vector.len()
            );
        }
        // usearch does not upsert; remove any prior key first.
        let _ = self.index.remove(id);
        self.index
            .reserve(self.index.size() + 1)
            .map_err(|e| anyhow::anyhow!("failed to reserve question index capacity: {e}"))?;
        self.index
            .add(id, vector)
            .map_err(|e| anyhow::anyhow!("failed to add question vector {id}: {e}"))?;
        self.save()
    }

    /// Remove a question vector by `qa_cache.id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the deletion or save fails.
    pub fn delete(&self, id: u64) -> Result<()> {
        let _ = self.index.remove(id);
        self.save()
    }

    /// Search for the `limit` nearest questions, returning cosine similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the search fails.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<QuestionNeighbor>> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let matches = self
            .index
            .search(query, limit)
            .map_err(|e| anyhow::anyhow!("question vector search failed: {e}"))?;
        let out = matches
            .keys
            .iter()
            .copied()
            .zip(matches.distances.iter().copied())
            .map(|(id, dist)| QuestionNeighbor {
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
            .map_err(|e| anyhow::anyhow!("failed to save question index: {e}"))?;
        Ok(())
    }
}

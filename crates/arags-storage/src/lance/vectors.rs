#![allow(clippy::unused_async)]
#![allow(clippy::unused_async_trait_impl)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Default embedding dimensionality for the stored vectors
/// (all-MiniLM-L6-v2 = 384). Callers with a different dimensionality must
/// pass it via [`VectorStore::open_with_dims`].
const DEFAULT_VECTOR_DIMS: usize = arags_core::EMBEDDING_DIMS;
const INDEX_FILE: &str = "vectors.usearch";
const META_FILE: &str = "vectors.meta";

/// Vector entry for insertion.
#[derive(Debug)]
pub struct VectorEntry {
    pub chunk_id: u64,
    pub buffer_id: u64,
    pub vector: Vec<f32>,
}

/// Search result with distance.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: u64,
    pub distance: f32,
}

/// Vector store backed by `usearch` (single-file HNSW, L2 metric).
///
/// `usearch` has no native metadata, so the `chunk_id -> buffer_id` mapping
/// required for buffer-scoped search is kept in an in-memory map and mirrored
/// to a side `vectors.meta` file next to the index.
pub struct VectorStore {
    index: Mutex<Index>,
    buffers: Mutex<HashMap<u64, u64>>,
    index_path: PathBuf,
    meta_path: PathBuf,
}

impl VectorStore {
    /// Open or create a vector store at the given directory using the default
    /// dimensionality (384, matching all-MiniLM-L6-v2).
    ///
    /// # Errors
    ///
    /// Returns an error if the `usearch` index cannot be created or restored,
    /// or if the side metadata file is present but unreadable.
    pub async fn open(path: &Path) -> Result<Self> {
        Self::open_with_dims(path, DEFAULT_VECTOR_DIMS).await
    }

    /// Open or create a vector store at the given directory with an explicit
    /// embedding dimensionality.
    ///
    /// # Errors
    ///
    /// Returns an error if the `usearch` index cannot be created or restored,
    /// or if the side metadata file is present but unreadable.
    pub async fn open_with_dims(path: &Path, dims: usize) -> Result<Self> {
        let start = std::time::Instant::now();
        std::fs::create_dir_all(path).context("failed to create vector store directory")?;

        let index_path = path.join(INDEX_FILE);
        let meta_path = path.join(META_FILE);

        let index = if index_path.exists() {
            let path_str = index_path.to_str().context("non-utf8 index path")?;
            info!(path = %index_path.display(), "restoring usearch index");
            Index::restore(path_str)
                .map_err(|e| anyhow::anyhow!("failed to restore vector index: {e}"))?
        } else {
            let opts = IndexOptions {
                dimensions: dims,
                metric: MetricKind::L2sq,
                quantization: ScalarKind::F32,
                connectivity: 0,
                expansion_add: 0,
                expansion_search: 0,
                ..Default::default()
            };
            info!(path = %index_path.display(), dims, "creating usearch index");
            Index::new(&opts).map_err(|e| anyhow::anyhow!("failed to create vector index: {e}"))?
        };

        let buffers = if meta_path.exists() {
            read_meta(&meta_path)?
        } else {
            HashMap::new()
        };

        info!(path = %index_path.display(), vectors = index.size(), duration_ms = %start.elapsed().as_millis(), "opened vector store");

        Ok(Self {
            index: Mutex::new(index),
            buffers: Mutex::new(buffers),
            index_path,
            meta_path,
        })
    }

    /// The embedding dimensionality of this store.
    #[must_use]
    pub fn dimensions(&self) -> usize {
        self.index.lock().dimensions()
    }

    /// Insert vectors into the store and persist the index + buffer mapping.
    ///
    /// # Errors
    ///
    /// Returns an error if a vector has the wrong dimensionality, the `usearch`
    /// add fails, or persisting the index/metadata fails.
    pub async fn insert_vectors(&self, entries: &[VectorEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let started = Instant::now();
        let dims = self.index.lock().dimensions();
        let mut buffers = self.buffers.lock();
        let index = self.index.lock();

        // usearch requires capacity to be reserved before insertions.
        let needed = index.size() + entries.len();
        index
            .reserve(needed)
            .map_err(|e| anyhow::anyhow!("failed to reserve vector capacity: {e}"))?;

        for entry in entries {
            if entry.vector.len() != dims {
                anyhow::bail!(
                    "vector dimension mismatch: expected {dims}, got {}",
                    entry.vector.len()
                );
            }
            // usearch has no native upsert: clear any prior key first so a
            // re-index over partially-committed data (same chunk_id reused after
            // a hash match) cannot trip "Duplicate keys not allowed".
            let _ = index.remove(entry.chunk_id);
            index
                .add(entry.chunk_id, &entry.vector)
                .map_err(|e| anyhow::anyhow!("failed to add vector {}: {e}", entry.chunk_id))?;
            buffers.insert(entry.chunk_id, entry.buffer_id);
        }

        drop(index);
        self.save_locked(&buffers)?;

        info!(
            inserted = entries.len(),
            total = self.index.lock().size(),
            elapsed_ms = started.elapsed().as_millis(),
            "inserted vectors"
        );
        Ok(())
    }

    /// Search for similar vectors and return structured results.
    ///
    /// When `buffer_id` is set, only vectors belonging to that buffer are
    /// considered (evaluated as a predicate during graph traversal).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying vector search fails.
    pub async fn search_similar(
        &self,
        query_vector: &[f32],
        buffer_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let raw = self.search(query_vector, buffer_id, limit).await?;
        Ok(raw
            .into_iter()
            .map(|(chunk_id, distance)| SearchResult { chunk_id, distance })
            .collect())
    }

    /// Core search, returning `(chunk_id, distance)` pairs.
    async fn search(
        &self,
        query_vector: &[f32],
        buffer_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<(u64, f32)>> {
        let started = Instant::now();

        let matches = match buffer_id {
            Some(bid) => {
                let buffers = self.buffers.lock();
                self.index
                    .lock()
                    .filtered_search(query_vector, limit, |key| buffers.get(&key) == Some(&bid))
                    .map_err(|e| anyhow::anyhow!("vector search failed: {e}"))?
            }
            None => self
                .index
                .lock()
                .search(query_vector, limit)
                .map_err(|e| anyhow::anyhow!("vector search failed: {e}"))?,
        };

        let results = matches
            .keys
            .iter()
            .copied()
            .zip(matches.distances.iter().copied())
            .collect::<Vec<_>>();

        debug!(
            limit,
            returned = results.len(),
            buffer_id,
            elapsed_ms = started.elapsed().as_millis(),
            "vector search"
        );

        Ok(results)
    }

    /// Return the number of vectors in the store.
    #[must_use]
    pub async fn count(&self) -> usize {
        self.index.lock().size()
    }

    /// Remove vectors for the given chunk ids from the index and the in-memory
    /// buffer map. Missing ids are ignored (idempotent), so this is safe to call
    /// with ids that were never embedded. Persists the index after removal.
    ///
    /// Used by the re-index stopgap (`agnostic-rag-rlm-tool-20cd`) to purge vectors
    /// for chunks deleted during a replace-style re-index, and by memory
    /// consolidation/decay (`agnostic-rag-rlm-tool-fa25`) to keep the usearch chunk
    /// space in sync with canonical SQLite after chunk rows are removed — which
    /// prevents the bootstrap count-divergence that forced a full rebuild on
    /// every restart.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot be saved.
    pub async fn delete_chunk_ids(&self, ids: &[u64]) -> Result<()> {
        self.delete_chunk_ids_sync(ids)
    }

    /// Synchronous variant of [`delete_chunk_ids`] for callers that must purge
    /// orphan vectors from a non-async context (e.g. memory consolidation, which
    /// runs while holding the SQLite write lock, or maintenance decay). The
    /// operation touches only this store's own locks and performs a single file
    /// save, so it is safe to invoke from a blocking worker.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot be saved.
    pub fn delete_chunk_ids_blocking(&self, ids: &[u64]) -> Result<()> {
        self.delete_chunk_ids_sync(ids)
    }

    fn delete_chunk_ids_sync(&self, ids: &[u64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        let started = Instant::now();
        let mut buffers = self.buffers.lock();
        let index = self.index.lock();
        let mut removed = 0usize;
        for &id in ids {
            match index.remove(id) {
                Ok(n) => removed += n,
                Err(e) => {
                    warn!(error = ?e, chunk_id = id, "failed to remove vector from index");
                }
            }
            buffers.remove(&id);
        }
        drop(index);
        self.save_locked(&buffers)?;

        info!(
            requested = ids.len(),
            removed,
            elapsed_ms = started.elapsed().as_millis(),
            "deleted vectors"
        );
        Ok(())
    }

    /// Persist the index and the buffer-mapping metadata.
    fn save_locked(&self, buffers: &HashMap<u64, u64>) -> Result<()> {
        let path_str = self.index_path.to_str().context("non-utf8 index path")?;
        self.index
            .lock()
            .save(path_str)
            .map_err(|e| anyhow::anyhow!("failed to save vector index: {e}"))?;
        write_meta(&self.meta_path, buffers)?;
        Ok(())
    }

    /// Drop every vector and rebuild an empty index with the same
    /// dimensionality/metric, clearing the buffer map too. Used by the server
    /// bootstrap rebuild (`agnostic-rag-rlm-tool-620d`) to reconstruct a divergent
    /// chunk space from canonical SQLite text. Persists the empty index.
    ///
    /// # Errors
    ///
    /// Returns an error if the fresh index cannot be created or saved.
    pub async fn clear(&self) -> Result<()> {
        let start = std::time::Instant::now();
        let dims = self.index.lock().dimensions();
        let opts = IndexOptions {
            dimensions: dims,
            metric: MetricKind::L2sq,
            quantization: ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            ..Default::default()
        };
        let fresh = Index::new(&opts)
            .map_err(|e| anyhow::anyhow!("failed to recreate vector index: {e}"))?;
        *self.index.lock() = fresh;
        self.buffers.lock().clear();
        self.save_locked(&self.buffers.lock())?;
        info!(duration_ms = %start.elapsed().as_millis(), "cleared vector store");
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Default)]
struct BufferMeta(HashMap<u64, u64>);

fn read_meta(path: &Path) -> Result<HashMap<u64, u64>> {
    let data = std::fs::read(path)
        .with_context(|| format!("failed to read vector metadata {}", path.display()))?;
    let meta: BufferMeta = serde_json::from_slice(&data).unwrap_or_default();
    Ok(meta.0)
}

fn write_meta(path: &Path, map: &HashMap<u64, u64>) -> Result<()> {
    let data = serde_json::to_vec(&BufferMeta(map.clone()))
        .context("failed to serialize vector metadata")?;
    std::fs::write(path, data)
        .with_context(|| format!("failed to write vector metadata {}", path.display()))?;
    Ok(())
}

//! Generic single-file `usearch` vector space shared by the dedicated
//! secondary indexes (QA questions, RLM summaries, exploration maps).
//!
//! Each space owns one HNSW index file persisted next to the SQLite database
//! and keyed by the matching table's rowid, keeping a 1:1 logical mapping.
//! This module centralises the previously duplicated open/upsert/search/save
//! logic and adds **debounced persistence**: mutations mark the index dirty
//! and flush at most once per [`SAVE_DEBOUNCE_MS`], so bursts of inserts
//! (bulk answers, RLM completions) amortise to a single O(N) file write while
//! keeping the worst-case loss window bounded. Call [`VectorSpaceStore::persist`]
//! to force a flush (server shutdown, maintenance).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tracing::{debug, warn};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// `usearch` index metric used by every secondary space (cosine).
const SPACE_METRIC: MetricKind = MetricKind::Cos;
/// `usearch` scalar precision used by every secondary space.
const SPACE_QUANT: ScalarKind = ScalarKind::F32;

/// Minimum interval between whole-file index saves under automatic
/// (debounced) persistence.
pub const SAVE_DEBOUNCE_MS: u64 = 2_000;

/// A nearest neighbour: rowid key + cosine similarity in `[0, 1]`.
#[derive(Debug, Clone)]
pub struct Neighbor {
    /// Rowid key used at insertion time.
    pub id: u64,
    /// Similarity clamped to `[0, 1]` (cosine spaces).
    pub similarity: f32,
}

/// Generic `usearch` index with debounced whole-file persistence.
pub struct VectorSpaceStore {
    index: Mutex<Index>,
    index_path: PathBuf,
    auto_persist: bool,
    dirty: AtomicBool,
    last_save: Mutex<Option<Instant>>,
}

impl VectorSpaceStore {
    /// Open (or create) an index file at `storage_path/<file_name>`.
    ///
    /// With `auto_persist` the index is flushed on mutation at most once per
    /// [`SAVE_DEBOUNCE_MS`]; otherwise only explicit [`VectorSpaceStore::persist`]
    /// calls write to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot be created or restored.
    pub fn open(
        storage_path: &Path,
        file_name: &str,
        dims: usize,
        auto_persist: bool,
    ) -> Result<Self> {
        std::fs::create_dir_all(storage_path).context("failed to create storage dir")?;
        let index_path = storage_path.join(file_name);

        let index = if index_path.exists() {
            let path_str = index_path.to_str().context("non-utf8 index path")?;
            Index::restore(path_str)
                .map_err(|e| anyhow::anyhow!("failed to restore index {file_name}: {e}"))?
        } else {
            let opts = IndexOptions {
                dimensions: dims,
                metric: SPACE_METRIC,
                quantization: SPACE_QUANT,
                connectivity: 0,
                expansion_add: 0,
                expansion_search: 0,
                ..Default::default()
            };
            Index::new(&opts)
                .map_err(|e| anyhow::anyhow!("failed to create index {file_name}: {e}"))?
        };

        Ok(Self {
            index: Mutex::new(index),
            index_path,
            auto_persist,
            dirty: AtomicBool::new(false),
            last_save: Mutex::new(None),
        })
    }

    /// The embedding dimensionality.
    #[must_use]
    pub fn dimensions(&self) -> usize {
        self.index.lock().dimensions()
    }

    /// Number of indexed vectors.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.index.lock().size() as u64
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.lock().size() == 0
    }

    /// Drop every vector and rebuild an empty index with the same
    /// dimensionality/metric. Used by the server bootstrap rebuild
    /// (`agnostic-rag-rlm-tool-620d`) to reconstruct a divergent space from canonical
    /// SQLite text. Persists the empty index so the on-disk file matches.
    ///
    /// # Errors
    ///
    /// Returns an error if the fresh index cannot be created or saved.
    pub fn clear(&self) -> Result<()> {
        let dims = self.index.lock().dimensions();
        let opts = IndexOptions {
            dimensions: dims,
            metric: SPACE_METRIC,
            quantization: SPACE_QUANT,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            ..Default::default()
        };
        let fresh =
            Index::new(&opts).map_err(|e| anyhow::anyhow!("failed to recreate index: {e}"))?;
        *self.index.lock() = fresh;
        self.dirty.store(true, Ordering::Relaxed);
        self.persist()?;
        Ok(())
    }

    /// Insert or replace a vector keyed by `id` (usearch has no upsert, so any
    /// prior key is removed first).
    ///
    /// # Errors
    ///
    /// Returns an error if the vector dimensionality mismatches or the
    /// insertion fails.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<()> {
        let dims = self.index.lock().dimensions();
        if vector.len() != dims {
            anyhow::bail!(
                "vector dimension mismatch: expected {dims}, got {}",
                vector.len()
            );
        }
        let index = self.index.lock();
        let _ = index.remove(id);
        index
            .reserve(index.size() + 1)
            .map_err(|e| anyhow::anyhow!("failed to reserve index capacity: {e}"))?;
        index
            .add(id, vector)
            .map_err(|e| anyhow::anyhow!("failed to add vector {id}: {e}"))?;
        drop(index);
        self.mark_dirty();
        Ok(())
    }

    /// Remove a vector by key. Missing keys are ignored.
    ///
    /// # Errors
    ///
    /// Returns an error only if the eventual save fails.
    pub fn delete(&self, id: u64) -> Result<()> {
        let _ = self.index.lock().remove(id);
        self.mark_dirty();
        Ok(())
    }

    /// Search for the `limit` nearest neighbours, returning similarity in
    /// `[0, 1]` for cosine spaces.
    ///
    /// # Errors
    ///
    /// Returns an error if the search fails.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<Neighbor>> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let matches = self
            .index
            .lock()
            .search(query, limit)
            .map_err(|e| anyhow::anyhow!("vector search failed: {e}"))?;
        Ok(matches
            .keys
            .iter()
            .copied()
            .zip(matches.distances.iter().copied())
            .map(|(id, dist)| {
                // Cosine distance (1 - cos) → similarity.
                Neighbor {
                    id,
                    similarity: (1.0 - dist).clamp(0.0, 1.0),
                }
            })
            .collect())
    }

    /// Force a whole-file save when the index has unsaved mutations.
    ///
    /// # Errors
    ///
    /// Returns an error if the index file cannot be written.
    pub fn persist(&self) -> Result<()> {
        if !self.dirty.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.do_save()?;
        self.dirty.store(false, Ordering::Relaxed);
        *self.last_save.lock() = Some(Instant::now());
        Ok(())
    }

    /// Whether there are unsaved mutations.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        if !self.auto_persist {
            return;
        }
        let due = match *self.last_save.lock() {
            Some(last) => last.elapsed() >= Duration::from_millis(SAVE_DEBOUNCE_MS),
            None => true,
        };
        if due {
            // Debounced save failures must stay visible: the dirty flag keeps
            // the data eligible for the next flush, but silence here would
            // hide an unwritable index until shutdown.
            if let Err(e) = self.persist() {
                warn!(
                    error = %e,
                    index = %self.index_path.display(),
                    "debounced vector index save failed"
                );
            }
        }
    }

    fn do_save(&self) -> Result<()> {
        let start = std::time::Instant::now();
        let path_str = self.index_path.to_str().context("non-utf8 index path")?;
        self.index
            .lock()
            .save(path_str)
            .map_err(|e| anyhow::anyhow!("failed to save {}: {e}", self.index_path.display()))?;
        debug!(
            index = %self.index_path.display(),
            duration_ms = %start.elapsed().as_millis(),
            "vector index saved"
        );
        Ok(())
    }
}

/// Persistence surface shared by the dedicated secondary spaces, letting
/// callers flush heterogeneous stores uniformly.
pub trait FlushableVectorSpace {
    /// Whether there are unsaved (debounced) mutations.
    fn is_dirty(&self) -> bool;

    /// Force a whole-file save when there are unsaved mutations.
    ///
    /// # Errors
    ///
    /// Returns an error if the index file cannot be written.
    fn persist(&self) -> Result<()>;
}

#[cfg(test)]
mod testing;

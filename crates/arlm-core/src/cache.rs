use std::collections::HashMap;

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::types::now_ms;

/// Cache entry with expiration and optional dependency tracking.
#[derive(Debug, Clone)]
struct CacheEntry {
    result: String,
    created_at_ms: u64,
    /// Optional dependency key this entry is bound to (e.g. a config/file set id).
    dep_key: Option<String>,
    /// Optional dependency version/hash; the entry is invalid when this changes.
    dep_version: Option<String>,
}

/// Result cache for deduplicating identical subtask resolutions.
///
/// Supports TTL + LRU eviction (default) and, additionally, dependency-based
/// invalidation (#10): entries may be stored with a `dep_key`/`dep_version` so that
/// [`ResultCache::get_dep`] returns a miss when the dependency version no longer matches,
/// and [`ResultCache::invalidate_dep`] can drop every entry bound to a dependency.
#[derive(Debug)]
pub struct ResultCache {
    inner: RwLock<HashMap<String, CacheEntry>>,
    max_entries: usize,
    ttl_ms: u64,
}

impl ResultCache {
    /// Create a new cache with a maximum number of entries and TTL in milliseconds.
    #[must_use]
    pub fn new(max_entries: usize, ttl_ms: u64) -> Self {
        Self {
            inner: RwLock::new(HashMap::with_capacity(max_entries)),
            max_entries,
            ttl_ms,
        }
    }

    /// Create a default cache (2048 entries, 1 hour TTL).
    #[must_use]
    pub fn default_config() -> Self {
        Self::new(2048, 3_600_000)
    }

    /// Compute a stable hash for a task string.
    #[must_use]
    pub fn task_hash(task: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(task.as_bytes());
        let hash = hasher.finalize();
        hex::encode(hash)
    }

    /// Look up a cached result by task (dependency tracking is ignored here).
    #[must_use]
    pub fn get(&self, task: &str, _project: &str) -> Option<String> {
        let hash = Self::task_hash(task);
        let entries = self.inner.read();
        let now = now_ms();
        entries.get(&hash).and_then(|entry| {
            if now.saturating_sub(entry.created_at_ms) > self.ttl_ms {
                None
            } else {
                Some(entry.result.clone())
            }
        })
    }

    /// Look up a cached result, but only if its stored dependency key + version match.
    ///
    /// Returns `None` (a cache miss) when no entry exists, it is expired, or its
    /// `dep_key`/`dep_version` differ from the supplied values — signalling that the
    /// underlying inputs changed and the result must be recomputed.
    #[must_use]
    pub fn get_dep(
        &self,
        task: &str,
        _project: &str,
        dep_key: &str,
        dep_version: &str,
    ) -> Option<String> {
        let hash = Self::task_hash(task);
        let entries = self.inner.read();
        let now = now_ms();
        entries.get(&hash).and_then(|entry| {
            if now.saturating_sub(entry.created_at_ms) > self.ttl_ms {
                return None;
            }
            if entry.dep_key.as_deref() != Some(dep_key) {
                return None;
            }
            if entry.dep_version.as_deref() != Some(dep_version) {
                return None;
            }
            Some(entry.result.clone())
        })
    }

    /// Store a result in the cache (no dependency binding).
    pub fn put(&self, task: &str, project: &str, result: &str) {
        self.put_dep(task, project, result, None, None);
    }

    /// Store a result bound to a dependency key + version for invalidation (#10).
    pub fn put_dep(
        &self,
        task: &str,
        _project: &str,
        result: &str,
        dep_key: Option<&str>,
        dep_version: Option<&str>,
    ) {
        let hash = Self::task_hash(task);
        let entry = CacheEntry {
            result: result.to_string(),
            created_at_ms: now_ms(),
            dep_key: dep_key.map(String::from),
            dep_version: dep_version.map(String::from),
        };
        let mut entries = self.inner.write();

        // Evict expired entries
        let now = now_ms();
        entries.retain(|_, e| now.saturating_sub(e.created_at_ms) <= self.ttl_ms);

        // Evict oldest if at capacity (simple: clear half)
        if entries.len() >= self.max_entries {
            let to_remove = entries.len() / 2;
            let mut keys: Vec<String> = entries.keys().cloned().collect();
            keys.sort_by_key(|k| entries.get(k).map_or(0, |e| e.created_at_ms));
            for key in keys.into_iter().take(to_remove) {
                entries.remove(&key);
            }
        }

        entries.insert(hash, entry);
    }

    /// Invalidate every entry bound to the given dependency key.
    pub fn invalidate_dep(&self, dep_key: &str) {
        let mut entries = self.inner.write();
        entries.retain(|_, e| e.dep_key.as_deref() != Some(dep_key));
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.inner.write().clear();
    }

    /// Get the number of entries currently in the cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Check if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

impl Default for ResultCache {
    fn default() -> Self {
        Self::default_config()
    }
}

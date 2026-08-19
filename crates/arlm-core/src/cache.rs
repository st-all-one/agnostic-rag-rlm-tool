use std::collections::HashMap;

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::types::now_ms;

/// Cache entry with expiration.
#[derive(Debug, Clone)]
struct CacheEntry {
    result: String,
    created_at_ms: u64,
}

/// Result cache for deduplicating identical subtask resolutions.
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

    /// Look up a cached result by task.
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

    /// Store a result in the cache.
    pub fn put(&self, task: &str, _project: &str, result: &str) {
        let hash = Self::task_hash(task);
        let entry = CacheEntry {
            result: result.to_string(),
            created_at_ms: now_ms(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_cache_put_and_get() {
        let cache = ResultCache::new(10, 60_000);
        assert!(cache.is_empty());

        cache.put("task A", "proj", "result A");
        assert_eq!(cache.len(), 1);

        let got = cache.get("task A", "proj");
        assert_eq!(got.as_deref(), Some("result A"));
    }

    #[test]
    fn test_result_cache_miss() {
        let cache = ResultCache::new(10, 60_000);
        assert!(cache.get("nonexistent", "proj").is_none());
    }

    #[test]
    fn test_task_hash_deterministic() {
        let h1 = ResultCache::task_hash("hello world");
        let h2 = ResultCache::task_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_task_hash_different() {
        let h1 = ResultCache::task_hash("task A");
        let h2 = ResultCache::task_hash("task B");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_result_cache_clear() {
        let cache = ResultCache::new(10, 60_000);
        cache.put("task", "proj", "result");
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_result_cache_eviction() {
        let cache = ResultCache::new(3, 60_000);
        cache.put("t1", "p", "r1");
        cache.put("t2", "p", "r2");
        cache.put("t3", "p", "r3");
        cache.put("t4", "p", "r4"); // should trigger eviction
        // After eviction, should have removed some entries
        assert!(cache.len() <= 3);
    }

    #[test]
    fn test_result_cache_overwrite() {
        let cache = ResultCache::new(10, 60_000);
        cache.put("task", "proj", "v1");
        cache.put("task", "proj", "v2");
        let got = cache.get("task", "proj");
        assert_eq!(got.as_deref(), Some("v2"));
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_core::ResultCache;

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
    cache.put("t4", "p", "r4");
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

// --- Dependency invalidation (#10) ---

#[test]
fn test_dep_put_and_get_match() {
    let cache = ResultCache::new(10, 60_000);
    cache.put_dep("task", "proj", "result", Some("files"), Some("v1"));
    let got = cache.get_dep("task", "proj", "files", "v1");
    assert_eq!(got.as_deref(), Some("result"));
}

#[test]
fn test_dep_get_mismatch_version_misses() {
    let cache = ResultCache::new(10, 60_000);
    cache.put_dep("task", "proj", "result", Some("files"), Some("v1"));
    assert!(cache.get_dep("task", "proj", "files", "v2").is_none());
}

#[test]
fn test_dep_get_mismatch_key_misses() {
    let cache = ResultCache::new(10, 60_000);
    cache.put_dep("task", "proj", "result", Some("files"), Some("v1"));
    assert!(cache.get_dep("task", "proj", "other", "v1").is_none());
}

#[test]
fn test_dep_invalidate_drops_entries() {
    let cache = ResultCache::new(10, 60_000);
    cache.put_dep("t1", "p", "r1", Some("files"), Some("v1"));
    cache.put_dep("t2", "p", "r2", Some("files"), Some("v1"));
    cache.put_dep("t3", "p", "r3", Some("other"), Some("v1"));
    cache.invalidate_dep("files");
    assert!(cache.get("t1", "p").is_none());
    assert!(cache.get("t2", "p").is_none());
    assert_eq!(cache.get("t3", "p").as_deref(), Some("r3"));
}

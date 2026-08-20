#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp
)]

use arlm_storage::Storage;
use tempfile::TempDir;

#[test]
fn test_get_returns_none_when_empty() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let result = storage.get_cached_result("abc", "proj").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_put_and_get() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    storage
        .put_cached_result("hash1", "proj1", "{\"results\":[]}")
        .unwrap();
    let result = storage.get_cached_result("hash1", "proj1").unwrap();
    assert_eq!(result.as_deref(), Some("{\"results\":[]}"));
}

#[test]
fn test_put_overwrites_existing() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    storage.put_cached_result("h", "p", "first").unwrap();
    storage.put_cached_result("h", "p", "second").unwrap();
    let result = storage.get_cached_result("h", "p").unwrap();
    assert_eq!(result.as_deref(), Some("second"));
}

#[test]
fn test_different_projects_are_independent() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    storage.put_cached_result("h", "p1", "r1").unwrap();
    storage.put_cached_result("h", "p2", "r2").unwrap();
    assert_eq!(
        storage.get_cached_result("h", "p1").unwrap().as_deref(),
        Some("r1")
    );
    assert_eq!(
        storage.get_cached_result("h", "p2").unwrap().as_deref(),
        Some("r2")
    );
}

#[test]
fn test_invalidate_project() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    storage.put_cached_result("h1", "proj", "r1").unwrap();
    storage.put_cached_result("h2", "proj", "r2").unwrap();
    storage.put_cached_result("h3", "other", "r3").unwrap();
    let deleted = storage.invalidate_project_cache("proj").unwrap();
    assert_eq!(deleted, 2);
    assert!(storage.get_cached_result("h1", "proj").unwrap().is_none());
    assert_eq!(
        storage.get_cached_result("h3", "other").unwrap().as_deref(),
        Some("r3")
    );
}

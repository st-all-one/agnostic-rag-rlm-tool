//! Unit tests for the watch daemon's change-detection primitives.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn file_state_is_stable_for_unchanged_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "hello").unwrap();

    let s1 = file_state(&path);
    let s2 = file_state(&path);
    assert_eq!(s1, s2, "re-reading an untouched file must not change state");
    assert_ne!(s1, (0, 0), "real metadata must not be the zeroed fallback");
    assert_eq!(s1.1, 5, "size component is the byte length");
}

#[test]
fn file_state_changes_when_content_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.txt");
    std::fs::write(&path, "v1").unwrap();
    let before = file_state(&path);
    // Ensure a distinct mtime even on coarse filesystems.
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::write(&path, "v2 with different length").unwrap();
    let after = file_state(&path);
    assert_ne!(before, after);
}

#[test]
fn file_state_zeroes_for_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(file_state(&dir.path().join("nope")), (0, 0));
}

#[test]
fn snapshot_covers_discovered_files_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("keep.rs"), "x").unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target/skip.rs"), "y").unwrap();

    let snap = snapshot_state(root, &[], &[]);
    assert_eq!(snap.len(), 1, "default-ignored dirs are not snapshotted");
    assert!(snap.contains_key("keep.rs"));
}

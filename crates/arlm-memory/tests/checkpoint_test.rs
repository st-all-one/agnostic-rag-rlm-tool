#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::all, clippy::pedantic, clippy::nursery)]

use arlm_memory::checkpoint::{CHECKPOINT_DIR, CheckpointManager, parse_checkpoint_name};
use std::fs;
use tempfile::TempDir;

fn setup() -> (CheckpointManager, TempDir) {
    let tmp = TempDir::new().unwrap();
    let wiki_root = tmp.path().join("wiki");
    fs::create_dir_all(&wiki_root).unwrap();

    // Create some wiki content
    let searches = wiki_root.join("searches");
    fs::create_dir_all(&searches).unwrap();
    fs::write(searches.join("test.md"), "# Test Search").unwrap();

    let analyses = wiki_root.join("analyses");
    fs::create_dir_all(&analyses).unwrap();
    fs::write(analyses.join("001-analysis.md"), "# Analysis").unwrap();

    let manager = CheckpointManager::new(&wiki_root).unwrap();
    (manager, tmp)
}

#[test]
fn test_new_creates_checkpoint_dir() {
    let tmp = TempDir::new().unwrap();
    let wiki_root = tmp.path().join("wiki");
    fs::create_dir_all(&wiki_root).unwrap();

    let manager = CheckpointManager::new(&wiki_root).unwrap();
    assert!(manager.checkpoint_root().is_dir());
    assert_eq!(manager.checkpoint_root(), wiki_root.join(CHECKPOINT_DIR));
}

#[test]
fn test_create_checkpoint() {
    let (manager, _tmp) = setup();

    let path = manager.create_checkpoint("before-edit").unwrap();
    assert!(path.is_dir());
    assert!(path.join("searches").is_dir());
    assert!(path.join("searches/test.md").is_file());
    assert!(path.join("analyses").is_dir());
    assert!(path.join("analyses/001-analysis.md").is_file());

    let content = fs::read_to_string(path.join("searches/test.md")).unwrap();
    assert_eq!(content, "# Test Search");
}

#[test]
fn test_list_checkpoints_empty() {
    let (manager, _tmp) = setup();
    let list = manager.list_checkpoints().unwrap();
    assert!(list.is_empty());
}

#[test]
fn test_list_checkpoints() {
    let (manager, _tmp) = setup();

    manager.create_checkpoint("alpha").unwrap();
    manager.create_checkpoint("beta").unwrap();

    let list = manager.list_checkpoints().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|c| c.name == "alpha"));
    assert!(list.iter().any(|c| c.name == "beta"));
}

#[test]
fn test_list_checkpoints_sorted_by_timestamp() {
    let (manager, _tmp) = setup();

    manager.create_checkpoint("first").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    manager.create_checkpoint("second").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    manager.create_checkpoint("third").unwrap();

    let list = manager.list_checkpoints().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "first");
    assert!(list[0].timestamp <= list[1].timestamp);
    assert_eq!(list[1].name, "second");
    assert!(list[1].timestamp <= list[2].timestamp);
    assert_eq!(list[2].name, "third");
}

#[test]
fn test_checkpoint_info_size() {
    let (manager, _tmp) = setup();

    manager.create_checkpoint("sized").unwrap();
    let list = manager.list_checkpoints().unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].size > 0);
}

#[test]
fn test_restore_checkpoint() {
    let (manager, _tmp) = setup();

    manager.create_checkpoint("backup").unwrap();

    // Modify wiki
    let searches = manager.wiki_root().join("searches");
    fs::write(searches.join("new.md"), "# New Page").unwrap();
    fs::remove_file(searches.join("test.md")).unwrap();

    assert!(searches.join("new.md").is_file());
    assert!(!searches.join("test.md").is_file());

    // Restore
    manager.restore_checkpoint("backup").unwrap();

    assert!(!searches.join("new.md").is_file());
    assert!(searches.join("test.md").is_file());
    let content = fs::read_to_string(searches.join("test.md")).unwrap();
    assert_eq!(content, "# Test Search");
}

#[test]
fn test_restore_preserves_checkpoints_dir() {
    let (manager, _tmp) = setup();

    manager.create_checkpoint("snap").unwrap();
    manager.restore_checkpoint("snap").unwrap();

    // .checkpoints should still exist and the checkpoint should remain
    let list = manager.list_checkpoints().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "snap");
}

#[test]
fn test_delete_checkpoint() {
    let (manager, _tmp) = setup();

    manager.create_checkpoint("to-delete").unwrap();
    assert_eq!(manager.list_checkpoints().unwrap().len(), 1);

    manager.delete_checkpoint("to-delete").unwrap();
    assert_eq!(manager.list_checkpoints().unwrap().len(), 0);
}

#[test]
fn test_delete_nonexistent_checkpoint() {
    let (manager, _tmp) = setup();
    let result = manager.delete_checkpoint("nope");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_restore_nonexistent_checkpoint() {
    let (manager, _tmp) = setup();
    let result = manager.restore_checkpoint("nope");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_same_name_multiple_checkpoints() {
    let (manager, _tmp) = setup();

    manager.create_checkpoint("my-snap").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    manager.create_checkpoint("my-snap").unwrap();

    let list = manager.list_checkpoints().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|c| c.name == "my-snap"));

    // Restore should use the first (oldest) match
    manager.restore_checkpoint("my-snap").unwrap();
}

#[test]
fn test_create_checkpoint_empty_wiki() {
    let tmp = TempDir::new().unwrap();
    let wiki_root = tmp.path().join("empty-wiki");
    fs::create_dir_all(&wiki_root).unwrap();

    let manager = CheckpointManager::new(&wiki_root).unwrap();
    let path = manager.create_checkpoint("empty").unwrap();
    assert!(path.is_dir());
    assert!(manager.list_checkpoints().unwrap().len() == 1);
}

#[test]
fn test_parse_checkpoint_name_valid() {
    let (name, ts) = parse_checkpoint_name("my-snap_20240115T103000Z").unwrap();
    assert_eq!(name, "my-snap");
    assert_eq!(ts, "20240115T103000Z");
}

#[test]
fn test_parse_checkpoint_name_invalid() {
    assert!(parse_checkpoint_name("no-timestamp-here").is_none());
    assert!(parse_checkpoint_name("short_2024").is_none());
}

#[test]
fn test_checkpoint_nested_dirs() {
    let (manager, _tmp) = setup();

    let rules = manager.wiki_root().join("rules");
    fs::create_dir_all(rules.join("subdir")).unwrap();
    fs::write(rules.join("rule.md"), "# Rule").unwrap();
    fs::write(rules.join("subdir/nested.md"), "# Nested").unwrap();

    let path = manager.create_checkpoint("nested").unwrap();
    assert!(path.join("rules/rule.md").is_file());
    assert!(path.join("rules/subdir/nested.md").is_file());

    manager.restore_checkpoint("nested").unwrap();
    assert!(rules.join("rule.md").is_file());
    assert!(rules.join("subdir/nested.md").is_file());
}

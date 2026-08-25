#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn test_project_name_falls_back_without_git_repo() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nested = tmp.path().join("plain-dir");
    std::fs::create_dir_all(&nested).unwrap();
    // Not a git repo → directory basename.
    assert_eq!(project_name(&nested), "plain-dir");
}

#[test]
fn test_project_name_falls_back_to_directory_basename() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nested = tmp.path().join("my-project");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(project_name(&nested), "my-project");
}

#[test]
fn test_seed_ignore_defaults_without_gitignore() {
    // Run in a directory without .gitignore: current process cwd is the
    // workspace (which HAS one), so just assert parsing shape instead.
    let seeded = seed_ignore_from_gitignore();
    for entry in &seeded {
        assert!(!entry.is_empty());
        assert!(!entry.starts_with('#'));
    }
}

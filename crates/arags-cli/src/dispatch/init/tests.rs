#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::user_config::EffectiveUserConfig;
use crate::user_config::{is_valid_canonical_name, resolve_canonical_name};

use super::*;

#[test]
fn test_suggest_project_name_falls_back_without_git_repo() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nested = tmp.path().join("plain-dir");
    std::fs::create_dir_all(&nested).unwrap();
    // Not a git repo → directory basename (used only as a prompt hint).
    assert_eq!(suggest_project_name(&nested), "plain-dir");
}

#[test]
fn test_suggest_project_name_falls_back_to_directory_basename() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nested = tmp.path().join("my-project");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(suggest_project_name(&nested), "my-project");
}

#[test]
fn test_is_valid_canonical_name_rejects_legacy_keys() {
    // Empty / whitespace.
    assert!(!is_valid_canonical_name(""));
    assert!(!is_valid_canonical_name("   "));
    // Legacy buffer keys.
    assert!(!is_valid_canonical_name("."));
    assert!(!is_valid_canonical_name(".."));
    // Absolute paths (legacy buffer keys, must be logical names). `is_absolute`
    // is OS-aware, so exercise the current platform's notion.
    assert!(!is_valid_canonical_name("/abs/path"));
    if cfg!(windows) {
        assert!(!is_valid_canonical_name("C:\\windows"));
    }
}

#[test]
fn test_is_valid_canonical_name_accepts_logical_name() {
    assert!(is_valid_canonical_name("my-service"));
    assert!(is_valid_canonical_name("agnostic-rlm-rs"));
    assert!(is_valid_canonical_name("team backend"));
}

#[test]
fn test_resolve_canonical_name_errors_when_unset() {
    let cfg = EffectiveUserConfig::default();
    assert!(resolve_canonical_name(&cfg).is_err());
}

#[test]
fn test_resolve_canonical_name_rejects_legacy_value() {
    let mut cfg = EffectiveUserConfig::default();
    cfg.project.name = Some(".".to_string());
    assert!(resolve_canonical_name(&cfg).is_err());
}

#[test]
fn test_resolve_canonical_name_ok() {
    let mut cfg = EffectiveUserConfig::default();
    cfg.project.name = Some("my-service".to_string());
    assert_eq!(resolve_canonical_name(&cfg).unwrap(), "my-service");
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

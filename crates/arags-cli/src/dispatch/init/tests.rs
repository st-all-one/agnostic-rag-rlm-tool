#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::output::Format;
use crate::user_config::{
    EffectiveUserConfig, LocalConfig, ProjectSection, ServerSection, is_valid_canonical_name,
    load_local_at,
};

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
fn test_seed_ignore_defaults_without_gitignore() {
    let tmp = tempfile::TempDir::new().unwrap();
    // No `.gitignore` present → fixed default seed.
    let seeded = seed_ignore_from_gitignore(tmp.path());
    assert!(seeded.contains(&".git/".to_string()));
    assert!(seeded.contains(&"target/".to_string()));
}

#[test]
fn init_requires_name_in_non_interactive_mode() {
    let tmp = tempfile::TempDir::new().unwrap();
    // `--non-interactive` without `--name` must fail cleanly (no prompt).
    let err = resolve_init_name(None, tmp.path(), true);
    assert!(err.is_err());
    assert!(
        err.unwrap_err()
            .to_string()
            .contains("canonical project name required")
    );
}

#[test]
fn init_writes_canonical_arags_toml() {
    let tmp = tempfile::TempDir::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cfg = EffectiveUserConfig::default();
    let flags = InitFlags {
        name: Some("my-service".to_string()),
        ignore: vec!["*.o".to_string(), "build/".to_string()],
        server_addr: Some("192.168.1.10:50051".to_string()),
        register: false,
        do_index: false,
        non_interactive: true,
    };
    run_init(&rt, &cfg, tmp.path(), &flags, Format::Path).unwrap();

    let path = tmp.path().join(".arags.toml");
    assert!(path.exists(), "expected .arags.toml to be written");
    let parsed = load_local_at(&path).unwrap();
    let project = parsed.project.expect("project section present");
    assert_eq!(project.name.as_deref(), Some("my-service"));
    let ignore = project.ignore.expect("ignore present");
    assert!(ignore.contains(&"*.o".to_string()));
    assert!(ignore.contains(&"build/".to_string()));
    let server = parsed.server.expect("server section present");
    assert_eq!(server.addr.as_deref(), Some("192.168.1.10:50051"));
}

#[test]
fn init_is_idempotent_opens_edit_with_current_values() {
    let existing = LocalConfig {
        llm: None,
        server: Some(ServerSection {
            addr: Some("10.0.0.1:50051".to_string()),
            ..ServerSection::default()
        }),
        project: Some(ProjectSection {
            name: Some("existing-name".to_string()),
            ignore: Some(vec!["foo/".to_string()]),
        }),
        watch: None,
    };
    // Re-init without overrides must prefill the existing values.
    let write = build_init_write(
        existing
            .project
            .as_ref()
            .and_then(|p| p.name.clone())
            .as_deref()
            .unwrap_or("existing-name"),
        existing
            .project
            .as_ref()
            .and_then(|p| p.ignore.clone())
            .as_deref()
            .unwrap_or(&[]),
        existing
            .server
            .as_ref()
            .and_then(|s| s.addr.clone())
            .as_deref(),
        &existing,
    );
    assert_eq!(write.project.name.as_deref(), Some("existing-name"));
    let ignore = write.project.ignore.expect("ignore carried over");
    assert_eq!(ignore, vec!["foo/".to_string()]);
    assert_eq!(
        write
            .server
            .as_ref()
            .and_then(|s| s.addr.clone())
            .as_deref(),
        Some("10.0.0.1:50051")
    );
}

#[test]
fn init_ignore_patterns_seeded_from_gitignore() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join(".gitignore"),
        "# comment\n*.log\n/dist/\n\nnode_modules/\n",
    )
    .unwrap();
    let seeded = seed_ignore_from_gitignore(tmp.path());
    assert!(seeded.contains(&"*.log".to_string()));
    assert!(seeded.contains(&"/dist/".to_string()));
    assert!(seeded.contains(&"node_modules/".to_string()));
    assert!(!seeded.iter().any(|s| s.starts_with('#')));
}

#[test]
fn init_reinit_preserves_existing_registration() {
    let tmp = tempfile::TempDir::new().unwrap();
    let local = tmp.path().join(".arags.toml");
    // Pre-seed an existing init with a watch registration.
    std::fs::write(
        &local,
        "[project]\nname = \"keep-me\"\nignore = [\"a/\"]\n\n[watch]\nenabled = true\nproject = \"keep-me\"\n",
    )
    .unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let cfg = EffectiveUserConfig::default();
    // Re-init with no overrides: must prefill from the existing config and
    // preserve the `[watch]` registration (idempotent edit mode).
    let flags = InitFlags {
        name: None,
        ignore: vec![],
        server_addr: None,
        register: false,
        do_index: false,
        non_interactive: true,
    };
    run_init(&rt, &cfg, tmp.path(), &flags, Format::Path).unwrap();

    let parsed = load_local_at(&local).unwrap();
    assert_eq!(
        parsed
            .project
            .as_ref()
            .and_then(|p| p.name.clone())
            .as_deref(),
        Some("keep-me")
    );
    assert_eq!(
        parsed.watch.as_ref().map(|w| w.enabled),
        Some(true),
        "watch registration must survive re-init"
    );
}

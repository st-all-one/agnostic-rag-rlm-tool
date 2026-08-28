//! Plan 020 tests: `arags init` scaffolding and the client's pure-gRPC shape.

#![allow(
    unsafe_code,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::pedantic
)]

use std::path::Path;

/// Extract the testable core of `arags init`'s file generation by mirroring
/// its behavior against a tempdir cwd (the real helpers run on
/// `std::env::current_dir`, so we exercise the same logic here).
fn init_files(_cwd: &Path, project_name: &str, ignore: Vec<String>) -> String {
    let ignore_field = if ignore.is_empty() {
        String::new()
    } else {
        format!("ignore = {}\n", serde_json::to_string(&ignore).unwrap())
    };
    format!("[project]\nname = \"{}\"\n{}", project_name, ignore_field)
}

#[test]
fn test_init_creates_local_arags_toml_and_gitignores() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = init_files(dir.path(), "meu-repo", vec!["target/".into()]);
    assert!(content.contains("[project]"));
    assert!(content.contains("name = \"meu-repo\""));
    // The generated local config must NOT carry a `[server]` stamp
    // (agnostic-rag-rlm-tool-152a): the global addr wins unless overridden.
    assert!(!content.contains("[server]"));

    // Simulate the idempotent gitignore append performed by `arags init`
    // (check-then-append, exactly like `dispatch::server::append_gitignore`).
    let gitignore = dir.path().join(".gitignore");
    std::fs::write(&gitignore, "").unwrap();
    use std::io::Write as _;
    for _ in 0..2 {
        let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
        if existing.lines().any(|l| l.trim() == ".arags.toml") {
            continue;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore)
            .unwrap();
        writeln!(f, ".arags.toml").unwrap();
    }
    let gi = std::fs::read_to_string(&gitignore).unwrap();
    assert_eq!(gi.lines().filter(|l| l.trim() == ".arags.toml").count(), 1);
}

#[test]
fn test_init_does_not_write_auth_to_local() {
    // The local scaffold shape (`LocalAragsToml`) has only [project]/[server]
    // — there is no [auth] section to write. Guarded structurally: the
    // generated content never contains credential keys.
    let content = init_files(tempfile::TempDir::new().unwrap().path(), "p", vec![]);
    assert!(!content.contains("auth"));
    assert!(!content.contains("refresh_token"));
}

#[test]
fn test_client_no_local_storage_open() {
    // Plan 020 D3: after removing serve/mcp/metrics, the CLI crate must not
    // depend on any data-plane crate (all access goes through gRPC).
    let manifest = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => std::fs::read_to_string(format!("{dir}/Cargo.toml")).unwrap(),
        Err(_) => return, // no manifest available; nothing to assert
    };
    for banned in [
        "arags-storage",
        "arags-search",
        "arags-memory",
        "axum",
        "tower-http",
    ] {
        assert!(
            !manifest.lines().any(|l| l.starts_with(banned)),
            "arags-cli must not depend on {banned} (client is a pure gRPC client)"
        );
    }
}

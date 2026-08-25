#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn test_discovery_ignores_dotfiles_and_gitignore_rules() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::write(root.join(".hidden/file.rs"), "x").unwrap();
    std::fs::write(root.join(".env"), "SECRET=1").unwrap();
    std::fs::create_dir_all(root.join(".git/objects")).unwrap();
    std::fs::write(root.join(".git/config"), "[core]").unwrap();
    std::fs::write(root.join("debug.log"), "noise").unwrap();
    std::fs::write(root.join("keep.txt"), "real").unwrap();
    std::fs::write(root.join(".gitignore"), "*.log\n!keep.log\n").unwrap();

    // Nested .gitignore scoped to its directory.
    std::fs::create_dir_all(root.join("sub/pkg")).unwrap();
    std::fs::write(root.join("sub/pkg/.gitignore"), "cache/\n").unwrap();
    std::fs::create_dir_all(root.join("sub/pkg/cache")).unwrap();
    std::fs::write(root.join("sub/pkg/cache/junk.rs"), "j").unwrap();
    std::fs::write(root.join("sub/pkg/src.rs"), "s").unwrap();

    let files = discover_files(root, &[], &[]).unwrap();
    let rels: Vec<String> = files
        .iter()
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().to_string())
        .collect();

    assert!(rels.contains(&"main.rs".to_string()));
    assert!(rels.contains(&"keep.txt".to_string()));
    assert!(rels.contains(&"sub/pkg/src.rs".to_string()));

    assert!(
        !rels
            .iter()
            .any(|r| r.split('/').any(|seg| seg.starts_with('.'))),
        "no dot-paths expected, got {rels:?}"
    );
    assert!(!rels.contains(&"debug.log".to_string()), "{rels:?}");
    assert!(!rels.contains(&"sub/pkg/cache/junk.rs".to_string()));
}

#[test]
fn test_force_include_overrides_dot_ignore() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::write(root.join(".special"), "needed").unwrap();
    let files = discover_files(root, &[], &[".special".to_string()]).unwrap();
    assert_eq!(files.len(), 1);
}

#[test]
fn test_matches_pattern_forms() {
    assert!(matches_pattern("src/main.rs", "src/"));
    assert!(matches_pattern("a/src/main.rs", "src/"));
    assert!(!matches_pattern("srca/main.rs", "src/"));
    assert!(matches_pattern("src", "src/")); // exact dir name still matches
    assert!(matches_pattern("x.PNG", "*.png"));
    assert!(matches_pattern("some/subthing/here", "*sub*"));
    // Wildcard form is a plain substring match (original semantics).
    assert!(matches_pattern("subless", "*sub*"));
    assert!(matches_pattern("deep/path/README.md", "README.md"));
    assert!(matches_pattern("README.md", "README.md"));
    assert!(!matches_pattern("other.txt", "README.md"));
    assert!(matches_pattern("crates/foo/lib.rs", "crates/foo"));
    assert!(!matches_pattern("cratesx/foo/lib.rs", "crates/foo"));
}

#[test]
fn test_default_ignores_dirs_and_extensions() {
    assert!(is_default_ignored("target", true));
    assert!(is_default_ignored("a/node_modules", true));
    assert!(!is_default_ignored("xtarget", true));
    assert!(is_default_ignored("logo.PNG", false));
    assert!(is_default_ignored("Cargo.lock", false));
    assert!(!is_default_ignored("main.rs", false));
}

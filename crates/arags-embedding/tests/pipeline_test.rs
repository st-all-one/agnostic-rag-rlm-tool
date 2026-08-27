#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp,
    clippy::useless_vec
)]

use std::path::Path;
use std::sync::Arc;

use arags_embedding::embedder::config::EmbeddingConfig;
use arags_embedding::embedder::fallback::FallbackEmbedder;
use arags_embedding::pipeline::{
    IngestOptions, IngestionPipeline, compress_text, compute_hash, default_index_ignores,
    discover_files, glob_match, is_text_file, path_force_matches, path_is_ignored,
};

#[test]
fn test_compress_and_hash() {
    let text = "hello world, this is a test of compression";
    let compressed = compress_text(text);
    assert!(compressed.len() <= text.len() + 10);

    let hash = compute_hash(text);
    assert_eq!(hash.len(), 64); // SHA-256 hex
}

#[test]
fn test_compute_hash_deterministic() {
    let h1 = compute_hash("test");
    let h2 = compute_hash("test");
    assert_eq!(h1, h2);
}

#[test]
fn test_is_text_file() {
    assert!(is_text_file(Path::new("main.rs")));
    assert!(is_text_file(Path::new("README.md")));
    assert!(is_text_file(Path::new("config.json")));
    assert!(!is_text_file(Path::new("image.png")));
    assert!(!is_text_file(Path::new("binary")));
}

#[test]
fn test_discover_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.rs"), "fn main() {}").expect("write");
    std::fs::write(dir.path().join("b.py"), "print('hello')").expect("write");
    std::fs::write(dir.path().join("c.png"), b"binary").expect("write");
    std::fs::write(dir.path().join(".env"), "SECRET=1").expect("write");
    std::fs::write(dir.path().join("key.pem"), "-----").expect("write");

    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    std::fs::write(sub.join("d.txt"), "text").expect("write");

    let files = discover_files(dir.path(), &default_index_ignores(), &[], &[]).expect("discover");
    assert_eq!(files.len(), 3); // a.rs, b.py, sub/d.txt (filtered: .env, key.pem, c.png)
}

#[test]
fn test_discover_files_custom_ignore() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.rs"), "fn main() {}").expect("write");
    std::fs::write(dir.path().join("b.log"), "log entry").expect("write");
    std::fs::write(dir.path().join("c.rs"), "fn foo() {}").expect("write");

    let ignores = vec!["*.log".to_string()];
    let files =
        discover_files(dir.path(), &default_index_ignores(), &ignores, &[]).expect("discover");
    assert_eq!(files.len(), 2); // a.rs, c.rs (filtered: b.log)
}

#[test]
fn test_glob_match() {
    assert!(glob_match("*.pem", "server.pem"));
    assert!(glob_match("*.pem", "key.pem"));
    assert!(!glob_match("*.pem", "pem.txt"));
    assert!(glob_match(".env.*", ".env.local"));
    assert!(!glob_match(".env.*", ".env"));
    assert!(glob_match(".env", ".env"));
    assert!(!glob_match(".env", ".env.local"));
}

#[test]
fn test_default_ignores_sensitive_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.rs"), "fn main() {}").expect("write");
    std::fs::write(dir.path().join(".env"), "SECRET=1").expect("write");
    std::fs::create_dir_all(dir.path().join(".github")).expect("mkdir");
    std::fs::write(dir.path().join(".github").join("ci.yml"), "run").expect("write");
    std::fs::create_dir_all(dir.path().join(".vscode")).expect("mkdir");
    std::fs::write(dir.path().join(".vscode").join("settings.json"), "{}").expect("write");
    std::fs::create_dir_all(dir.path().join("vendor")).expect("mkdir");
    std::fs::write(dir.path().join("vendor").join("lib.rs"), "code").expect("write");

    let files = discover_files(dir.path(), &default_index_ignores(), &[], &[]).expect("discover");
    // Only the non-ignored source file is indexed by default.
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("a.rs"));
}

#[test]
fn test_force_include_bypasses_ignore() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(".env"), "SECRET=1").expect("write");
    std::fs::create_dir_all(dir.path().join(".github").join("workflows")).expect("mkdir");
    std::fs::write(
        dir.path().join(".github").join("workflows").join("ci.yml"),
        "run",
    )
    .expect("write");

    let force = vec![".env".to_string(), ".github".to_string()];
    let files =
        discover_files(dir.path(), &default_index_ignores(), &[], &force).expect("discover");
    assert_eq!(files.len(), 2);
}

#[test]
fn test_path_force_matches_glob() {
    assert!(path_force_matches(
        &["vendor/**".to_string()],
        "vendor/foo/bar.rs"
    ));
    assert!(path_force_matches(&["*.env".to_string()], "config.env"));
    assert!(!path_force_matches(
        &["vendor".to_string()],
        "vendorx/foo.rs"
    ));
}

/// Build a temp dir tree with the given `(rel_path -> content)` files.
fn write_tree(dir: &std::path::Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&p, content).expect("write");
    }
}

#[test]
fn test_path_is_ignored_default_patterns() {
    let defaults = default_index_ignores();
    // Each default pattern excludes a representative noisy path ...
    assert!(path_is_ignored(&defaults, "any/path/Seeds/x.rs"));
    assert!(path_is_ignored(&defaults, ".seeds/notes.md"));
    assert!(path_is_ignored(&defaults, "storage/logs/run.log"));
    assert!(path_is_ignored(&defaults, "storage/logs/sub/debug.log"));
    assert!(path_is_ignored(&defaults, "REFERENCE/ref.txt"));
    assert!(path_is_ignored(&defaults, "_Exemplos/exemplo.rs"));
    assert!(path_is_ignored(&defaults, "vendor/foo/bar.rs"));
    // ... and does NOT exclude legitimate source paths.
    assert!(!path_is_ignored(&defaults, "src/main.rs"));
    assert!(!path_is_ignored(&defaults, "crates/foo/src/lib.rs"));
    assert!(!path_is_ignored(&defaults, "storage/data.db"));
    assert!(!path_is_ignored(&defaults, "storage/logging.rs"));
    assert!(!path_is_ignored(&defaults, "seeds_util.rs"));
}

#[test]
fn test_discover_files_respects_ignores() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(
        dir.path(),
        &[
            ("src/main.rs", "fn main() {}"),
            ("vendor/lib.rs", "pub fn v() {}"),
            ("Seeds/seed.rs", "fn seed() {}"),
            (".seeds/notes.md", "# notes"),
            ("REFERENCE/ref.txt", "reference dump"),
            ("_Exemplos/exemplo.rs", "fn ex() {}"),
            ("storage/logs/run.log", "log line"),
        ],
    );

    let files = discover_files(dir.path(), &default_index_ignores(), &[], &[]).expect("discover");
    // Only the legitimate source file is indexed by default.
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("src/main.rs"));
}

#[test]
fn test_discover_files_custom_ignore_and_cleared_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(
        dir.path(),
        &[
            ("src/main.rs", "fn main() {}"),
            ("docs/api.md", "# api"),
            ("vendor/lib.rs", "pub fn v() {}"),
        ],
    );

    // Custom ignore (docs/) honored on top of defaults.
    let extra = vec!["docs".to_string()];
    let files =
        discover_files(dir.path(), &default_index_ignores(), &extra, &[]).expect("discover");
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("src/main.rs"));

    // Defaults cleared: vendor is now indexed (not skipped), docs still present
    // because extra ignores are omitted in this call.
    let files = discover_files(dir.path(), &[], &[], &[]).expect("discover");
    assert_eq!(files.len(), 3);
    assert!(files.iter().any(|f| f.ends_with("vendor/lib.rs")));
}

#[test]
fn test_pipeline_new() {
    let embedder = Arc::new(FallbackEmbedder::new(128));
    let pipeline = IngestionPipeline::new(embedder, None);
    assert_eq!(pipeline.batch_size(), 64);
}

#[test]
fn test_pipeline_from_config_lightweight() {
    let config = EmbeddingConfig::for_tests();
    let pipeline = IngestionPipeline::from_config(&config, None).expect("pipeline");
    assert_eq!(pipeline.batch_size(), 64);
}

#[test]
fn test_pipeline_ingest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "fn main() {\n    println!(\"hello\");\n}").expect("write");

    let embedder = Arc::new(FallbackEmbedder::new(128));
    let pipeline = IngestionPipeline::new(embedder, None);
    let options = IngestOptions::default();
    let result = pipeline.ingest(&[file_path], &options).expect("ingest");

    assert_eq!(result.total_files, 1);
    assert!(result.total_chunks >= 1);
    assert!(result.total_embedded >= 1);
}

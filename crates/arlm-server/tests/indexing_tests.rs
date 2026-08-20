//! Unit tests for the deterministic indexing primitives (chunking, language
//! detection, hashing), extracted from inline `#[cfg(test)]` modules.

use std::path::{Path, PathBuf};

use arlm_server::indexing::{chunk_lines, classify, hash_text, index_file, infer_language};

#[test]
fn test_infer_language() {
    assert_eq!(infer_language(Path::new("a.rs")), Some("rust"));
    assert_eq!(infer_language(Path::new("b.md")), Some("markdown"));
    assert_eq!(infer_language(Path::new("noext")), None);
}

#[test]
fn test_classify_code() {
    let path = PathBuf::from("main.rs");
    assert_eq!(classify(&path), "code");
}

#[test]
fn test_classify_markdown() {
    let path = PathBuf::from("README.md");
    assert_eq!(classify(&path), "markdown");
}

#[test]
fn test_chunk_lines_basic() {
    let content = "1\n2\n3\n4\n5";
    let chunks = chunk_lines(content, 3, 0);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].1, 3);
    assert_eq!(chunks[1].0, 4);
}

#[test]
fn test_chunk_lines_overlap() {
    let content = "1\n2\n3\n4\n5";
    let chunks = chunk_lines(content, 3, 1);
    assert_eq!(chunks[0].0, 1);
    assert_eq!(chunks[1].0, 3);
    assert_eq!(chunks[2].0, 5);
}

#[test]
fn test_chunk_lines_empty() {
    assert!(chunk_lines("", 3, 0).is_empty());
}

#[test]
fn test_hash_text_stable() {
    assert_eq!(hash_text("hello"), hash_text("hello"));
    assert_ne!(hash_text("hello"), hash_text("world"));
}

#[test]
fn test_index_file_assigns_metadata() {
    let chunks = index_file(Path::new("src/main.rs"), "fn foo() {}\nfn bar() {}");
    assert!(!chunks.is_empty());
    for c in &chunks {
        assert_eq!(c.file_path, "src/main.rs");
        assert_eq!(c.language.as_deref(), Some("rust"));
        assert_eq!(c.chunk_type, "code");
        assert_eq!(c.hash.len(), 64);
        assert!(c.line_start >= 1 && c.line_end >= c.line_start);
    }
}
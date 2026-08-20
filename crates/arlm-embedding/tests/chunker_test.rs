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

use arlm_embedding::chunker::code::{CodeChunker, is_structure_start};
use arlm_embedding::chunker::markdown::MarkdownChunker;
use arlm_embedding::chunker::recursive::RecursiveChunker;
use arlm_embedding::chunker::text::TextChunker;
use arlm_embedding::chunker::{
    ChunkingStrategy, detect_language, estimate_tokens, nth_line_byte_offset, prev_char_boundary,
};

// ---- chunker/mod.rs ----

#[test]
fn test_detect_language_rust() {
    let path = Path::new("src/main.rs");
    assert_eq!(detect_language(path).as_deref(), Some("rust"));
}

#[test]
fn test_detect_language_python() {
    let path = Path::new("script.py");
    assert_eq!(detect_language(path).as_deref(), Some("python"));
}

#[test]
fn test_detect_language_unknown() {
    let path = Path::new("file.xyz");
    assert_eq!(detect_language(path), None);
}

#[test]
fn test_prev_char_boundary_ascii() {
    let s = "hello";
    assert_eq!(prev_char_boundary(s, 5), 5);
    assert_eq!(prev_char_boundary(s, 3), 3);
}

#[test]
fn test_prev_char_boundary_unicode() {
    let s = "héllo"; // é is 2 bytes: [0xC3, 0xA9]
    assert_eq!(prev_char_boundary(s, 5), 5); // 'o' start
    assert_eq!(prev_char_boundary(s, 4), 4); // second 'l' start
    assert_eq!(prev_char_boundary(s, 3), 3); // first 'l' start
    assert_eq!(prev_char_boundary(s, 2), 1); // middle of 'é', recede to byte 1
}

#[test]
fn test_nth_line_byte_offset() {
    let s = "line1\nline2\nline3";
    assert_eq!(nth_line_byte_offset(s, 0), 0);
    assert_eq!(nth_line_byte_offset(s, 1), 6);
    assert_eq!(nth_line_byte_offset(s, 2), 12);
}

#[test]
fn test_estimate_tokens() {
    assert!(estimate_tokens("hello world") >= 2);
    assert_eq!(estimate_tokens(""), 0);
    assert!(estimate_tokens("  spaced  out  ") >= 2);
}

// ---- chunker/code.rs ----

#[test]
fn test_code_chunker_rust_functions() {
    let chunker = CodeChunker::new(512, 64);
    let content = r#"fn main() {
    println!("hello");
}

fn helper() -> i32 {
    42
}

fn another() {
    let x = 1;
}
"#;
    let path = Path::new("test.rs");
    let chunks = chunker.chunk(content, path);
    assert!(!chunks.is_empty());
    for chunk in &chunks {
        assert_eq!(chunk.language.as_deref(), Some("rust"));
    }
}

#[test]
fn test_code_chunker_python() {
    let chunker = CodeChunker::new(512, 64);
    let content = r#"def hello():
    print("hello")

def world():
    print("world")
"#;
    let path = Path::new("test.py");
    let chunks = chunker.chunk(content, path);
    assert!(!chunks.is_empty());
    for chunk in &chunks {
        assert_eq!(chunk.language.as_deref(), Some("python"));
    }
}

#[test]
fn test_code_chunker_line_fallback() {
    let chunker = CodeChunker::new(10, 2);
    let content =
        "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\nline11\nline12\n";
    let path = Path::new("unknown.xyz");
    let chunks = chunker.chunk(content, path);
    assert!(chunks.len() > 1);
}

#[test]
fn test_is_structure_start_rust() {
    assert!(is_structure_start("fn main() {", Some("rust")));
    assert!(is_structure_start("pub fn helper() -> i32 {", Some("rust")));
    assert!(is_structure_start("struct Foo {", Some("rust")));
    assert!(is_structure_start("impl Bar {", Some("rust")));
    assert!(!is_structure_start("let x = 1;", Some("rust")));
}

#[test]
fn test_is_structure_start_python() {
    assert!(is_structure_start("def hello():", Some("python")));
    assert!(is_structure_start("class Foo:", Some("python")));
    assert!(!is_structure_start("x = 1", Some("python")));
}

#[test]
fn test_merge_small_chunks() {
    let chunker = CodeChunker::new(100, 0);
    let content = "fn a() {}\n\nfn b() {}\n";
    let path = Path::new("test.rs");
    let chunks = chunker.chunk(content, path);
    assert!(!chunks.is_empty());
}

// ---- chunker/markdown.rs ----

#[test]
fn test_markdown_chunker_headings() {
    let chunker = MarkdownChunker::new(512);
    let content = "# Title\n\nSome intro text.\n\n## Section 1\n\nContent of section 1.\n\n## Section 2\n\nContent of section 2.\n";
    let path = Path::new("test.md");
    let chunks = chunker.chunk(content, path);
    assert_eq!(chunks.len(), 3);
    assert!(chunks[0].content.as_ref().contains("# Title"));
    assert!(chunks[1].content.as_ref().contains("## Section 1"));
    assert!(chunks[2].content.as_ref().contains("## Section 2"));
}

#[test]
fn test_markdown_chunker_no_headings() {
    let chunker = MarkdownChunker::new(512);
    let content = "Just some text without any headings.\n";
    let path = Path::new("test.md");
    let chunks = chunker.chunk(content, path);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content.as_ref(), content);
}

#[test]
fn test_markdown_chunker_consecutive_headings() {
    let chunker = MarkdownChunker::new(512);
    let content = "# A\n## B\n### C\n";
    let path = Path::new("test.md");
    let chunks = chunker.chunk(content, path);
    assert_eq!(chunks.len(), 3);
}

#[test]
fn test_markdown_chunker_empty() {
    let chunker = MarkdownChunker::new(512);
    let content = "";
    let path = Path::new("test.md");
    let chunks = chunker.chunk(content, path);
    assert!(chunks.is_empty());
}

// ---- chunker/recursive.rs ----

#[test]
fn test_recursive_chunker_short_text() {
    let chunker = RecursiveChunker::new(1000, 0);
    let content = "Short text.";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert_eq!(chunks.len(), 1);
}

#[test]
fn test_recursive_chunker_paragraphs() {
    let chunker = RecursiveChunker::new(3, 0);
    let content = "First paragraph has many words here.\n\nSecond paragraph also has many words here.\n\nThird paragraph with even more words here.";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert!(chunks.len() >= 2);
}

#[test]
fn test_recursive_chunker_sentences() {
    let chunker = RecursiveChunker::new(3, 0);
    let content = "First sentence has some words. Second sentence has more words. Third sentence has extra words.";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert!(chunks.len() >= 2);
}

#[test]
fn test_recursive_chunker_empty() {
    let chunker = RecursiveChunker::new(100, 0);
    let content = "";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert!(chunks.is_empty());
}

#[test]
fn test_recursive_chunker_custom_separators() {
    let chunker = RecursiveChunker::with_separators(3, 0, vec!["|||"]);
    let content = "First word here. ||| Second word here. ||| Third word here.";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert!(chunks.len() >= 2);
}

// ---- chunker/text.rs ----

#[test]
fn test_text_chunker_paragraphs() {
    let chunker = TextChunker::new(20, 0);
    let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert!(!chunks.is_empty());
    for chunk in &chunks {
        assert_eq!(chunk.chunk_type.as_deref(), Some("paragraph"));
    }
}

#[test]
fn test_text_chunker_single_paragraph() {
    let chunker = TextChunker::new(1000, 0);
    let content = "A single paragraph without any breaks.";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content.as_ref(), content);
}

#[test]
fn test_text_chunker_empty() {
    let chunker = TextChunker::new(100, 0);
    let content = "";
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(content, path);
    assert!(chunks.is_empty());
}

#[test]
fn test_text_chunker_many_paragraphs() {
    let chunker = TextChunker::new(10, 0);
    let content = (0..20)
        .map(|i| format!("Paragraph {i} with some words."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let path = Path::new("test.txt");
    let chunks = chunker.chunk(&content, path);
    assert!(chunks.len() > 1);
}

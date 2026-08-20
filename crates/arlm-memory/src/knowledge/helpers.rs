//! Pure helper functions for knowledge chunking and hashing.

use std::path::Path;

/// Find a line boundary at or just before `near` to avoid splitting mid-line.
#[must_use]
pub fn find_line_boundary(bytes: &[u8], near: usize) -> usize {
    // Search backwards from `near` for a newline
    let search_start = near.saturating_sub(100);
    for i in (search_start..near).rev() {
        if bytes[i] == b'\n' {
            return i + 1;
        }
    }
    near
}

/// Compute a SHA-256 hash of the given bytes.
#[must_use]
pub fn compute_hash(data: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Detect the programming language of a file from its extension.
#[must_use]
pub fn detect_language(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext {
            "rs" => "rust",
            "py" => "python",
            "js" => "javascript",
            "ts" => "typescript",
            "tsx" => "typescriptreact",
            "jsx" => "javascriptreact",
            "go" => "go",
            "java" => "java",
            "c" | "h" | "hpp" => "c",
            "cpp" | "cc" | "cxx" => "cpp",
            "rb" => "ruby",
            "sh" | "bash" => "shell",
            "sql" => "sql",
            "md" => "markdown",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            "html" => "html",
            "css" => "css",
            _ => "",
        })
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Estimate the token count of a piece of text (~4 chars/token).
#[must_use]
pub fn estimate_tokens(text: &str) -> i64 {
    // Rough estimate: ~4 chars per token
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    {
        ((text.len() as f64 / 4.0).ceil() as i64).max(1)
    }
}

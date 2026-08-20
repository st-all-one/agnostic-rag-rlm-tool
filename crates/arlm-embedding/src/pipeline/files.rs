//! File discovery, glob matching, content hashing and zstd compression.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zstd::stream::encode_all;

use crate::embedder::EmbeddingResult;

/// Compress text using zstd.
///
/// Returns the compressed bytes. Falls back to the raw bytes if compression
/// fails (e.g. empty/edge inputs).
#[must_use]
pub fn compress_text(text: &str) -> Vec<u8> {
    encode_all(text.as_bytes(), 0).unwrap_or_else(|_| text.as_bytes().to_vec())
}

/// Compute SHA-256 hash of text content.
#[must_use]
pub fn compute_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

/// Discover files in a directory recursively, respecting common ignore patterns.
///
/// # Arguments
///
/// * `root` - Root directory to search.
/// * `extra_ignores` - Additional glob patterns to ignore (e.g., `["*.log", "dist/"]`).
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn discover_files(root: &Path, extra_ignores: &[String]) -> EmbeddingResult<Vec<PathBuf>> {
    let _timer = crate::Timer::new("discover_files");
    let mut files = Vec::with_capacity(256);
    discover_files_recursive(root, extra_ignores, &mut files)?;
    Ok(files)
}

/// Default ignore patterns applied to all projects.
const DEFAULT_IGNORES: &[&str] = &[
    ".env", ".env.*", "*.pem", "*.key", "*.p12", "*.pfx", "*.jks",
];

fn discover_files_recursive(
    dir: &Path,
    extra_ignores: &[String],
    files: &mut Vec<PathBuf>,
) -> EmbeddingResult<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();

    for entry in &entries {
        let path = entry.path();

        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.')
                || name == "node_modules"
                || name == "target"
                || name == "vendor"
                || name == "__pycache__"
                || name == ".git"
            {
                continue;
            }

            let dominated = DEFAULT_IGNORES.iter().any(|pat| glob_match(pat, name));
            if dominated {
                continue;
            }

            let dominated = extra_ignores.iter().any(|pat| glob_match(pat, name));
            if dominated {
                continue;
            }
        }

        if path.is_dir() {
            discover_files_recursive(&path, extra_ignores, files)?;
        } else if path.is_file() && is_text_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

/// Simple glob matching for single-component patterns.
///
/// Supports `*` wildcard matching against a filename.
#[must_use]
pub fn glob_match(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return name.ends_with(suffix) && name.len() > suffix.len() + 1;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return name.starts_with(prefix) && name.len() > prefix.len() + 1;
    }
    name == pattern
}

/// Check if a file is likely text-based (by extension).
#[must_use]
pub fn is_text_file(path: &Path) -> bool {
    let text_extensions = [
        "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "cpp", "cc", "cxx", "c", "h", "rb",
        "php", "md", "txt", "log", "json", "yaml", "yml", "toml", "xml", "html", "css", "scss",
        "sql", "sh", "bash", "zsh", "fish", "vim", "el", "lisp", "r", "R", "jl", "swift", "kt",
        "scala", "ex", "exs", "erl", "hs", "ml", "clj", "lua", "pl", "pm",
    ];

    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| text_extensions.contains(&ext))
}

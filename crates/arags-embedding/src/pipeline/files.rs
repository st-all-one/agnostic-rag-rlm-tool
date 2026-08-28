//! File discovery, glob matching, content hashing and zstd compression.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use tracing::{debug, info};
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
/// * `default_ignores` - Default ignore patterns applied to every project
///   (see [`default_index_ignores`]). Pass an empty slice to clear the built-in
///   defaults (e.g. when a project wants to opt back into indexing them).
/// * `extra_ignores` - Additional glob patterns to ignore (e.g., `["*.log", "dist/"]`).
/// * `force_include` - Glob patterns that bypass every ignore rule above. A file
///   (or any of its ancestor directories) matching one of these patterns is
///   always indexed, even if it would otherwise be skipped. This is how
///   sensitive/ignored paths (`.env`, `.github`, `vendor`, …) can be explicitly
///   opted into indexing via `--force-include`.
///
/// Patterns are matched **as path segments or prefixes** anywhere in the path
/// (see [`path_is_ignored`]): `vendor` skips `vendor/foo/bar.rs`, `Seeds` skips
/// `any/path/Seeds/x.rs`, and the multi-component `storage/logs` skips
/// `storage/logs/run.log` and every file beneath it.
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn discover_files(
    root: &Path,
    default_ignores: &[String],
    extra_ignores: &[String],
    force_include: &[String],
) -> EmbeddingResult<Vec<PathBuf>> {
    let _timer = crate::Timer::new("discover_files");
    let mut files = Vec::with_capacity(256);
    let skipped = AtomicUsize::new(0);
    discover_files_recursive(
        root,
        root,
        default_ignores,
        extra_ignores,
        force_include,
        &skipped,
        &mut files,
    )?;

    info!(
        discovered = files.len(),
        ignored = skipped.load(Ordering::Relaxed),
        root = %root.display(),
        "discover_files complete"
    );
    Ok(files)
}

/// Default ignore patterns applied to all projects.
///
/// These cover sensitive dot-directories/files (`.env`, `.vscode`, `.github`,
/// `.gitlab`, `.zed`, …), vendored / build trees (`vendor`, `node_modules`,
/// `target`, …) and noisy corpus-diluting directories that hurt NL search
/// relevance (`Seeds`, `.seeds`, `REFERENCE`, `_Exemplos`, and any
/// `storage/logs` tree). They are never sent to the server unless explicitly
/// forced in via `force_include` or cleared by passing an empty default list.
const DEFAULT_IGNORES: &[&str] = &[
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
    "*.keystore",
    ".vscode",
    ".github",
    ".gitlab",
    ".zed",
    ".idea",
    "vendor",
    "node_modules",
    "target",
    "__pycache__",
    ".git",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".turbo",
    "bower_components",
    // Noisy corpus-diluting paths (issue agnostic-rag-rlm-tool-a884).
    "Seeds",
    ".seeds",
    "storage/logs",
    "REFERENCE",
    "_Exemplos",
];

/// Return the built-in default index ignore patterns as owned strings.
///
/// Use this to seed a config-driven ignore list; pass the (possibly edited)
/// result back into [`discover_files`] so project configuration can extend or
/// clear the defaults.
#[must_use]
pub fn default_index_ignores() -> Vec<String> {
    DEFAULT_IGNORES.iter().map(|s| (*s).to_string()).collect()
}

fn discover_files_recursive(
    root: &Path,
    dir: &Path,
    default_ignores: &[String],
    extra_ignores: &[String],
    force_include: &[String],
    skipped: &AtomicUsize,
    files: &mut Vec<PathBuf>,
) -> EmbeddingResult<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();

    for entry in &entries {
        let path = entry.path();

        // Relative path (used for force-include and ignore matching).
        let rel = match path.strip_prefix(root).ok().and_then(|p| p.to_str()) {
            Some(r) => r.to_string(),
            None => {
                // Outside root: skip to be safe.
                continue;
            }
        };

        // Force-included paths bypass every ignore rule below.
        if !force_include.is_empty() && path_force_matches(force_include, &rel) {
            if path.is_dir() {
                discover_files_recursive(
                    root,
                    &path,
                    default_ignores,
                    extra_ignores,
                    force_include,
                    skipped,
                    files,
                )?;
            } else {
                files.push(path);
            }
            continue;
        }

        if path_is_ignored(default_ignores, &rel) || path_is_ignored(extra_ignores, &rel) {
            skipped.fetch_add(1, Ordering::Relaxed);
            debug!(path = %rel, "skipped by ignore rule");
            continue;
        }

        if path.is_dir() {
            discover_files_recursive(
                root,
                &path,
                default_ignores,
                extra_ignores,
                force_include,
                skipped,
                files,
            )?;
        } else if path.is_file() && is_text_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

/// Returns `true` if `rel` (a `/`-separated relative path) is matched by any of
/// `patterns`.
///
/// Matching is "anywhere in the path": a single-component pattern (e.g. `vendor`,
/// `*.pem`, `Seeds`) matches if any `/`-separated segment of `rel` matches the
/// pattern (wildcards allowed), while a multi-component pattern (e.g.
/// `storage/logs`) matches only as a path **prefix** of `rel`.
#[must_use]
pub fn path_is_ignored(patterns: &[String], rel: &str) -> bool {
    let rcomp: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    patterns.iter().any(|p| {
        let pcomp: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
        if pcomp.is_empty() {
            return false;
        }
        if pcomp.len() == 1 {
            rcomp.iter().any(|seg| wildcard_component(pcomp[0], seg))
        } else {
            if rcomp.len() < pcomp.len() {
                return false;
            }
            pcomp
                .iter()
                .enumerate()
                .all(|(i, pc)| wildcard_component(pc, rcomp[i]))
        }
    })
}

/// Returns `true` if `rel` (a `/`-separated relative path) or any of its
/// ancestor directory prefixes matches one of the `patterns`.
#[must_use]
pub fn path_force_matches(patterns: &[String], rel: &str) -> bool {
    patterns.iter().any(|p| path_matches(p, rel))
}

/// Match a `/`-separated glob `pattern` against a `/`-separated `rel` path.
///
/// Supports `*` (matches within a path component) and `**` (matches across
/// `/` separators). A pattern matches if it equals `rel` or any ancestor
/// prefix of `rel` (so `vendor` forces in `vendor/foo/bar.rs`).
#[must_use]
fn path_matches(pattern: &str, rel: &str) -> bool {
    let pcomp: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let rcomp: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    if pcomp.is_empty() {
        return false;
    }
    if component_match(&pcomp, &rcomp) {
        return true;
    }
    for i in 1..rcomp.len() {
        if component_match(&pcomp, &rcomp[..i]) {
            return true;
        }
    }
    false
}

/// Recursive component-wise matcher backing [`path_matches`].
#[must_use]
fn component_match(pat: &[&str], rel: &[&str]) -> bool {
    match (pat.first(), rel.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            if pat.len() == 1 {
                return true;
            }
            for start in 0..=rel.len() {
                if component_match(&pat[1..], &rel[start..]) {
                    return true;
                }
            }
            false
        }
        (Some(p), Some(r)) => wildcard_component(p, r) && component_match(&pat[1..], &rel[1..]),
        _ => false,
    }
}

/// Single-component wildcard match (`*` matches zero or more characters).
#[must_use]
fn wildcard_component(pat: &str, comp: &str) -> bool {
    if pat == comp {
        return true;
    }
    if !pat.contains('*') {
        return false;
    }
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = comp.chars().collect();
    let mut dp = vec![vec![false; s.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == '*' {
            for j in 0..=s.len() {
                dp[i][j] = dp[i - 1][j] || (j > 0 && dp[i][j - 1]);
            }
        } else {
            for j in 1..=s.len() {
                if p[i - 1] == s[j - 1] {
                    dp[i][j] = dp[i - 1][j - 1];
                }
            }
        }
    }
    dp[p.len()][s.len()]
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
        "php", "md", "markdown", "txt", "log", "json", "yaml", "yml", "toml", "xml", "html", "css",
        "scss", "sql", "sh", "bash", "zsh", "fish", "vim", "el", "lisp", "r", "R", "jl", "swift",
        "kt", "scala", "ex", "exs", "erl", "hs", "ml", "clj", "lua", "pl", "pm", "env", "ini",
        "cfg", "conf", "envrc",
    ];

    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| text_extensions.contains(&ext))
}

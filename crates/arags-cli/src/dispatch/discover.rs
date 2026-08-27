//! File discovery and ignore-rule composition.
//!
//! Order of precedence for every candidate path: `force_include` wins over
//! everything; otherwise a path is skipped when it is dot-hidden, default-
//! ignored (sensitive/non-source), user-ignored, or excluded by `.gitignore`
//! rules.

use std::path::Path;

use anyhow::{Context, Result};

/// Discover files under `root`, skipping dot-paths, `.gitignore` rules
/// (root and nested), default-ignored and user-ignored paths unless
/// force-included.
pub(crate) fn discover_files(
    root: &Path,
    ignore: &[String],
    force_include: &[String],
) -> Result<Vec<std::path::PathBuf>> {
    let gitignore_rules = crate::gitignore::load_gitignores(root);
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read dir {}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| anyhow::anyhow!("read dir entry failed: {e}"))?;
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_s = rel.to_string_lossy().to_string();
            let is_dir = path.is_dir();

            // Every path component starting with '.' is hidden (git-style).
            let has_dot_component = rel_s.split('/').any(|seg| seg.starts_with('.'));
            if has_dot_component && !matches_any(&rel_s, force_include) {
                continue;
            }

            let forced = matches_any(&rel_s, force_include);
            let ignored = is_default_ignored(&rel_s, is_dir)
                || matches_any(&rel_s, ignore)
                || gitignore_decides(&gitignore_rules, &rel_s, is_dir);

            if is_dir {
                if forced || !ignored {
                    stack.push(path);
                }
                continue;
            }
            if forced || !ignored {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// Apply `.gitignore` semantics over the loaded rule list: the LAST matching
/// rule wins (so negations re-include), and deeper files were loaded later.
fn gitignore_decides(rules: &[crate::gitignore::IgnoreRule], rel: &str, is_dir: bool) -> bool {
    let mut decision = false;
    for rule in rules {
        if rule.decides(rel, is_dir) {
            decision = !rule.negated;
        }
    }
    decision
}

/// Directories ignored by default (non-source / VCS / build outputs).
const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".arags",
    "target",
    "node_modules",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
    "dist",
    "build",
    ".next",
    ".terraform",
    // Noisy corpus-diluting paths (issue agnostic-rlm-rs-a884).
    "Seeds",
    ".seeds",
    "REFERENCE",
    "_Exemplos",
];

/// Multi-component default-ignore path prefixes (matched as a `/`-separated
/// prefix, anywhere in the path). e.g. `storage/logs` skips
/// `storage/logs/run.log` and every file beneath it.
const DEFAULT_IGNORED_PATH_PREFIXES: &[&str] = &["storage/logs"];

/// File extensions ignored by default (binaries, media, lockfiles).
const DEFAULT_IGNORED_EXTS: &[&str] = &[
    "lock", "png", "jpg", "jpeg", "gif", "ico", "pdf", "zip", "gz", "tar", "bin", "exe", "dll",
    "so", "dylib", "woff", "woff2", "ttf", "eot", "mp4", "mp3", "wav",
];

/// Directories/files ignored by default (sensitive or non-source). Component-
/// based comparison avoids per-entry `format!` allocations in the hot loop.
fn is_default_ignored(rel: &str, is_dir: bool) -> bool {
    if is_dir {
        DEFAULT_IGNORED_DIRS.iter().any(|d| has_component(rel, d))
            || DEFAULT_IGNORED_PATH_PREFIXES
                .iter()
                .any(|p| rel == *p || rel.starts_with(&format!("{p}/")))
    } else {
        let rel_lc = rel.to_ascii_lowercase();
        DEFAULT_IGNORED_EXTS.iter().any(|ext| rel_lc.ends_with(ext))
    }
}

/// Simple glob-ish matcher supporting `dir/`, `*.ext`, `*sub*`, and exact.
pub(crate) fn matches_any(rel: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| matches_pattern(rel, p))
}

fn matches_pattern(rel: &str, pat: &str) -> bool {
    if let Some(dir) = pat.strip_suffix('/') {
        return has_component(rel, dir);
    }
    if let Some(ext) = pat.strip_prefix("*.") {
        return rel.to_ascii_lowercase().ends_with(ext);
    }
    if pat.contains('*') {
        let simple = pat.replace('*', "");
        return !simple.is_empty() && rel.to_ascii_lowercase().contains(&simple);
    }
    if pat.contains('/') {
        return rel == pat || rel.ends_with(pat) || rel.contains(pat);
    }
    has_component(rel, pat)
}

/// Whether any `/`-separated component of `rel` equals `name` exactly.
fn has_component(rel: &str, name: &str) -> bool {
    rel.split('/').any(|seg| seg == name)
}

#[cfg(test)]
mod tests;

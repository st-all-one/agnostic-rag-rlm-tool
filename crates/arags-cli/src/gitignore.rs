//! Minimal `.gitignore` rule parsing for file discovery.
//!
//! Supports the pragmatic subset of gitignore semantics needed for indexing:
//! blank lines and `#` comments, trailing `/` (directory-only), leading `/`
//! (anchored to the `.gitignore`'s directory), `*` globs, `**` segments, and
//! negation via leading `!` (last matching rule wins, like git).

use std::path::{Path, PathBuf};

/// One parsed ignore rule from a single `.gitignore` file.
#[derive(Debug, Clone)]
pub struct IgnoreRule {
    /// Normalized pattern (no leading `!`, no trailing `/`).
    pub pattern: String,
    /// Directory containing the `.gitignore` this rule came from, **relative
    /// to the project root** (`""`/`.` for the root-level file).
    pub base: PathBuf,
    /// Rule only matches directories.
    pub dir_only: bool,
    /// Pattern is anchored to `base` (leading `/` or contains a `/`).
    pub anchored: bool,
    /// Negated rule (`!pattern`) re-includes previously ignored paths.
    pub negated: bool,
}

/// Parse a single line into a rule. Returns `None` for blanks/comments.
#[must_use]
pub fn parse_line(line: &str, base: &Path) -> Option<IgnoreRule> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let mut rest = trimmed;
    let negated = rest.starts_with('!');
    if negated {
        rest = &rest[1..];
    }
    let dir_only = rest.ends_with('/');
    if dir_only {
        rest = rest.trim_end_matches('/');
    }
    if rest.is_empty() {
        return None;
    }
    // A literal escape of `\#` / `\!` keeps the character (git semantics);
    // treat other backslashes verbatim.
    if rest.starts_with('\\') && rest.len() > 1 {
        rest = &rest[1..];
    }
    let anchored = rest.contains('/');
    let pattern = rest.trim_start_matches('/').to_string();

    Some(IgnoreRule {
        pattern,
        base: base.to_path_buf(),
        dir_only,
        anchored,
        negated,
    })
}

/// Load every `.gitignore` under `root`, returning rules sorted so that
/// deeper files come later (deeper rules win ties, mirroring git precedence).
#[must_use]
pub fn load_gitignores(root: &Path) -> Vec<IgnoreRule> {
    let mut files = Vec::new();
    collect_gitignore_files(root, 0, &mut files);
    let mut rules = Vec::new();
    for path in files {
        // Store the rule's base relative to the project root: discovery works
        // with root-relative paths, and an absolute base would never strip
        // (making every nested rule either leak project-wide or never apply).
        let abs_base = path.parent().unwrap_or(root).to_path_buf();
        let base = abs_base.strip_prefix(root).unwrap_or(Path::new(".")).to_path_buf();
        let depth = abs_base.components().count();
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if let Some(rule) = parse_line(line, &base) {
                    rules.push((depth, rule));
                }
            }
        }
    }
    // Stable sort by depth: deeper .gitignore files are evaluated later so
    // their decisions override shallower ones on conflict.
    rules.sort_by_key(|(depth, _)| *depth);
    rules.into_iter().map(|(_, r)| r).collect()
}

fn collect_gitignore_files(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if depth > 32 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Skip common heavy/junk dirs; dotdirs hold no .gitignore we need.
            if name == ".git" || name == "target" || name == "node_modules" || name == ".venv" {
                continue;
            }
            collect_gitignore_files(&path, depth.saturating_add(1), out);
        } else if name == ".gitignore" {
            out.push(path);
        }
    }
}

impl IgnoreRule {
    /// Whether `rel` (slash-separated, relative to the project root) matches
    /// this rule. `is_dir` distinguishes files from directories.
    #[must_use]
    pub fn matches(&self, rel: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        // A `.gitignore` only governs paths inside its own directory; an
        // unanchored rule from `bootstrap/cache/.gitignore` must never ignore
        // `index.php` at the project root (Laravel wipes entire indexes
        // otherwise).
        let Some(rel_rel_to_base) = relative_to(&self.base, rel) else {
            return false;
        };
        let mut candidates: Vec<&str> = Vec::new();
        if self.anchored {
            // The path itself plus every ancestor segment, so that a rule
            // naming a directory also ignores everything beneath it.
            let bytes = rel_rel_to_base.as_str();
            candidates.push(bytes);
            let mut idx = 0;
            while let Some(pos) = bytes[idx..].find('/') {
                candidates.push(&bytes[..idx + pos]);
                idx += pos + 1;
            }
        } else {
            // Unanchored patterns match at any depth below `base`.
            candidates.extend(all_suffix_segments(&rel_rel_to_base));
        }
        candidates
            .iter()
            .any(|candidate| glob_match(&self.pattern, candidate))
    }

    /// Whether a decision was reached for `rel` (matched or negation hit).
    #[must_use]
    pub fn decides(&self, rel: &str, is_dir: bool) -> bool {
        self.matches(rel, is_dir)
    }
}

/// Path of `rel` as seen from `base` (both relative to project root), or
/// `None` when `rel` is not under `base`: a rule from a nested `.gitignore`
/// must not apply to paths outside its directory.
#[must_use]
fn relative_to(base: &Path, rel: &str) -> Option<String> {
    let base_s = base.to_string_lossy();
    if base_s == "." || base_s.is_empty() {
        return Some(rel.to_string());
    }
    let prefix = format!("{base_s}/");
    rel.strip_prefix(&prefix).map(str::to_string)
}

/// All `/`-suffixes of a slash-separated path, longest first.
#[must_use]
fn all_suffix_segments(rel: &str) -> Vec<&str> {
    let mut out = vec![rel];
    let mut idx = 0;
    while let Some(pos) = rel[idx..].find('/') {
        let at = idx + pos + 1;
        out.push(&rel[at..]);
        idx = at;
    }
    out
}

/// Glob matcher with `*`, `?`, `**`, and `{a,b}`-free semantics (git subset:
/// no brace expansion). `**` matches zero or more path segments.
#[must_use]
pub fn glob_match(pattern: &str, candidate: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = candidate.chars().collect();
    glob_impl(&pat, &text)
}

fn glob_impl(pat: &[char], text: &[char]) -> bool {
    if pat.is_empty() {
        return text.is_empty();
    }
    match pat[0] {
        '*' => {
            if pat.len() >= 2 && pat[1] == '*' {
                // `**`: matches zero or more whole segments (slashes allowed).
                let rest = &pat[2..];
                let rest = if rest.first() == Some(&'/') {
                    &rest[1..]
                } else {
                    rest
                };
                // Try every suffix starting at a segment boundary, plus the
                // empty suffix at end-of-text.
                if glob_impl(rest, &[]) {
                    return true;
                }
                for start in 0..text.len() {
                    let boundary = text[start] == '/' || start == 0;
                    if !boundary {
                        continue;
                    }
                    let suffix = if text[start] == '/' {
                        &text[start + 1..]
                    } else {
                        &text[start..]
                    };
                    if glob_impl(rest, suffix) {
                        return true;
                    }
                }
                false
            } else {
                // `*`: anything except `/`.
                for skip in 0..=text.len() {
                    if glob_impl(&pat[1..], &text[skip..]) {
                        return true;
                    }
                    if text.get(skip) == Some(&'/') {
                        break;
                    }
                }
                false
            }
        }
        '?' => matches!(text.first(), Some(c) if *c != '/') && glob_impl(&pat[1..], &text[1..]),
        c => text.first() == Some(&c) && glob_impl(&pat[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests;

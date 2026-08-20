//! Markdown rendering/parsing and filename sanitization helpers.

use anyhow::{Context, Result};
use serde_yaml_ng;

use crate::persist::types::Frontmatter;

/// The wiki directory name inside a project.
pub const WIKI_DIR: &str = ".arlm/wiki";

/// Render a markdown page with YAML frontmatter.
#[must_use]
pub fn render_markdown(frontmatter: &Frontmatter, body: &str) -> String {
    let yaml = serde_yaml_ng::to_string(frontmatter).unwrap_or_default();
    format!("---\n{yaml}---\n\n{body}")
}

/// Parse a markdown page with YAML frontmatter.
///
/// Returns `(frontmatter, body)`.
///
/// # Errors
///
/// Returns an error if the content is not valid wiki markdown.
pub fn parse_markdown(content: &str) -> Result<(Frontmatter, String)> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .context("missing opening frontmatter delimiter")?;

    let (yaml_part, body) = rest
        .split_once("\n---\n")
        .or_else(|| rest.split_once("\n---\r\n"))
        .or_else(|| {
            // Handle case where closing --- is at end of content
            rest.rsplit_once("\n---")
                .filter(|(_, after)| after.trim().is_empty())
                .map(|(before, _)| (before, ""))
        })
        .context("missing closing frontmatter delimiter")?;

    let frontmatter: Frontmatter =
        serde_yaml_ng::from_str(yaml_part).context("failed to parse frontmatter YAML")?;

    // Strip leading blank line after frontmatter
    let body = body.strip_prefix('\n').unwrap_or(body);

    Ok((frontmatter, body.to_string()))
}

/// Sanitize a string into a filesystem-safe slug.
///
/// Lowercases, replaces non-alphanumeric characters with hyphens,
/// collapses consecutive hyphens, and trims leading/trailing hyphens.
#[must_use]
pub fn sanitize_slug(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut prev_hyphen = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if ((ch == '_' || ch == '-') || (ch.is_whitespace() || ch == '/' || ch == '\\'))
            && !prev_hyphen
            && !slug.is_empty()
        {
            slug.push('-');
            prev_hyphen = true;
        }
        // Skip other characters (punctuation, etc.)
    }

    slug.trim_matches('-').to_string()
}

/// Sanitize an identifier for use as a filename.
///
/// Unlike `sanitize_slug`, this preserves underscores and alphanumeric characters,
/// only replacing truly unsafe characters. Designed for IDs like `s_abc123`.
#[must_use]
pub fn sanitize_identifier(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            result.push(ch);
        }
    }
    result
}

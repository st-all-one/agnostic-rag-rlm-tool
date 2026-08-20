//! Wiki markdown persistence: rendering, sanitization, and page management.
//!
//! This module is organized into cohesive submodules:
//! - [`types`] — frontmatter, scopes, and persist option/result types.
//! - [`format`] — markdown rendering/parsing and filename sanitization.
//! - [`engine`] — the [`PersistEngine`] lifecycle and raw IO helpers.
//! - [`ops`] — high-level persist operations for each wiki scope.

pub mod engine;
pub mod format;
pub mod ops;
pub mod types;

pub use engine::PersistEngine;
pub use format::{parse_markdown, render_markdown, sanitize_identifier, sanitize_slug};
pub use types::*;

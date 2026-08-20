//! The [`PersistEngine`] lifecycle and low-level file IO helpers.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ScopedTimer;
use crate::persist::format::WIKI_DIR;
use crate::persist::types::WikiScope;

/// Engine for persisting wiki markdown pages.
pub struct PersistEngine {
    pub(crate) wiki_root: PathBuf,
}

impl PersistEngine {
    /// Create a new `PersistEngine` rooted at `<project>/.arlm/wiki/`.
    ///
    /// # Errors
    ///
    /// Returns an error if the wiki directory cannot be created.
    pub fn new(project_path: &Path) -> Result<Self> {
        let wiki_root = project_path.join(WIKI_DIR);
        Self::ensure_dirs(&wiki_root)?;
        Ok(Self { wiki_root })
    }

    /// Create a `PersistEngine` with an explicit wiki root (for testing).
    #[must_use]
    pub fn with_wiki_root(wiki_root: PathBuf) -> Self {
        Self { wiki_root }
    }

    /// Get the wiki root directory.
    #[must_use]
    pub fn wiki_root(&self) -> &Path {
        &self.wiki_root
    }

    /// Persist a raw markdown body at an arbitrary wiki path.
    ///
    /// The `wiki_path` is relative to the wiki root (e.g. `rules/no-unwrap.md`).
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_raw(
        &self,
        wiki_path: &str,
        body: &str,
    ) -> Result<crate::persist::PersistResult> {
        let _timer = ScopedTimer::new("persist_raw");

        let file_path = self.wiki_root.join(wiki_path);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
        }

        std::fs::write(&file_path, body)
            .with_context(|| format!("failed to write file: {}", file_path.display()))?;

        tracing::info!(path = %file_path.display(), "raw page persisted");

        Ok(crate::persist::PersistResult {
            path: file_path,
            relative_path: wiki_path.to_string(),
        })
    }

    /// Read and parse a persisted wiki page.
    ///
    /// Returns the frontmatter and the body (everything after the frontmatter block).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn read_page(&self, wiki_path: &str) -> Result<(crate::persist::Frontmatter, String)> {
        let file_path = self.wiki_root.join(wiki_path);
        let content = std::fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read file: {}", file_path.display()))?;

        crate::persist::parse_markdown(&content)
    }

    /// List all pages under a scope.
    ///
    /// # Errors
    ///
    /// Returns an error if directory reading fails.
    pub fn list_pages(&self, scope: WikiScope) -> Result<Vec<String>> {
        let dir = self.wiki_root.join(scope.dir_name());
        if !dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut pages = Vec::new();
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry.context("failed to read dir entry")?;
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    pages.push(format!("{}/{}", scope.dir_name(), name));
                }
            }
        }

        pages.sort();
        Ok(pages)
    }

    pub(crate) fn ensure_dirs(wiki_root: &Path) -> Result<()> {
        for scope in &[
            WikiScope::Searches,
            WikiScope::Analyses,
            WikiScope::Decisions,
            WikiScope::Sessions,
            WikiScope::Trajectories,
            WikiScope::Global,
        ] {
            let dir = wiki_root.join(scope.dir_name());
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create directory: {}", dir.display()))?;
        }
        Ok(())
    }

    pub(crate) fn next_sequence(&self, scope: WikiScope) -> Result<u32> {
        let dir = self.wiki_root.join(scope.dir_name());
        if !dir.is_dir() {
            return Ok(1);
        }

        let mut max_seq: u32 = 0;
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry.context("failed to read dir entry")?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(seq_part) = name_str.split('-').next() {
                if let Ok(seq) = seq_part.parse::<u32>() {
                    max_seq = max_seq.max(seq);
                }
            }
        }

        Ok(max_seq + 1)
    }
}

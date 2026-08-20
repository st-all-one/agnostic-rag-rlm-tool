//! Wiki checkpoint management: snapshot, list, restore, delete.

pub mod fs;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ScopedTimer;
use crate::checkpoint::fs::{copy_dir_recursive, dir_size};

pub use fs::parse_checkpoint_name;

/// The subdirectory under the wiki root where checkpoints are stored.
pub const CHECKPOINT_DIR: &str = ".checkpoints";

/// Metadata about a single checkpoint.
#[derive(Debug, Clone)]
pub struct CheckpointInfo {
    /// User-supplied name for the checkpoint.
    pub name: String,
    /// ISO 8601 timestamp of when the checkpoint was created.
    pub timestamp: String,
    /// Total size of the checkpoint in bytes.
    pub size: u64,
    /// Absolute path to the checkpoint directory.
    pub path: PathBuf,
}

/// Manages creating, listing, restoring, and deleting wiki checkpoints.
///
/// A checkpoint is a timestamped copy of the entire wiki directory,
/// stored under `<wiki_root>/.checkpoints/{name}_{timestamp}/`.
pub struct CheckpointManager {
    wiki_root: PathBuf,
    checkpoint_root: PathBuf,
}

impl CheckpointManager {
    /// Create a new `CheckpointManager` for the given wiki root.
    ///
    /// # Errors
    ///
    /// Returns an error if the checkpoint directory cannot be created.
    pub fn new(wiki_root: &Path) -> Result<Self> {
        let checkpoint_root = wiki_root.join(CHECKPOINT_DIR);
        std::fs::create_dir_all(&checkpoint_root).with_context(|| {
            format!(
                "failed to create checkpoint dir: {}",
                checkpoint_root.display()
            )
        })?;
        Ok(Self {
            wiki_root: wiki_root.to_path_buf(),
            checkpoint_root,
        })
    }

    /// Get the checkpoint root directory.
    #[must_use]
    pub fn checkpoint_root(&self) -> &Path {
        &self.checkpoint_root
    }

    /// Get the wiki root directory this manager snapshots.
    #[must_use]
    pub fn wiki_root(&self) -> &Path {
        &self.wiki_root
    }

    /// Create a snapshot of the wiki directory.
    ///
    /// The checkpoint is stored at `.checkpoints/{name}_{timestamp}/`.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the wiki or writing the checkpoint fails.
    pub fn create_checkpoint(&self, name: &str) -> Result<PathBuf> {
        let _timer = ScopedTimer::new("create_checkpoint");

        let now = chrono::Utc::now();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let dir_name = format!("{name}_{timestamp}");
        let checkpoint_path = self.checkpoint_root.join(&dir_name);

        if self.wiki_root.is_dir() {
            copy_dir_recursive(&self.wiki_root, &checkpoint_path).with_context(|| {
                format!(
                    "failed to copy wiki to checkpoint: {}",
                    checkpoint_path.display()
                )
            })?;
        } else {
            std::fs::create_dir_all(&checkpoint_path).with_context(|| {
                format!(
                    "failed to create checkpoint dir: {}",
                    checkpoint_path.display()
                )
            })?;
        }

        tracing::info!(
            name = name,
            path = %checkpoint_path.display(),
            "checkpoint created"
        );

        Ok(checkpoint_path)
    }

    /// List all available checkpoints.
    ///
    /// Checkpoints are sorted by timestamp (oldest first).
    ///
    /// # Errors
    ///
    /// Returns an error if reading the checkpoint directory fails.
    pub fn list_checkpoints(&self) -> Result<Vec<CheckpointInfo>> {
        let _timer = ScopedTimer::new("list_checkpoints");

        if !self.checkpoint_root.is_dir() {
            return Ok(Vec::new());
        }

        let mut checkpoints = Vec::new();
        let entries = std::fs::read_dir(&self.checkpoint_root).with_context(|| {
            format!(
                "failed to read checkpoint dir: {}",
                self.checkpoint_root.display()
            )
        })?;

        for entry in entries {
            let entry = entry.context("failed to read dir entry")?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(info) = parse_checkpoint_name(dir_name) {
                    let size = dir_size(&path)?;
                    checkpoints.push(CheckpointInfo {
                        name: info.0,
                        timestamp: info.1,
                        size,
                        path,
                    });
                }
            }
        }

        checkpoints.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(checkpoints)
    }

    /// Restore the wiki from a checkpoint.
    ///
    /// This replaces the current wiki contents with the checkpoint contents.
    ///
    /// # Errors
    ///
    /// Returns an error if the checkpoint doesn't exist or the copy fails.
    pub fn restore_checkpoint(&self, name: &str) -> Result<()> {
        let _timer = ScopedTimer::new("restore_checkpoint");

        let checkpoint_path = self.find_checkpoint(name)?;

        // Clear current wiki (remove all subdirs except .checkpoints)
        if self.wiki_root.is_dir() {
            let entries = std::fs::read_dir(&self.wiki_root).with_context(|| {
                format!("failed to read wiki dir: {}", self.wiki_root.display())
            })?;
            for entry in entries {
                let entry = entry.context("failed to read dir entry")?;
                let path = entry.path();
                if path.file_name().and_then(|n| n.to_str()) == Some(CHECKPOINT_DIR) {
                    continue;
                }
                if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                        .with_context(|| format!("failed to remove dir: {}", path.display()))?;
                } else {
                    std::fs::remove_file(&path)
                        .with_context(|| format!("failed to remove file: {}", path.display()))?;
                }
            }
        } else {
            std::fs::create_dir_all(&self.wiki_root).with_context(|| {
                format!("failed to create wiki dir: {}", self.wiki_root.display())
            })?;
        }

        // Copy checkpoint contents into wiki root
        copy_dir_recursive(&checkpoint_path, &self.wiki_root).with_context(|| {
            format!(
                "failed to restore checkpoint {} to {}",
                checkpoint_path.display(),
                self.wiki_root.display()
            )
        })?;

        tracing::info!(
            name = name,
            from = %checkpoint_path.display(),
            "checkpoint restored"
        );

        Ok(())
    }

    /// Delete a checkpoint by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the checkpoint doesn't exist or removal fails.
    pub fn delete_checkpoint(&self, name: &str) -> Result<()> {
        let _timer = ScopedTimer::new("delete_checkpoint");

        let checkpoint_path = self.find_checkpoint(name)?;

        std::fs::remove_dir_all(&checkpoint_path).with_context(|| {
            format!("failed to remove checkpoint: {}", checkpoint_path.display())
        })?;

        tracing::info!(
            name = name,
            path = %checkpoint_path.display(),
            "checkpoint deleted"
        );

        Ok(())
    }

    /// Find the checkpoint directory matching `name`.
    ///
    /// Returns the first match if multiple checkpoints share the same name.
    fn find_checkpoint(&self, name: &str) -> Result<PathBuf> {
        let entries = std::fs::read_dir(&self.checkpoint_root).with_context(|| {
            format!(
                "failed to read checkpoint dir: {}",
                self.checkpoint_root.display()
            )
        })?;

        for entry in entries {
            let entry = entry.context("failed to read dir entry")?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    if dir_name.starts_with(&format!("{name}_")) {
                        return Ok(path);
                    }
                }
            }
        }

        anyhow::bail!("checkpoint not found: {name}")
    }
}

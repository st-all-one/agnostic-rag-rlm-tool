use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::ScopedTimer;

/// The subdirectory under the wiki root where checkpoints are stored.
const CHECKPOINT_DIR: &str = ".checkpoints";

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

    /// Create a snapshot of the wiki directory.
    ///
    /// The checkpoint is stored at `.checkpoints/{name}_{timestamp}/`.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the wiki or writing the checkpoint fails.
    pub fn create_checkpoint(&self, name: &str) -> Result<PathBuf> {
        let _timer = ScopedTimer::new("create_checkpoint");

        let now = Utc::now();
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

/// Recursively copy a directory tree, skipping `.checkpoints` directories.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .with_context(|| format!("failed to create dir: {}", dst.display()))?;

    let entries =
        std::fs::read_dir(src).with_context(|| format!("failed to read dir: {}", src.display()))?;

    for entry in entries {
        let entry = entry.context("failed to read dir entry")?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        // Skip .checkpoints to avoid infinite recursion
        if src_path.is_dir() && src_path.file_name().is_some_and(|n| n == CHECKPOINT_DIR) {
            continue;
        }

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
}

/// Calculate the total size of a directory recursively.
fn dir_size(path: &Path) -> Result<u64> {
    let mut total: u64 = 0;
    let entries = std::fs::read_dir(path)
        .with_context(|| format!("failed to read dir: {}", path.display()))?;

    for entry in entries {
        let entry = entry.context("failed to read dir entry")?;
        let metadata = entry.metadata().context("failed to read metadata")?;
        if metadata.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }

    Ok(total)
}

/// Parse a checkpoint directory name into (name, timestamp).
///
/// Expected format: `{name}_{YYYYmmddTHHMMSSZ}`
/// Returns `None` if the format doesn't match.
fn parse_checkpoint_name(dir_name: &str) -> Option<(String, String)> {
    // Find the last underscore followed by a timestamp-like pattern
    let underscore_pos = dir_name.rfind('_')?;
    let name = &dir_name[..underscore_pos];
    let timestamp = &dir_name[underscore_pos + 1..];

    // Basic validation: timestamp should be 16 chars (YYYYmmddTHHMMSSZ)
    if timestamp.len() == 16 && timestamp.ends_with('Z') {
        Some((name.to_string(), timestamp.to_string()))
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (CheckpointManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let wiki_root = tmp.path().join("wiki");
        std::fs::create_dir_all(&wiki_root).unwrap();

        // Create some wiki content
        let searches = wiki_root.join("searches");
        std::fs::create_dir_all(&searches).unwrap();
        std::fs::write(searches.join("test.md"), "# Test Search").unwrap();

        let analyses = wiki_root.join("analyses");
        std::fs::create_dir_all(&analyses).unwrap();
        std::fs::write(analyses.join("001-analysis.md"), "# Analysis").unwrap();

        let manager = CheckpointManager::new(&wiki_root).unwrap();
        (manager, tmp)
    }

    #[test]
    fn test_new_creates_checkpoint_dir() {
        let tmp = TempDir::new().unwrap();
        let wiki_root = tmp.path().join("wiki");
        std::fs::create_dir_all(&wiki_root).unwrap();

        let manager = CheckpointManager::new(&wiki_root).unwrap();
        assert!(manager.checkpoint_root().is_dir());
        assert_eq!(manager.checkpoint_root(), wiki_root.join(CHECKPOINT_DIR));
    }

    #[test]
    fn test_create_checkpoint() {
        let (manager, _tmp) = setup();

        let path = manager.create_checkpoint("before-edit").unwrap();
        assert!(path.is_dir());
        assert!(path.join("searches").is_dir());
        assert!(path.join("searches/test.md").is_file());
        assert!(path.join("analyses").is_dir());
        assert!(path.join("analyses/001-analysis.md").is_file());

        let content = std::fs::read_to_string(path.join("searches/test.md")).unwrap();
        assert_eq!(content, "# Test Search");
    }

    #[test]
    fn test_list_checkpoints_empty() {
        let (manager, _tmp) = setup();
        let list = manager.list_checkpoints().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_list_checkpoints() {
        let (manager, _tmp) = setup();

        manager.create_checkpoint("alpha").unwrap();
        manager.create_checkpoint("beta").unwrap();

        let list = manager.list_checkpoints().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|c| c.name == "alpha"));
        assert!(list.iter().any(|c| c.name == "beta"));
    }

    #[test]
    fn test_list_checkpoints_sorted_by_timestamp() {
        let (manager, _tmp) = setup();

        manager.create_checkpoint("first").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        manager.create_checkpoint("second").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        manager.create_checkpoint("third").unwrap();

        let list = manager.list_checkpoints().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "first");
        assert!(list[0].timestamp <= list[1].timestamp);
        assert_eq!(list[1].name, "second");
        assert!(list[1].timestamp <= list[2].timestamp);
        assert_eq!(list[2].name, "third");
    }

    #[test]
    fn test_checkpoint_info_size() {
        let (manager, _tmp) = setup();

        manager.create_checkpoint("sized").unwrap();
        let list = manager.list_checkpoints().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].size > 0);
    }

    #[test]
    fn test_restore_checkpoint() {
        let (manager, _tmp) = setup();

        manager.create_checkpoint("backup").unwrap();

        // Modify wiki
        let searches = manager.wiki_root.join("searches");
        std::fs::write(searches.join("new.md"), "# New Page").unwrap();
        std::fs::remove_file(searches.join("test.md")).unwrap();

        assert!(searches.join("new.md").is_file());
        assert!(!searches.join("test.md").is_file());

        // Restore
        manager.restore_checkpoint("backup").unwrap();

        assert!(!searches.join("new.md").is_file());
        assert!(searches.join("test.md").is_file());
        let content = std::fs::read_to_string(searches.join("test.md")).unwrap();
        assert_eq!(content, "# Test Search");
    }

    #[test]
    fn test_restore_preserves_checkpoints_dir() {
        let (manager, _tmp) = setup();

        manager.create_checkpoint("snap").unwrap();
        manager.restore_checkpoint("snap").unwrap();

        // .checkpoints should still exist and the checkpoint should remain
        let list = manager.list_checkpoints().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "snap");
    }

    #[test]
    fn test_delete_checkpoint() {
        let (manager, _tmp) = setup();

        manager.create_checkpoint("to-delete").unwrap();
        assert_eq!(manager.list_checkpoints().unwrap().len(), 1);

        manager.delete_checkpoint("to-delete").unwrap();
        assert_eq!(manager.list_checkpoints().unwrap().len(), 0);
    }

    #[test]
    fn test_delete_nonexistent_checkpoint() {
        let (manager, _tmp) = setup();
        let result = manager.delete_checkpoint("nope");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_restore_nonexistent_checkpoint() {
        let (manager, _tmp) = setup();
        let result = manager.restore_checkpoint("nope");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_same_name_multiple_checkpoints() {
        let (manager, _tmp) = setup();

        manager.create_checkpoint("my-snap").unwrap();
        // Small sleep to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(1100));
        manager.create_checkpoint("my-snap").unwrap();

        let list = manager.list_checkpoints().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|c| c.name == "my-snap"));

        // Restore should use the first (oldest) match
        manager.restore_checkpoint("my-snap").unwrap();
    }

    #[test]
    fn test_create_checkpoint_empty_wiki() {
        let tmp = TempDir::new().unwrap();
        let wiki_root = tmp.path().join("empty-wiki");
        std::fs::create_dir_all(&wiki_root).unwrap();

        let manager = CheckpointManager::new(&wiki_root).unwrap();
        let path = manager.create_checkpoint("empty").unwrap();
        assert!(path.is_dir());
        assert!(manager.list_checkpoints().unwrap().len() == 1);
    }

    #[test]
    fn test_parse_checkpoint_name_valid() {
        let (name, ts) = parse_checkpoint_name("my-snap_20240115T103000Z").unwrap();
        assert_eq!(name, "my-snap");
        assert_eq!(ts, "20240115T103000Z");
    }

    #[test]
    fn test_parse_checkpoint_name_invalid() {
        assert!(parse_checkpoint_name("no-timestamp-here").is_none());
        assert!(parse_checkpoint_name("short_2024").is_none());
    }

    #[test]
    fn test_checkpoint_nested_dirs() {
        let (manager, _tmp) = setup();

        let rules = manager.wiki_root.join("rules");
        std::fs::create_dir_all(rules.join("subdir")).unwrap();
        std::fs::write(rules.join("rule.md"), "# Rule").unwrap();
        std::fs::write(rules.join("subdir/nested.md"), "# Nested").unwrap();

        let path = manager.create_checkpoint("nested").unwrap();
        assert!(path.join("rules/rule.md").is_file());
        assert!(path.join("rules/subdir/nested.md").is_file());

        manager.restore_checkpoint("nested").unwrap();
        assert!(rules.join("rule.md").is_file());
        assert!(rules.join("subdir/nested.md").is_file());
    }
}

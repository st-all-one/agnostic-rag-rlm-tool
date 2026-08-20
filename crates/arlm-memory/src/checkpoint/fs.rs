//! Filesystem helpers for checkpoint copy/restore and name parsing.

use std::path::Path;

use anyhow::{Context, Result};

use crate::checkpoint::CHECKPOINT_DIR;

/// Recursively copy a directory tree, skipping `.checkpoints` directories.
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
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
pub(crate) fn dir_size(path: &Path) -> Result<u64> {
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
#[must_use]
pub fn parse_checkpoint_name(dir_name: &str) -> Option<(String, String)> {
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

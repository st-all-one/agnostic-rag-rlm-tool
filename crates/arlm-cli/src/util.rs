use std::path::PathBuf;

/// Get the shared arlm data directory.
///
/// All projects share a single database at `~/.arlm/knowledge.db`.
/// Override with `ARLM_DATA_DIR` env var (used in tests).
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ARLM_DATA_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var("HOME")
        .map_or_else(|_| PathBuf::from("."), PathBuf::from)
        .join(".arlm")
}

/// Get the project name from a path.
///
/// Extracts the last component of the path as the project name.
#[must_use]
pub fn project_name(project: &std::path::Path) -> String {
    project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

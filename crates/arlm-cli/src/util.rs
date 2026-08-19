use std::path::PathBuf;

/// Get the arlm data directory for projects.
#[must_use]
pub fn project_dirs() -> PathBuf {
    std::env::var("HOME")
        .map_or_else(|_| PathBuf::from("."), PathBuf::from)
        .join(".arlm")
        .join("projects")
}

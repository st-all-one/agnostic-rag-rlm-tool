//! Watcher registration (`arags index --register/--unregister`).

use std::path::Path;

use anyhow::{Context, Result};

/// Persist the registration and start the detached watcher daemon
/// (`arags index --register`).
pub(crate) fn run_register(root: &Path, project_name: &str) -> Result<()> {
    if crate::watcher::is_running(root) {
        println!("Watcher already running for {}", root.display());
        return Ok(());
    }
    crate::user_config::set_watch_enabled(&root.join(".arags.toml"), true, project_name)?;
    crate::watcher::spawn_daemon(root)?;
    println!(
        "Registered {} for background auto-update (re-index after 1 min of quiet). Stop with `arags index --unregister`.",
        root.display()
    );
    Ok(())
}

/// Stop the watcher daemon and clear the registration flag.
pub(crate) fn run_unregister(path: &Path) -> Result<()> {
    let absolute = std::fs::canonicalize(path)
        .with_context(|| format!("failed to resolve path: {}", path.display()))?;
    if crate::watcher::is_running(&absolute) {
        crate::watcher::request_stop(&absolute)?;
        println!("Watcher stop requested for {}", absolute.display());
    } else {
        println!("No watcher running for {}", absolute.display());
    }
    crate::user_config::set_watch_enabled(&absolute.join(".arags.toml"), false, "")
}

//! Client-side background watcher for registered projects (`arags index
//! --register`).
//!
//! Mirrors `git maintenance`: registration is persisted in the project's
//! `.arags.toml` under `[watch]`, and a detached daemon process monitors the
//! tree. Filesystem changes start a **1-minute quiet window**; when it closes,
//! only the changed (still present, still includable) files are re-streamed to
//! the server, which replaces the chunks of each touched file.
//!
//! Legacy note: this replaces the orphan `WatchMonitor` in `arags-memory`
//! (the old `--watch` experiment), which is removed as part of the migration.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use notify::{EventKind, RecursiveMode, Watcher};
use tracing::{debug, info};

/// Quiet-window delay before flushing accumulated changes to the server.
pub const FLUSH_DELAY: Duration = Duration::from_secs(60);

/// Marker file that asks the daemon to exit gracefully (avoids signals).
const STOP_FILE: &str = ".arags-watch.stop";
/// PID bookkeeping for "is it running?" checks.
const PID_FILE: &str = ".arags-watch.pid";

/// Both marker paths are dotfiles at the project root, so the indexer's
/// dot-path rule ignores them automatically.
#[must_use]
pub fn stop_path(root: &Path) -> PathBuf {
    root.join(STOP_FILE)
}

#[must_use]
pub fn pid_path(root: &Path) -> PathBuf {
    root.join(PID_FILE)
}

/// Whether a watcher daemon is (apparently) running for `root`.
#[must_use]
pub fn is_running(root: &Path) -> bool {
    pid_path(root).exists() && !stop_path(root).exists()
}

/// Ask a running daemon to stop by creating the stop marker.
///
/// # Errors
///
/// Propagates filesystem errors from creating the marker file.
pub fn request_stop(root: &Path) -> Result<()> {
    std::fs::write(stop_path(root), b"stop\n")
        .with_context(|| format!("failed to write {}", stop_path(root).display()))
}

/// Persist the daemon PID after spawning (best-effort bookkeeping).
fn write_pid(root: &Path, pid: u32) -> Result<()> {
    std::fs::write(pid_path(root), format!("{pid}\n"))
        .with_context(|| format!("failed to write {}", pid_path(root).display()))
}

/// Spawn `arags __watch <root>` fully detached from this process.
/// The child is orphaned on parent exit and keeps running (no unsafe).
///
/// # Errors
///
/// Fails if the current executable path cannot be resolved or spawn fails.
pub fn spawn_daemon(root: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("watch-daemon")
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().context("failed to spawn watch daemon")?;
    write_pid(root, child.id())?;
    Ok(())
}

/// Accumulates changed relative paths and owns the quiet-window deadline.
#[derive(Debug, Default)]
pub struct ChangeBuffer {
    pending: HashSet<PathBuf>,
    deadline: Option<Instant>,
}

impl ChangeBuffer {
    /// Record changed absolute paths and push the deadline one window out.
    pub fn extend(&mut self, rel_paths: impl IntoIterator<Item = PathBuf>) {
        self.pending.extend(rel_paths);
        self.deadline = Some(Instant::now() + FLUSH_DELAY);
    }

    /// Whether the quiet window has elapsed with pending changes.
    #[must_use]
    pub fn due(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    /// Take the pending set (emptying it); also clears the deadline.
    pub fn take(&mut self) -> Vec<PathBuf> {
        self.deadline = None;
        std::mem::take(&mut self.pending).into_iter().collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Run the watch loop until [`request_stop`] creates the stop file.
///
/// Events are coalesced into [`ChangeBuffer`]; whenever `buffer.due()` turns
/// true, `flush` receives the changed relative paths and must re-index them.
///
/// # Errors
///
/// Fails on watcher setup or if `flush` fails.
pub fn watch_loop<F>(root: &Path, flush: &mut F) -> Result<()>
where
    F: FnMut(&[PathBuf]) -> Result<()>,
{
    let (tx, rx) = mpsc::channel();
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // Only content-relevant mutations interest us.
                if matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                ) {
                    let _ = tx.send(event.paths);
                }
            }
        })
        .context("failed to create file watcher")?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", root.display()))?;

    let stop = stop_path(root);
    let _guard = RemoveOnDrop {
        path: pid_path(root),
    };
    let mut buffer = ChangeBuffer::default();

    loop {
        if stop.exists() {
            let _ = std::fs::remove_file(&stop);
            info!("watch daemon stopping");
            return Ok(());
        }

        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(paths) => {
                let rels: Vec<PathBuf> = paths
                    .into_iter()
                    .filter_map(|p| p.strip_prefix(root).ok().map(Path::to_path_buf))
                    .collect();
                if !rels.is_empty() {
                    debug!(count = rels.len(), "changes detected");
                    buffer.extend(rels);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("watcher channel disconnected"),
        }

        if buffer.due() {
            let changed = buffer.take();
            info!(count = changed.len(), "quiet window closed; flushing");
            flush(&changed)?;
        }
    }
}

struct RemoveOnDrop {
    path: PathBuf,
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests;

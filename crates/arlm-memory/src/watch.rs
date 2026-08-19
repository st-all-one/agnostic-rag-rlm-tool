use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::ScopedTimer;

/// A file change event detected by the watcher.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub paths: Vec<PathBuf>,
    pub kind: WatchEventKind,
}

/// Kind of file system event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEventKind {
    Created,
    Modified,
    Removed,
}

/// Options for the file watcher.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// Debounce interval in milliseconds.
    pub debounce_ms: u64,
    /// Whether to watch subdirectories recursively.
    pub recursive: bool,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            debounce_ms: 500,
            recursive: true,
        }
    }
}

/// Handle to a running watch session. Drop to stop watching.
pub struct WatchHandle {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<WatchEvent>,
}

impl WatchHandle {
    /// Receive the next batch of events (blocks until available).
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is disconnected.
    pub fn recv(&self) -> Result<WatchEvent> {
        self.rx.recv().context("watch channel disconnected")
    }

    /// Try to receive an event without blocking.
    #[must_use]
    pub fn try_recv(&self) -> Option<WatchEvent> {
        self.rx.try_recv().ok()
    }
}

/// Monitors a directory for file changes using inotify.
pub struct WatchMonitor;

impl WatchMonitor {
    /// Start watching a directory for changes.
    ///
    /// # Errors
    ///
    /// Returns an error if the watcher cannot be created or the path is invalid.
    pub fn watch(path: &Path, options: &WatchOptions) -> Result<WatchHandle> {
        let _timer = ScopedTimer::new("watch_start");

        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |result: std::result::Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    let kind = match event.kind {
                        EventKind::Create(_) => WatchEventKind::Created,
                        EventKind::Modify(_) => WatchEventKind::Modified,
                        EventKind::Remove(_) => WatchEventKind::Removed,
                        _ => return,
                    };

                    let watch_event = WatchEvent {
                        paths: event.paths,
                        kind,
                    };

                    // Ignore send errors (receiver dropped)
                    let _ = tx.send(watch_event);
                }
            },
            notify::Config::default()
                .with_poll_interval(Duration::from_millis(options.debounce_ms)),
        )
        .context("failed to create file watcher")?;

        let mode = if options.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        watcher
            .watch(path, mode)
            .with_context(|| format!("failed to watch path: {}", path.display()))?;

        tracing::info!(path = %path.display(), recursive = options.recursive, "file watcher started");

        Ok(WatchHandle {
            _watcher: watcher,
            rx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_watch_and_event() {
        let tmp = TempDir::new().unwrap();
        let opts = WatchOptions {
            debounce_ms: 10,
            recursive: true,
        };

        let handle = WatchMonitor::watch(tmp.path(), &opts).unwrap();

        // Create a file to trigger an event
        std::fs::write(tmp.path().join("new.txt"), "hello").unwrap();

        // Wait for the event with a timeout
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut event_received = false;

        while std::time::Instant::now() < deadline {
            if let Some(event) = handle.try_recv() {
                assert_eq!(event.kind, WatchEventKind::Created);
                event_received = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(event_received, "expected file creation event");
    }

    #[test]
    fn test_watch_options_default() {
        let opts = WatchOptions::default();
        assert_eq!(opts.debounce_ms, 500);
        assert!(opts.recursive);
    }
}

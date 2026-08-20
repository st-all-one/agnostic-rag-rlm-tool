use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::events::RlmEvent;

/// A JSONL event logger that appends one serialized `RlmEvent` per line.
///
/// Each run writes to its own `run_{run_id}.events.jsonl` file, which can be
/// replayed back in order with [`Self::replay`].
#[derive(Debug)]
pub struct JsonlEventLogger {
    writer: Mutex<BufWriter<File>>,
    log_dir: PathBuf,
    log_path: PathBuf,
    run_id: Option<String>,
}

impl JsonlEventLogger {
    /// Create a new logger writing to `run_{run_id}.events.jsonl` under `log_dir`.
    ///
    /// Creates the directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the log file cannot be opened.
    pub fn new(run_id: &str, log_dir: &Path) -> Result<Self> {
        fs::create_dir_all(log_dir)
            .with_context(|| format!("failed to create log dir {}", log_dir.display()))?;
        let log_path = log_dir.join(format!("run_{run_id}.events.jsonl"));
        let file = File::create(&log_path)
            .with_context(|| format!("failed to open event log {}", log_path.display()))?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
            log_dir: log_dir.to_path_buf(),
            log_path,
            run_id: Some(run_id.to_string()),
        })
    }

    /// Serialize `event` as a JSON line, append it to the log, and flush.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or writing fails.
    pub fn log(&self, event: &RlmEvent) -> Result<()> {
        let line = serde_json::to_string(event).context("failed to serialize event")?;
        let mut writer = self.writer.lock();
        writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.write_all(b"\n"))
            .context("failed to write event line")?;
        writer.flush().context("failed to flush event log")
    }

    /// Replay events from a JSONL log file in order, ignoring empty lines.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or a non-empty line fails to deserialize.
    pub fn replay(log_path: &Path) -> Result<Vec<RlmEvent>> {
        let content = fs::read_to_string(log_path)
            .with_context(|| format!("failed to read event log {}", log_path.display()))?;
        content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).context("failed to deserialize event line"))
            .collect()
    }

    /// Return the directory containing this logger's log file.
    #[must_use]
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Return the full path to this logger's log file.
    #[must_use]
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Return the run id this logger was created for, if known.
    #[must_use]
    pub fn run_id(&self) -> Option<&str> {
        self.run_id.as_deref()
    }
}

/// Find all event log files (`*.events.jsonl`) in `log_dir`.
///
/// Missing directories or unreadable entries are silently skipped.
#[must_use]
pub fn files(log_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".events.jsonl"))
        })
        .collect()
}

/// Spawn a background task that writes events received on `rx` to `logger`.
///
/// Write errors are logged as warnings and never propagated. The task exits
/// once the broadcast channel is closed (all senders dropped).
pub fn spawn_event_writer(
    mut rx: broadcast::Receiver<RlmEvent>,
    logger: Arc<JsonlEventLogger>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = logger.log(&event) {
                        warn!("failed to write event to JSONL log: {e}");
                    }
                }
                Err(broadcast::error::RecvError::Lagged(dropped)) => {
                    warn!(dropped, "event writer lagged; dropped events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

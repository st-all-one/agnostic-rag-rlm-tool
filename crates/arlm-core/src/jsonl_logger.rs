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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::events::EventBus;

    fn sample_events() -> Vec<RlmEvent> {
        vec![
            RlmEvent::RunStart {
                run_id: Arc::from("run-test"),
                task: "solve the problem".to_string(),
                backend: "mock".to_string(),
                mode: "auto".to_string(),
                max_depth: 3,
                max_nodes: 10,
                max_budget: 1.0,
                started_at_ms: 1_700_000_000_000,
            },
            RlmEvent::CostUpdate {
                run_id: Arc::from("run-test"),
                spent: 0.5,
                budget: 1.0,
            },
            RlmEvent::RunEnd {
                run_id: Arc::from("run-test"),
                duration_ms: 100,
                nodes_visited: 5,
            },
        ]
    }

    #[test]
    fn test_log_and_replay_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logger = JsonlEventLogger::new("run-1", dir.path()).expect("logger");
        let events = sample_events();

        for event in &events {
            logger.log(event).expect("log");
        }

        let replayed = JsonlEventLogger::replay(logger.log_path()).expect("replay");
        assert_eq!(replayed.len(), events.len());
        for (original, replay) in events.iter().zip(&replayed) {
            let a = serde_json::to_string(original).expect("serialize original");
            let b = serde_json::to_string(replay).expect("serialize replayed");
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_new_creates_directory() {
        let base = tempfile::tempdir().expect("tempdir");
        let nested = base.path().join("a/b/c");
        let logger = JsonlEventLogger::new("run-1", &nested).expect("logger");
        assert!(logger.log_path().is_file());
        assert_eq!(logger.log_dir(), &nested);
        assert_eq!(logger.run_id(), Some("run-1"));
    }

    #[test]
    fn test_files_finds_event_logs() {
        let dir = tempfile::tempdir().expect("tempdir");
        JsonlEventLogger::new("run-1", dir.path()).expect("logger");
        JsonlEventLogger::new("run-2", dir.path()).expect("logger");

        let mut log_files = files(dir.path());
        log_files.sort();
        assert_eq!(log_files.len(), 2);

        let names: Vec<String> = log_files
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(
            names,
            vec!["run_run-1.events.jsonl", "run_run-2.events.jsonl"]
        );
    }

    #[test]
    fn test_files_missing_dir_is_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(files(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn test_replay_ignores_empty_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logger = JsonlEventLogger::new("run-1", dir.path()).expect("logger");
        logger.log(&sample_events()[0]).expect("log");

        std::fs::write(logger.log_path(), b"\n").expect("truncate");
        let replayed = JsonlEventLogger::replay(logger.log_path()).expect("replay");
        assert!(replayed.is_empty());
    }

    #[tokio::test]
    async fn test_spawn_event_writer_writes_all_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logger = Arc::new(JsonlEventLogger::new("run-2", dir.path()).expect("logger"));
        let bus = EventBus::new();
        let handle = spawn_event_writer(bus.subscribe(), logger);
        let mut check_rx = bus.subscribe();

        let events = sample_events();
        let count = events.len();
        for event in &events {
            bus.emit(event.clone());
        }

        for _ in 0..count {
            check_rx.recv().await.expect("should receive event");
        }
        drop(check_rx);
        drop(bus);
        handle.await.expect("writer task should complete");

        let replayed =
            JsonlEventLogger::replay(dir.path().join("run_run-2.events.jsonl").as_path())
                .expect("replay");
        assert_eq!(replayed.len(), count);
    }
}

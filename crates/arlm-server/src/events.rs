//! Server-side event hub.
//!
//! Bridges internal events (RLM engine, summarization) to gRPC streaming
//! clients. Supports per-run and per-summary broadcast channels plus a
//! catch-all channel for `stream_events`.

use std::collections::HashMap;
use std::sync::Arc;

use arlm_proto::proto::{RunEvent, SummarizeProgress};
use parking_lot::Mutex;
use tokio::sync::broadcast;

/// Global event emitted to every `stream_events` subscriber.
#[derive(Clone, Debug)]
pub enum ServerEvent {
    /// A run lifecycle event (node start/end, run start/end, etc.).
    Run(RunEvent),
    /// A summarization progress tick.
    Summarize(SummarizeProgress),
}

const EVENT_BUFFER: usize = 256;

struct EventHubInner {
    runs: Mutex<HashMap<String, broadcast::Sender<RunEvent>>>,
    summaries: Mutex<HashMap<String, broadcast::Sender<SummarizeProgress>>>,
    all: broadcast::Sender<ServerEvent>,
}

/// Thread-safe hub for streaming run and summarization events.
#[derive(Clone)]
pub struct EventHub {
    inner: Arc<EventHubInner>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHub {
    /// Create an empty event hub.
    #[must_use]
    pub fn new() -> Self {
        let (all, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            inner: Arc::new(EventHubInner {
                runs: Mutex::new(HashMap::new()),
                summaries: Mutex::new(HashMap::new()),
                all,
            }),
        }
    }

    /// Subscribe to the catch-all event stream (`stream_events`).
    #[must_use]
    pub fn subscribe_all(&self) -> broadcast::Receiver<ServerEvent> {
        self.inner.all.subscribe()
    }

    /// Emit an event to every `stream_events` subscriber.
    pub fn emit_all(&self, event: ServerEvent) {
        if self.inner.all.receiver_count() == 0 {
            return;
        }
        if let Err(e) = self.inner.all.send(event) {
            tracing::warn!(error = %e, "event hub all-channel send error");
        }
    }

    /// Register a run stream channel; returns a receiver for the caller.
    pub fn register_run(&self, run_id: &str) -> broadcast::Receiver<RunEvent> {
        self.unregister_run(run_id);
        let (tx, rx) = broadcast::channel(EVENT_BUFFER);
        self.inner.runs.lock().insert(run_id.to_string(), tx);
        rx
    }

    /// Remove a run stream channel when it is done.
    pub fn unregister_run(&self, run_id: &str) {
        self.inner.runs.lock().remove(run_id);
    }

    /// Publish a run event to the run's subscribers and the catch-all stream.
    pub fn publish_run(&self, event: RunEvent) {
        let run_id = event.run_id.clone();
        let mut sent = 0usize;
        if let Some(tx) = self.inner.runs.lock().get(&run_id) {
            sent = tx.send(event.clone()).map(|n| n).unwrap_or(0);
        }
        if sent == 0 {
            // No run-specific subscribers: still deliver to catch-all below.
        }
        self.emit_all(ServerEvent::Run(event));
    }

    /// Register a summarization progress channel.
    pub fn register_summarize(&self, run_id: &str) -> broadcast::Receiver<SummarizeProgress> {
        self.unregister_summarize(run_id);
        let (tx, rx) = broadcast::channel(EVENT_BUFFER);
        self.inner.summaries.lock().insert(run_id.to_string(), tx);
        rx
    }

    /// Remove a summarization progress channel.
    pub fn unregister_summarize(&self, run_id: &str) {
        self.inner.summaries.lock().remove(run_id);
    }

    /// Publish a summarization progress tick.
    pub fn publish_summarize(&self, event: SummarizeProgress) {
        if let Some(tx) = self.inner.summaries.lock().get(&event.run_id) {
            let _ = tx.send(event.clone());
        }
        self.emit_all(ServerEvent::Summarize(event));
    }
}
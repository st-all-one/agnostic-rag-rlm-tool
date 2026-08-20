use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use arlm_core::AbortSignal;
use arlm_llm::LlmBackend;
use arlm_storage::Storage;
use arlm_storage::VectorStore;
use parking_lot::Mutex;

use crate::config::ServerConfig;
use crate::events::EventHub;
use crate::summarizer;
use crate::summarizer::SummarizeSender;
use crate::write_queue::WriteQueue;

/// Shared state across gRPC handlers.
#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    pub write_queue: WriteQueue,
    pub config: ServerConfig,
    pub llm: Arc<dyn LlmBackend + Send + Sync>,
    pub events: EventHub,
    /// Sender that triggers background summarization jobs.
    pub summarize_tx: SummarizeSender,
    /// Optional vector store (LanceDB) used by `IndexProject`.
    pub vector_store: Option<Arc<VectorStore>>,
    /// Active run abort signals (keyed by run id) for cancellation.
    runs: Arc<Mutex<HashMap<String, AbortSignal>>>,
    started_at: std::time::Instant,
}

impl AppState {
    /// Create a new `AppState`.
    ///
    /// Builds the configured LLM backend, starts the batched write queue and
    /// the summarization worker, and initialises the event hub. Must be
    /// called inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM backend cannot be built.
    pub fn new(
        storage: Storage,
        config: ServerConfig,
        llm: Arc<dyn LlmBackend + Send + Sync>,
        vector_store: Option<Arc<VectorStore>>,
    ) -> Result<Self> {
        let write_queue = WriteQueue::new(
            storage.clone(),
            std::time::Duration::from_millis(config.flush_interval_ms),
            config.max_batch_size,
        );

        let events = EventHub::new();
        let summarize_tx = summarizer::spawn_worker(storage.clone(), llm.clone(), events.clone());

        Ok(Self {
            storage,
            write_queue,
            config,
            llm,
            events,
            summarize_tx,
            vector_store,
            runs: Arc::new(Mutex::new(HashMap::new())),
            started_at: std::time::Instant::now(),
        })
    }

    /// Build the LLM backend from the active configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM backend cannot be built.
    pub fn build_llm(config: &ServerConfig) -> Result<Arc<dyn LlmBackend + Send + Sync>> {
        config
            .build_llm()
            .context("failed to configure LLM backend")
    }

    /// Register a fresh abort signal for a run (replacing any previous one).
    pub fn register_abort(&self, run_id: &str) -> AbortSignal {
        let signal = AbortSignal::new();
        self.runs.lock().insert(run_id.to_string(), signal.clone());
        signal
    }

    /// Request cancellation of an active run.
    pub fn abort_run(&self, run_id: &str) -> bool {
        let cancelled = self.runs.lock();
        if let Some(signal) = cancelled.get(run_id) {
            signal.cancel();
            true
        } else {
            false
        }
    }

    /// Drop the abort signal when a run finishes.
    pub fn release_run(&self, run_id: &str) {
        self.runs.lock().remove(run_id);
    }

    /// Seconds since the server started.
    #[must_use]
    pub fn uptime_seconds(&self) -> u64 {
        u64::try_from(self.started_at.elapsed().as_secs()).unwrap_or(0)
    }
}

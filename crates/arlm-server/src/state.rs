use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use arlm_core::AbortSignal;
use arlm_embedding::embedder::{Embedder, bge_m3, fallback};
use arlm_llm::LlmBackend;
use arlm_storage::Storage;
use arlm_storage::VectorStore;
use parking_lot::Mutex;

use crate::config::ServerConfig;
use crate::events::EventHub;
use crate::summarizer;
use crate::summarizer::SummarizeSender;

/// Shared state across gRPC handlers.
#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    pub config: ServerConfig,
    pub llm: Arc<dyn LlmBackend + Send + Sync>,
    pub events: EventHub,
    /// Sender that triggers background summarization jobs.
    pub summarize_tx: SummarizeSender,
    /// Optional vector store (LanceDB) used by `IndexProject`.
    pub vector_store: Option<Arc<VectorStore>>,
    /// Embedder used for chunk (index) and query (search) embeddings.
    /// Real BGE-M3 when `ARLM_MODEL_DIR` points at a directory containing
    /// `model.safetensors` + `tokenizer.json`; otherwise a hash fallback that
    /// keeps the pipeline running without semantic search.
    pub embedder: Arc<dyn Embedder + Send + Sync>,
    /// Active run abort signals (keyed by run id) for cancellation.
    runs: Arc<Mutex<HashMap<String, AbortSignal>>>,
    started_at: std::time::Instant,
}

/// Build the embedder: BGE-M3 when weights are available, else hash fallback.
fn load_embedder() -> Arc<dyn Embedder + Send + Sync> {
    const DIMS: usize = 1024;
    match std::env::var("ARLM_MODEL_DIR").ok().map(PathBuf::from) {
        Some(dir) if dir.join("model.safetensors").exists() => {
            match bge_m3::BgeM3Embedder::new(&dir, DIMS) {
                Ok(embedder) => {
                    tracing::info!(model_dir = %dir.display(), "loaded BGE-M3 embedder");
                    Arc::new(embedder)
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "BGE-M3 load failed, falling back to hash embedder"
                    );
                    Arc::new(fallback::FallbackEmbedder::new(DIMS))
                }
            }
        }
        Some(dir) => {
            tracing::warn!(
                model_dir = %dir.display(),
                "ARLM_MODEL_DIR set but model.safetensors missing; using hash embedder"
            );
            Arc::new(fallback::FallbackEmbedder::new(DIMS))
        }
        None => {
            tracing::info!("ARLM_MODEL_DIR not set; using fallback hash embedder");
            Arc::new(fallback::FallbackEmbedder::new(DIMS))
        }
    }
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
        let events = EventHub::new();
        let summarize_tx = summarizer::spawn_worker(storage.clone(), llm.clone(), events.clone());
        let embedder = load_embedder();

        Ok(Self {
            storage,
            config,
            llm,
            events,
            summarize_tx,
            vector_store,
            embedder,
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

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

/// Build the embedder: Ollama when configured, else BGE-M3 (quantized) when
/// weights are available, else a hash fallback.
fn load_embedder() -> Arc<dyn Embedder + Send + Sync> {
    const DIMS: usize = 1024;

    // Ollama backend (laptop-friendly): enabled via ARLM_OLLAMA_MODEL.
    if let Ok(model) = std::env::var("ARLM_OLLAMA_MODEL") {
        let url = std::env::var("ARLM_OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let dims = std::env::var("ARLM_OLLAMA_DIMS")
            .ok()
            .and_then(|d| d.parse::<usize>().ok())
            .unwrap_or(768);
        let prefix = std::env::var("ARLM_OLLAMA_PREFIX")
            .unwrap_or_else(|_| "search_document: ".to_string());
        let cfg = arlm_embedding::embedder::config::EmbeddingConfig {
            model: arlm_embedding::embedder::config::EmbeddingModel::Ollama,
            quantization: arlm_embedding::embedder::config::Quantization::None,
            matryoshka_dims: None,
            model_dir: None,
            dims,
            ollama_url: Some(url.clone()),
            ollama_model: Some(model.clone()),
            ollama_prefix: Some(prefix),
        };
        match arlm_embedding::embedder::config::build_embedder(&cfg) {
            Ok(embedder) => {
                tracing::info!(model = "ollama", ollama_model = %model, "loaded Ollama embedder");
                return embedder;
            }
            Err(err) => {
                tracing::warn!(error = %err, "Ollama embedder failed; falling back");
            }
        }
    }

    match std::env::var("ARLM_MODEL_DIR").ok().map(PathBuf::from) {
        Some(dir) if dir.join("model.safetensors").exists() => {
            // Quantize to INT8 at load time: runs real BGE-M3 semantics via
            // `QMatMul` at ~3-4x less CPU/RAM than FP32 (set ARLM_MODEL_QUANT
            // to override). FP32 ("none") is far too slow for CPU indexing.
            let quant = match std::env::var("ARLM_MODEL_QUANT").as_deref() {
                Ok("none") => arlm_embedding::embedder::config::Quantization::None,
                Ok("int4") => arlm_embedding::embedder::config::Quantization::Int4,
                _ => arlm_embedding::embedder::config::Quantization::Int8,
            };
            let cfg = arlm_embedding::embedder::config::EmbeddingConfig {
                model: arlm_embedding::embedder::config::EmbeddingModel::BgeM3,
                quantization: quant,
                matryoshka_dims: Some(DIMS),
                model_dir: Some(dir.clone()),
                dims: DIMS,
                ollama_url: None,
                ollama_model: None,
                ollama_prefix: None,
            };
            match bge_m3::BgeM3Embedder::new_with_config(&dir, &cfg) {
                Ok(embedder) => {
                    tracing::info!(
                        model_dir = %dir.display(),
                        quantization = ?quant,
                        "loaded BGE-M3 embedder"
                    );
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

/// Dimensionality of the embedder [`load_embedder`] will build, used to size
/// the server's global vector store so stored and query vectors are comparable.
#[must_use]
pub fn embedder_dimension() -> usize {
    if std::env::var("ARLM_OLLAMA_MODEL").is_ok() {
        std::env::var("ARLM_OLLAMA_DIMS")
            .ok()
            .and_then(|d| d.parse::<usize>().ok())
            .unwrap_or(768)
    } else {
        1024
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

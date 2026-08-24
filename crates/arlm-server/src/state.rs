use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arlm_embedding::embedder::{Embedder, bge_m3, fallback};
use arlm_storage::QuestionVectorStore;
use arlm_storage::Storage;
use arlm_storage::VectorStore;

use crate::config::{QaCacheConfig, ServerConfig};

/// Shared state across gRPC handlers.
#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    pub config: ServerConfig,
    /// Optional vector store (LanceDB) used by `IndexProject`.
    pub vector_store: Option<Arc<VectorStore>>,
    /// Question-vector index (plan 017) for semantic cache lookup, in its own
    /// cosine space, separate from the chunk vector store.
    pub question_vector_store: Option<Arc<QuestionVectorStore>>,
    /// Embedder used for chunk (index) and query (search) embeddings.
    /// Real BGE-M3 when `ARLM_MODEL_DIR` points at a directory containing
    /// `model.safetensors` + `tokenizer.json`; otherwise a hash fallback that
    /// keeps the pipeline running without semantic search.
    pub embedder: Arc<dyn Embedder + Send + Sync>,
    /// Semantic query-answer cache tunables (plan 017).
    pub qa_config: QaCacheConfig,
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
        let prefix =
            std::env::var("ARLM_OLLAMA_PREFIX").unwrap_or_else(|_| "search_document: ".to_string());
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
    /// Loads the embedder and starts the background semantic-cache eviction
    /// worker. Must be called inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage handle cannot be cloned for the
    /// eviction worker.
    pub fn new(
        storage: Storage,
        config: ServerConfig,
        vector_store: Option<Arc<VectorStore>>,
        question_vector_store: Option<Arc<QuestionVectorStore>>,
    ) -> Result<Self> {
        let embedder = load_embedder();
        let qa_config = config.qa_cache.clone();

        let state = Self {
            storage: storage.clone(),
            config,
            vector_store,
            question_vector_store,
            embedder,
            qa_config: qa_config.clone(),
            started_at: std::time::Instant::now(),
        };

        spawn_eviction_worker(storage, qa_config);
        Ok(state)
    }

    /// Seconds since the server started.
    #[must_use]
    pub fn uptime_seconds(&self) -> u64 {
        u64::try_from(self.started_at.elapsed().as_secs()).unwrap_or(0)
    }
}

/// Spawn the background weighted-LRU eviction worker for the semantic cache.
///
/// Eviction runs on a fixed interval (disabled when `eviction_interval_ms == 0`)
/// and is best-effort: any failure is logged and retried next tick.
fn spawn_eviction_worker(storage: Storage, qa_config: QaCacheConfig) {
    if qa_config.eviction_interval_ms == 0 {
        return;
    }
    let interval = std::time::Duration::from_millis(qa_config.eviction_interval_ms);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            if let Err(e) = storage.evict_all_qa(
                qa_config.max_entries_per_project,
                qa_config.eviction_lambda_ms,
            ) {
                tracing::warn!(error = %e, "qa_cache eviction tick failed");
            }
        }
    });
}

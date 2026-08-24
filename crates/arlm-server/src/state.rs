use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arlm_embedding::embedder::{Embedder, bge_m3, fallback};
use arlm_storage::QuestionVectorStore;
use arlm_storage::Storage;
use arlm_storage::VectorStore;

use crate::config::{EmbedderModel, QaCacheConfig, ServerConfig};

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
    /// Embedder used for chunk (index) and query (search) embeddings. Built
    /// from `server.toml [embedder]` (plan 020): real BGE-M3 when
    /// `[embedder] model = "bge-m3"` and `model_dir` contains weights;
    /// Ollama when `model = "ollama"`; otherwise a hash fallback that keeps
    /// the pipeline running without semantic search.
    pub embedder: Arc<dyn Embedder + Send + Sync>,
    /// Semantic query-answer cache tunables (plan 017).
    pub qa_config: QaCacheConfig,
    started_at: std::time::Instant,
}

/// Build the embedder from the `[embedder]` section of `server.toml`
/// (plan 020): Ollama when `model = "ollama"`, BGE-M3 (quantized) when
/// `model = "bge-m3"` and weights are available, else a hash fallback.
fn load_embedder(cfg: &crate::config::EmbedderConfig) -> Arc<dyn Embedder + Send + Sync> {
    use arlm_embedding::embedder::config::{
        EmbeddingConfig, EmbeddingModel as CfgModel, Quantization,
    };

    let dims = cfg.dims;
    match cfg.resolved_model() {
        EmbedderModel::Ollama => {
            let url = cfg
                .ollama_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = cfg
                .ollama_model
                .clone()
                .unwrap_or_else(|| "nomic-embed-text-v2-moe".to_string());
            let prefix = Some(
                cfg.ollama_prefix
                    .clone()
                    .unwrap_or_else(|| "search_document: ".to_string()),
            );
            let emb_cfg = EmbeddingConfig {
                model: CfgModel::Ollama,
                quantization: Quantization::None,
                matryoshka_dims: None,
                model_dir: None,
                dims,
                ollama_url: Some(url.clone()),
                ollama_model: Some(model.clone()),
                ollama_prefix: prefix,
            };
            match arlm_embedding::embedder::config::build_embedder(&emb_cfg) {
                Ok(embedder) => {
                    tracing::info!(model = "ollama", ollama_model = %model, %url, "loaded Ollama embedder");
                    return embedder;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "Ollama embedder failed; falling back");
                }
            }
        }
        EmbedderModel::BgeM3 => {
            if let Some(dir) = cfg.model_dir.clone().map(PathBuf::from) {
                if dir.join("model.safetensors").exists() {
                    // Quantize to INT8 at load time: runs real BGE-M3 semantics
                    // via `QMatMul` at ~3-4x less CPU/RAM than FP32.
                    let quant = cfg.resolved_quantization();
                    let emb_cfg = EmbeddingConfig {
                        model: CfgModel::BgeM3,
                        quantization: quant,
                        matryoshka_dims: Some(dims),
                        model_dir: Some(dir.clone()),
                        dims,
                        ollama_url: None,
                        ollama_model: None,
                        ollama_prefix: None,
                    };
                    match bge_m3::BgeM3Embedder::new_with_config(&dir, &emb_cfg) {
                        Ok(embedder) => {
                            tracing::info!(
                                model_dir = %dir.display(),
                                quantization = ?quant,
                                "loaded BGE-M3 embedder"
                            );
                            return Arc::new(embedder);
                        }
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                "BGE-M3 load failed, falling back to hash embedder"
                            );
                        }
                    }
                } else {
                    tracing::warn!(
                        model_dir = %dir.display(),
                        "model.safetensors missing in [embedder].model_dir; using hash embedder"
                    );
                }
            } else {
                tracing::warn!(
                    "[embedder] model = \"bge-m3\" without model_dir; using hash embedder"
                );
            }
        }
        EmbedderModel::Lightweight => {
            tracing::info!("[embedder] model = \"lightweight\"; using hash embedder");
        }
    }

    Arc::new(fallback::FallbackEmbedder::new(dims))
}

/// Dimensionality of the embedder built for `cfg`, used to size the server's
/// global vector stores so stored and query vectors are comparable.
#[must_use]
pub fn embedder_dimension(cfg: &crate::config::EmbedderConfig) -> usize {
    cfg.dims
}

/// Wrap the embedder with the SQLite content-hash cache when
/// `server.toml [embedder] cache = true` (plan 020). Cache failures degrade
/// to the uncached embedder so indexing never stops because of the cache.
fn wrap_with_cache(
    embedder: Arc<dyn Embedder + Send + Sync>,
    config: &ServerConfig,
) -> Arc<dyn Embedder + Send + Sync> {
    if !config.embedder.cache {
        tracing::info!("[embedder] cache = false; running without embedding cache");
        return embedder;
    }
    let db_path = config.data_dir.join("embedding-cache.db");
    match arlm_embedding::embedder::cache::EmbeddingCache::open(
        &db_path.to_string_lossy(),
        config.embedder.dims,
    ) {
        Ok(cache) => {
            tracing::info!(db = %db_path.display(), dims = config.embedder.dims, "embedding cache enabled");
            Arc::new(arlm_embedding::embedder::cache::CachedEmbedder::new(
                embedder, cache,
            ))
        }
        Err(e) => {
            tracing::warn!(error = %e, "embedding cache open failed; running uncached");
            embedder
        }
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
        let embedder = load_embedder(&config.embedder);
        let embedder = wrap_with_cache(embedder, &config);
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

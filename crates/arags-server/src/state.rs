use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arags_embedding::embedder::{Embedder, MinilmEmbedder, fallback};
use arags_storage::QuestionVectorStore;
use arags_storage::RlmVectorStore;
use arags_storage::Storage;
use arags_storage::VectorStore;

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
    /// RLM summary-vector index (own cosine space, separate from chunks and
    /// the QA question index).
    pub rlm_vector_store: Option<Arc<RlmVectorStore>>,
    /// Embedder used for chunk (index) and query (search) embeddings. Built
    /// from `server.toml [embedder]`: the native all-`MiniLM`-L6-v2 checkpoint
    /// at `model_dir`; a hash fallback that keeps the pipeline running
    /// without semantic search when the weights are missing/unloadable.
    pub embedder: Arc<dyn Embedder + Send + Sync>,
    /// Semantic query-answer cache tunables (plan 017).
    pub qa_config: QaCacheConfig,
    started_at: std::time::Instant,
}

/// Build the embedder from the `[embedder]` section of `server.toml`:
/// the native all-`MiniLM`-L6-v2 checkpoint at `model_dir` when present,
/// else a hash fallback.
fn load_embedder(cfg: &crate::config::EmbedderConfig) -> Arc<dyn Embedder + Send + Sync> {
    if let Some(dir) = cfg.model_dir.clone().map(PathBuf::from) {
        if dir.join("model.safetensors").exists() {
            // Quantize to INT8 by default: `QMatMul` runs `MiniLM` at a
            // fraction of the f32 CPU/RAM cost with negligible quality loss.
            let quant = cfg.resolved_quantization();
            match MinilmEmbedder::new(&dir, quant) {
                Ok(embedder) => {
                    tracing::info!(
                        model_dir = %dir.display(),
                        ?quant,
                        "loaded all-MiniLM-L6-v2 embedder"
                    );
                    return Arc::new(embedder);
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "`MiniLM` load failed, falling back to hash embedder"
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
        tracing::warn!("[embedder] without model_dir; using hash embedder");
    }

    Arc::new(fallback::FallbackEmbedder::new(
        arags_embedding::embedder::minilm::HIDDEN_SIZE,
    ))
}

/// Dimensionality of the embedding model (all-`MiniLM`-L6-v2 → 384), used to
/// size the server's global vector stores so stored and query vectors are
/// comparable.
#[must_use]
pub fn embedder_dimension() -> usize {
    arags_embedding::embedder::minilm::HIDDEN_SIZE
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
    match arags_embedding::embedder::cache::EmbeddingCache::open(
        &db_path.to_string_lossy(),
        embedder_dimension(),
    ) {
        Ok(cache) => {
            tracing::info!(db = %db_path.display(), dims = embedder_dimension(), "embedding cache enabled");
            Arc::new(arags_embedding::embedder::cache::CachedEmbedder::new(
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
        Self::with_rlm_vectors(storage, config, vector_store, question_vector_store, None)
    }

    /// Full constructor including the optional RLM summary-vector store.
    ///
    /// # Errors
    ///
    /// Returns an error if the embedder cannot be built.
    pub fn with_rlm_vectors(
        storage: Storage,
        config: ServerConfig,
        vector_store: Option<Arc<VectorStore>>,
        question_vector_store: Option<Arc<QuestionVectorStore>>,
        rlm_vector_store: Option<Arc<RlmVectorStore>>,
    ) -> Result<Self> {
        let embedder = load_embedder(&config.embedder);
        let embedder = wrap_with_cache(embedder, &config);
        let qa_config = config.qa_cache.clone();

        let state = Self {
            storage: storage.clone(),
            config,
            vector_store,
            question_vector_store,
            rlm_vector_store,
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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use tracing::{info, warn};

use anyhow::Context;
use anyhow::Result;
use arags_embedding::embedder::{Embedder, MinilmEmbedder, fallback};
use arags_storage::QuestionVectorStore;
use arags_storage::RlmVectorStore;
use arags_storage::Storage;
use arags_storage::VectorStore;
use rayon::ThreadPool;

use crate::config::{QaCacheConfig, ServerConfig};
use crate::ratelimit::RateLimiter;

pub use arags_storage::ExplorationVectorStore;

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
    /// Exploration-map vector index (plan 022, own cosine space).
    pub exploration_vector_store: Option<Arc<ExplorationVectorStore>>,
    /// Embedder used for chunk (index) and query (search) embeddings. Built
    /// from `server.toml [embedder]`: the native all-`MiniLM`-L6-v2 checkpoint
    /// at `model_dir`; a hash fallback that keeps the pipeline running
    /// without semantic search when the weights are missing/unloadable.
    pub embedder: Arc<dyn Embedder + Send + Sync>,
    /// Semantic query-answer cache tunables (plan 017).
    pub qa_config: QaCacheConfig,
    /// Dedicated (capped) rayon pool for **index (Phase-2) embedding** (issue
    /// `agnostic-rlm-rs-6690`). Running the embed inside `pool.install(...)`
    /// keeps candle's internal `rayon::join` matmul off the global rayon pool,
    /// so concurrent query embeds (which run on the global pool) keep serving
    /// while a large `arags index` is in flight.
    pub index_embed_pool: Arc<ThreadPool>,
    /// Number of index embed batches currently in flight. Read by the query
    /// path to surface contention (debug) and to gate backpressure so a query
    /// never hangs behind a saturating index (issue `agnostic-rlm-rs-6690`).
    pub active_index_embeds: Arc<AtomicUsize>,
    /// Per-user fixed-window rate limiter for mutating RPCs (issue
    /// `agnostic-rlm-rs-7222`). Always present; a disabled config is a no-op
    /// pass via [`RateLimiter::check`].
    pub rate_limiter: Arc<RateLimiter>,
    started_at: std::time::Instant,
}

/// Build the embedder from the `[embedder]` section of `server.toml`:
/// a local Ollama daemon (`kind = "ollama"`, e.g. `all-minilm:22m`), the
/// native all-`MiniLM`-L6-v2 checkpoint at `model_dir` (candle, default), or
/// a hash fallback when no usable backend is configured.
pub(crate) fn load_embedder(
    cfg: &crate::config::EmbedderConfig,
) -> Arc<dyn Embedder + Send + Sync> {
    use arags_embedding::embedder::{EmbeddingConfig, EmbeddingModel};

    let kind = cfg.kind.clone().unwrap_or_else(|| {
        // Default (portable) backend: candle `Minilm` when its weights are
        // present, otherwise the hash fallback. The self-contained llama.cpp
        // backend is opt-in via explicit `kind = "llamacpp"` (built with the
        // `llamacpp`/`llamacpp-vulkan` feature) so the shipped binary stays
        // free of the C++/Vulkan toolchain.
        if cfg
            .model_dir
            .as_ref()
            .is_some_and(|d| PathBuf::from(d).join("model.safetensors").exists())
        {
            "minilm".to_string()
        } else {
            "fallback".to_string()
        }
    });

    match kind.as_str() {
        "ollama" => {
            let embed_cfg = EmbeddingConfig {
                model: EmbeddingModel::Ollama,
                model_dir: None,
                quantization: cfg.resolved_quantization(),
                ollama_url: cfg.ollama_url.clone(),
                ollama_model: cfg.ollama_model.clone(),
                #[cfg(feature = "llamacpp")]
                llama_cpp_model: None,
                #[cfg(feature = "llamacpp")]
                llama_cpp_gpu_layers: 99,
            };
            match arags_embedding::embedder::build_embedder(&embed_cfg) {
                Ok(embedder) => {
                    info!(
                        model = embedder.name(),
                        dims = embedder.dimensions(),
                        "loaded ollama embedder"
                    );
                    return embedder;
                }
                Err(err) => {
                    warn!(
                        error = %err,
                        "ollama embedder failed to load; using hash embedder"
                    );
                }
            }
        }
        "minilm" => {
            if let Some(dir) = cfg.model_dir.clone().map(PathBuf::from) {
                if dir.join("model.safetensors").exists() {
                    let quant = cfg.resolved_quantization();
                    match MinilmEmbedder::new(&dir, quant) {
                        Ok(embedder) => {
                            info!(
                                model_dir = %dir.display(),
                                ?quant,
                                "loaded minilm embedder"
                            );
                            return Arc::new(embedder);
                        }
                        Err(err) => {
                            warn!(
                                error = %err,
                                "`MiniLM` load failed, falling back to hash embedder"
                            );
                        }
                    }
                } else {
                    warn!(
                        model_dir = %dir.display(),
                        "model.safetensors missing; using hash embedder"
                    );
                }
            } else {
                warn!("[embedder] kind=minilm without model_dir; using hash embedder");
            }
        }
        "lightweight" => {
            return Arc::new(fallback::FallbackEmbedder::new(
                arags_embedding::embedder::minilm::HIDDEN_SIZE,
            ));
        }
        #[cfg(feature = "llamacpp")]
        "llamacpp" => {
            let Some(model_path) = cfg.llama_cpp_model.clone() else {
                warn!("[embedder] kind=llamacpp without llama_cpp_model; using hash embedder");
                return Arc::new(fallback::FallbackEmbedder::new(
                    arags_embedding::embedder::minilm::HIDDEN_SIZE,
                ));
            };
            let model_path = PathBuf::from(model_path);
            let gpu_layers = cfg.llama_cpp_gpu_layers.unwrap_or(99);
            let embed_cfg = EmbeddingConfig {
                model: EmbeddingModel::LlamaCpp,
                model_dir: None,
                quantization: cfg.resolved_quantization(),
                ollama_url: None,
                ollama_model: None,
                llama_cpp_model: Some(model_path.clone()),
                llama_cpp_gpu_layers: gpu_layers,
            };
            match arags_embedding::embedder::build_embedder(&embed_cfg) {
                Ok(embedder) => {
                    info!(
                        model = embedder.name(),
                        dims = embedder.dimensions(),
                        "loaded llama.cpp embedder"
                    );
                    return embedder;
                }
                Err(err) => {
                    warn!(
                        error = %err,
                        "llama.cpp embedder failed to load; using hash embedder"
                    );
                }
            }
        }
        _ => {
            warn!(kind = %kind, "[embedder] unknown kind; using hash embedder");
        }
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

/// Build the **capped** rayon pool used for index (Phase-2) embedding (issue
/// `agnostic-rlm-rs-6690`).
///
/// candle's internal matmul uses `rayon::join`, which runs on the *currently
/// installed* rayon pool. By spawning index embeds inside
/// `pool.install(|| embedder.embed_batch(...))`, the index work is confined to
/// `config.index_embed_threads` cores, leaving the global rayon pool (full core
/// count) free for concurrent query embedding — so a large `arags index` can no
/// longer saturate every core and starve `arags search`.
///
/// # Errors
///
/// Returns an error if the rayon pool cannot be built.
fn build_index_embed_pool(cfg: &ServerConfig) -> anyhow::Result<Arc<ThreadPool>> {
    let threads = cfg.index_embed_threads.max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("idx-embed-{i}"))
        .build()
        .context("failed to build capped index-embed rayon pool")?;
    Ok(Arc::new(pool))
}

/// Wrap the embedder with the SQLite content-hash cache when
/// `server.toml [embedder] cache = true` (plan 020). Cache failures degrade
/// to the uncached embedder so indexing never stops because of the cache.
fn wrap_with_cache(
    embedder: Arc<dyn Embedder + Send + Sync>,
    config: &ServerConfig,
) -> Arc<dyn Embedder + Send + Sync> {
    if !config.embedder.cache {
        info!("[embedder] cache = false; running without embedding cache");
        return embedder;
    }
    let db_path = config.data_dir.join("embedding-cache.db");
    let dims = embedder.dimensions();
    let cache_start = std::time::Instant::now();
    match arags_embedding::embedder::cache::EmbeddingCache::open(&db_path.to_string_lossy(), dims) {
        Ok(cache) => {
            info!(
                db = %db_path.display(),
                dims,
                duration_ms = %cache_start.elapsed().as_millis(),
                "embedding cache enabled"
            );
            Arc::new(arags_embedding::embedder::cache::CachedEmbedder::new(
                embedder, cache,
            ))
        }
        Err(e) => {
            warn!(
                error = %e,
                duration_ms = %cache_start.elapsed().as_millis(),
                "embedding cache open failed; running uncached"
            );
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
        Self::with_vector_stores(
            storage,
            config,
            vector_store,
            question_vector_store,
            None,
            None,
        )
    }

    /// Full constructor including the optional RLM and exploration vector
    /// stores (plans 018/022). The embedder is built from the `[embedder]`
    /// section (candle `Minilm`, Ollama, llama.cpp or hash fallback).
    ///
    /// # Errors
    ///
    /// Returns an error if the embedder cannot be built or the capped index
    /// embed pool cannot be created.
    pub fn with_vector_stores(
        storage: Storage,
        config: ServerConfig,
        vector_store: Option<Arc<VectorStore>>,
        question_vector_store: Option<Arc<QuestionVectorStore>>,
        rlm_vector_store: Option<Arc<RlmVectorStore>>,
        exploration_vector_store: Option<Arc<ExplorationVectorStore>>,
    ) -> Result<Self> {
        let embedder = load_embedder(&config.embedder);
        let embedder = wrap_with_cache(embedder, &config);
        Self::with_embedder(
            storage,
            config,
            embedder,
            vector_store,
            question_vector_store,
            rlm_vector_store,
            exploration_vector_store,
        )
    }

    /// Constructor with an explicit embedder (used by tests and by callers that
    /// want to inject a non-default embedder, e.g. the lightweight fixture).
    /// Builds the capped index-embed rayon pool (issue `agnostic-rlm-rs-6690`).
    ///
    /// # Errors
    ///
    /// Returns an error if the capped index embed pool cannot be created.
    pub fn with_embedder(
        storage: Storage,
        config: ServerConfig,
        embedder: Arc<dyn Embedder + Send + Sync>,
        vector_store: Option<Arc<VectorStore>>,
        question_vector_store: Option<Arc<QuestionVectorStore>>,
        rlm_vector_store: Option<Arc<RlmVectorStore>>,
        exploration_vector_store: Option<Arc<ExplorationVectorStore>>,
    ) -> Result<Self> {
        let pool_start = std::time::Instant::now();
        let index_embed_pool = build_index_embed_pool(&config)?;
        let pool_ms = pool_start.elapsed().as_millis();
        let qa_config = config.qa_cache.clone();
        let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit));

        let state = Self {
            storage: storage.clone(),
            config,
            vector_store,
            question_vector_store,
            rlm_vector_store,
            exploration_vector_store,
            embedder,
            qa_config: qa_config.clone(),
            index_embed_pool,
            active_index_embeds: Arc::new(AtomicUsize::new(0)),
            rate_limiter,
            started_at: std::time::Instant::now(),
        };

        info!(
            index_embed_threads = state.index_embed_pool.current_num_threads(),
            total_cpus = num_cpus::get(),
            duration_ms = %pool_ms,
            "built capped index-embed rayon pool (reserves cores for query serving)"
        );

        spawn_eviction_worker(storage, qa_config);
        Ok(state)
    }

    /// The reserved thread count of the capped index-embed rayon pool.
    #[must_use]
    pub fn index_embed_threads(&self) -> usize {
        self.index_embed_pool.current_num_threads()
    }

    /// Whether an index embedding batch is currently in flight (used by the
    /// query path for backpressure/observability, issue `agnostic-rlm-rs-6690`).
    #[must_use]
    pub fn index_embed_in_flight(&self) -> usize {
        self.active_index_embeds
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Seconds since the server started.
    #[must_use]
    pub fn uptime_seconds(&self) -> u64 {
        u64::try_from(self.started_at.elapsed().as_secs()).unwrap_or(0)
    }

    /// Current Unix epoch seconds (used as the rate-limiter clock).
    #[must_use]
    pub fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Check the per-user rate limit for a mutating RPC. Returns `true` if the
    /// caller may proceed; `false` means the request must be rejected with
    /// `RESOURCE_EXHAUSTED`.
    #[must_use]
    pub fn check_rate_limit(&self, username: &str, now: u64) -> bool {
        self.rate_limiter.check(username, now)
    }

    /// Best-effort audit-log write (issue `agnostic-rlm-rs-7222`). A failure to
    /// record the entry is warned and ignored — it MUST never fail the request
    /// being audited.
    pub fn audit(
        &self,
        project: &str,
        username: &str,
        action: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) {
        if let Err(e) = self
            .storage
            .write_audit_log(project, username, action, target, detail)
        {
            warn!(
                error = %e,
                action,
                username,
                "audit log write failed (request still succeeded)"
            );
        }
    }

    /// Force-flush any debounced vector-index mutations to disk (graceful
    /// shutdown). Best-effort: failures are logged, never fatal.
    pub fn flush_vector_stores(&self) {
        fn flush(name: &str, store: Option<&impl arags_storage::FlushableVectorSpace>) {
            if let Some(store) = store {
                if store.is_dirty() {
                    if let Err(e) = store.persist() {
                        warn!(error = %e, space = name, "vector index flush failed");
                    }
                }
            }
        }
        flush("question_vectors", self.question_vector_store.as_deref());
        flush("rlm_vectors", self.rlm_vector_store.as_deref());
        flush(
            "exploration_vectors",
            self.exploration_vector_store.as_deref(),
        );
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
                warn!(error = %e, "qa_cache eviction tick failed");
            }
        }
    });
}

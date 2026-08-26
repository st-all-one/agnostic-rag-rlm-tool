use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Server configuration loaded from TOML.
///
/// Plan 020: this is the **server-only data-plane** file (`server.toml`, a
/// host file mounted into the container at `/etc/arags/server.toml`). It owns
/// everything that touches data — serving (listen/tls), storage (data_dir),
/// processing ([embedder]) and serving defaults ([search]). It has no LLM
/// section (the server is LLM-free) and is disjoint from the user files
/// (`~/.arags/arags.toml` / `.arags.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Address to listen on (e.g., "127.0.0.1:50051").
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// Data directory for SQLite and LanceDB.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Optional PEM certificate path. Enables TLS when set together with
    /// `tls_key`.
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,

    /// Optional PEM private key path. Enables TLS when set together with
    /// `tls_cert`.
    #[serde(default)]
    pub tls_key: Option<PathBuf>,

    /// Optional PEM CA bundle. When set together with TLS, clients must
    /// present a certificate signed by this CA (mutual TLS).
    #[serde(default)]
    pub mtls_ca: Option<PathBuf>,

    /// SQLite writer pool size (plan 020 "Armazenamento / dados"). `1`
    /// degrades to single-connection mode.
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Interval for the background WAL flush (`PRAGMA wal_checkpoint
    /// (PASSIVE)`), in milliseconds. `0` disables the flusher.
    #[serde(default = "default_flush_interval_ms")]
    pub flush_interval_ms: u64,

    /// Maximum number of chunk rows per write transaction during indexing.
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,

    /// Server-side chunking + embedding parameters (plan 020). The server
    /// owns all data-plane processing.
    #[serde(default)]
    pub embedder: EmbedderConfig,

    /// Search serving defaults (plan 020), applied when a request omits them.
    #[serde(default)]
    pub search: SearchConfig,

    /// Semantic query-answer cache configuration (plan 017).
    #[serde(default)]
    pub qa_cache: QaCacheConfig,

    /// Background memory maintenance (plan 019, C.1): consolidate + decay.
    #[serde(default)]
    pub maintenance: MaintenanceConfig,

    /// Query-history retention (plan 020): rows older than `retention_days`
    /// are purged by the maintenance ticker. `0` keeps history forever.
    #[serde(default)]
    pub history: HistoryConfig,

    /// RLM recursive summaries: volunteer processing pipeline.
    #[serde(default)]
    pub rlm: RlmConfig,

    /// Explorations dataset (plan 022): confidence + feedback knobs.
    #[serde(default)]
    pub exploration: ExplorationConfig,
}

/// Exploration dataset knobs (plan 022).
#[derive(Debug, Clone, Deserialize)]
pub struct ExplorationConfig {
    /// Master switch: `false` rejects persists and skips the staleness hook.
    #[serde(default = "default_exploration_enabled")]
    pub enabled: bool,

    /// Similarity at/above which a map surfaces spontaneously (plan 022).
    #[serde(default = "default_hit_high")]
    pub hit_high: f32,

    /// Similarity below which nothing surfaces; between the bounds a map is
    /// returned as "possibly related".
    #[serde(default = "default_hit_low")]
    pub hit_low: f32,

    /// Age (days) at which the full age decay applies.
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u32,

    /// Contradictions needed to auto-retire a map (`0` disables retirement).
    #[serde(default = "default_contradiction_limit")]
    pub contradiction_limit: i64,

    /// Lazy verify-on-hit (plan 022.8): ground each surfaced map's claim
    /// against current chunk vectors; weak evidence forces `stale`.
    #[serde(default)]
    pub verify_on_hit: bool,

    /// Minimum chunk-space similarity for the claim to count as grounded.
    #[serde(default = "default_grounding_min")]
    pub grounding_min_similarity: f32,

    /// Review gate (plan 023, borrowed from the RLM quality gate): maps from
    /// non-admin submitters land as `pending_review` and never surface in
    /// search until an admin approves them. `false` keeps fire-and-forget.
    #[serde(default)]
    pub require_review: bool,
}

fn default_grounding_min() -> f32 {
    0.25
}

impl Default for ExplorationConfig {
    fn default() -> Self {
        Self {
            enabled: default_exploration_enabled(),
            hit_high: default_hit_high(),
            hit_low: default_hit_low(),
            max_age_days: default_max_age_days(),
            contradiction_limit: default_contradiction_limit(),
            verify_on_hit: false,
            grounding_min_similarity: default_grounding_min(),
            require_review: false,
        }
    }
}

fn default_exploration_enabled() -> bool {
    true
}
fn default_hit_high() -> f32 {
    0.72
}
fn default_hit_low() -> f32 {
    0.55
}
fn default_max_age_days() -> u32 {
    90
}
fn default_contradiction_limit() -> i64 {
    3
}

/// RLM recursive-summary pipeline knobs.
#[derive(Debug, Clone, Deserialize)]
pub struct RlmConfig {
    /// Master switch for enqueueing summary work after indexing. `false`
    /// disables the whole RLM pipeline (nodes already stored stay readable).
    #[serde(default = "default_rlm_enabled")]
    pub enabled: bool,

    /// L2 tolerance: fraction of a theme's file hashes allowed to change
    /// before the theme summary is re-enqueued. Lower = stricter.
    #[serde(default = "default_l2_tolerance")]
    pub l2_tolerance: f64,

    /// L3 tolerance: fraction of module summaries allowed to change before
    /// the project overview is re-enqueued. Progressively higher than L2 so
    /// trivial edits never rebuild the global summary.
    #[serde(default = "default_l3_tolerance")]
    pub l3_tolerance: f64,
}

fn default_rlm_enabled() -> bool {
    true
}

fn default_l2_tolerance() -> f64 {
    0.3
}

fn default_l3_tolerance() -> f64 {
    0.5
}

impl Default for RlmConfig {
    fn default() -> Self {
        Self {
            enabled: default_rlm_enabled(),
            l2_tolerance: default_l2_tolerance(),
            l3_tolerance: default_l3_tolerance(),
        }
    }
}

fn default_listen_addr() -> String {
    "127.0.0.1:50051".to_string()
}

fn default_pool_size() -> u32 {
    4
}

fn default_flush_interval_ms() -> u64 {
    100
}

fn default_max_batch_size() -> usize {
    50
}

fn default_data_dir() -> PathBuf {
    dirs().unwrap_or_else(|| PathBuf::from(".")).join(".arags")
}

fn dirs() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

/// Server-side chunking + embedding parameters.
///
/// The embedding model is **fixed**: native all-`MiniLM`-L6-v2 via candle,
/// in-process. The server chunks raw file content it receives over gRPC using
/// `max_tokens`/`overlap_tokens`, then embeds and stores vectors (384 dims).
/// All of this is configured exclusively here — the client has no data config.
#[derive(Debug, Clone, Deserialize)]
pub struct EmbedderConfig {
    /// Checkpoint directory (`model.safetensors` + `tokenizer.json`, as
    /// shipped by `sentence-transformers/all-MiniLM-L6-v2`). Without weights
    /// the server degrades to a hash embedder (no semantic search).
    #[serde(default)]
    pub model_dir: Option<PathBuf>,

    /// Weight quantization: `int8` (default, best speed/memory/quality
    /// balance) or `none` (f32).
    #[serde(default)]
    pub quantization: Option<String>,

    /// Chunks per embedding request.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Target chunk size in tokens (server chunks raw file content it
    /// receives over gRPC).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Overlap between adjacent chunks in tokens.
    #[serde(default = "default_overlap_tokens")]
    pub overlap_tokens: usize,
    /// Whether to keep the embedder's in-memory vector cache warm.
    #[serde(default = "default_cache_enabled")]
    pub cache: bool,
}

fn default_batch_size() -> usize {
    32
}

fn default_max_tokens() -> usize {
    512
}

fn default_overlap_tokens() -> usize {
    64
}

fn default_cache_enabled() -> bool {
    true
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            model_dir: None,
            batch_size: default_batch_size(),
            quantization: None,
            max_tokens: default_max_tokens(),
            overlap_tokens: default_overlap_tokens(),
            cache: default_cache_enabled(),
        }
    }
}

impl EmbedderConfig {
    /// The configured weight quantization (INT8 by default).
    #[must_use]
    pub fn resolved_quantization(&self) -> arags_embedding::embedder::config::Quantization {
        self.quantization.as_deref().map_or(
            arags_embedding::embedder::config::Quantization::Int8,
            arags_embedding::embedder::config::Quantization::parse,
        )
    }
}

/// Search serving defaults (plan 020). Applied by the handlers when a request
/// omits the corresponding field.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    /// Default tier when a request does not specify one: `hybrid` (default),
    /// `fts`, `entity` or `vector`.
    #[serde(default = "default_search_tier")]
    pub tier: String,
    /// Default `top_k` for requests without an explicit limit.
    #[serde(default = "default_search_top_k")]
    pub top_k: usize,
    /// Default token budget for rendered context.
    #[serde(default = "default_search_max_tokens")]
    pub max_tokens: u32,
    /// Serving-path salience decay rate (`score * e^(-lambda * age_hours)`).
    /// Applied after RRF fusion when > 0; `0` disables decay at query time.
    #[serde(default)]
    pub decay_lambda: f64,
    /// Unified query (plan 023): share of the result budget given to approved
    /// RLM summaries when they are available (`0.0..=1.0`, default 0.6).
    /// `0` disables summary fusion entirely.
    #[serde(default = "default_summary_ratio")]
    pub summary_ratio: f64,
    /// Unified query: minimum normalized score for an RLM summary to be
    /// considered for fusion.
    #[serde(default = "default_summary_min_score")]
    pub summary_min_score: f64,
    /// Unified query: surface relevant exploration maps in search responses.
    #[serde(default = "default_exploration_in_query")]
    pub exploration_enabled: bool,
    /// Unified query: maximum number of exploration hits attached per query.
    #[serde(default = "default_exploration_limit")]
    pub exploration_limit: usize,
}

fn default_search_tier() -> String {
    "hybrid".to_string()
}

fn default_search_top_k() -> usize {
    10
}

fn default_search_max_tokens() -> u32 {
    8000
}

fn default_summary_ratio() -> f64 {
    0.6
}

fn default_summary_min_score() -> f64 {
    0.35
}

fn default_exploration_in_query() -> bool {
    true
}

fn default_exploration_limit() -> usize {
    2
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            tier: default_search_tier(),
            top_k: default_search_top_k(),
            max_tokens: default_search_max_tokens(),
            decay_lambda: 0.0,
            summary_ratio: default_summary_ratio(),
            summary_min_score: default_summary_min_score(),
            exploration_enabled: default_exploration_in_query(),
            exploration_limit: default_exploration_limit(),
        }
    }
}

/// Background maintenance configuration (plan 019, C.1).
#[derive(Debug, Clone, Deserialize)]
pub struct MaintenanceConfig {
    /// Cron interval in seconds. `0` disables the periodic ticker.
    #[serde(default = "default_maintenance_interval")]
    pub interval_secs: u64,
    /// Salience floor below which decayed chunks are removed.
    #[serde(default = "default_decay_score_floor")]
    pub decay_score_floor: f32,
}

fn default_maintenance_interval() -> u64 {
    3600
}

fn default_decay_score_floor() -> f32 {
    0.1
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_maintenance_interval(),
            decay_score_floor: default_decay_score_floor(),
        }
    }
}

/// Query-history retention (plan 020).
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryConfig {
    /// Purge history rows older than this many days via the maintenance
    /// ticker (`0` = keep forever).
    #[serde(default = "default_history_retention_days")]
    pub retention_days: u32,
}

fn default_history_retention_days() -> u32 {
    90
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            retention_days: default_history_retention_days(),
        }
    }
}

impl ServerConfig {
    /// Load configuration from the server config file.
    ///
    /// Order: `ARAGS_SERVER_CONFIG` env var → `/etc/arags/server.toml` → env
    /// overrides → built-in defaults.
    ///
    /// The server no longer reads the client's `.arags/config.toml` /
    /// `~/.arags/config.toml` (plan 020): `server.toml` is a disjoint host
    /// file mounted into the container.
    ///
    /// # Errors
    ///
    /// Returns an error if a config file exists but cannot be read or parsed.
    pub fn load() -> Result<Self> {
        let path = std::env::var("ARAGS_SERVER_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/etc/arags/server.toml"));
        Ok(Self::load_from_path(&path)?.with_env_overrides())
    }

    /// Load from an explicit path (missing file → defaults). Env overrides
    /// are **not** applied here; call [`Self::with_env_overrides`] after.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load_from_path(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Apply the `ARAGS_SERVER_ADDR` / `ARAGS_DATA_DIR` /
    /// `ARAGS_EMBEDDER_MODEL_DIR` environment overrides (plan 020 keeps them
    /// as ops escape hatches over the file; the model dir one lets container
    /// images bake or mount checkpoints without a config file).
    #[must_use]
    pub fn with_env_overrides(self) -> Self {
        let addr = std::env::var("ARAGS_SERVER_ADDR").ok();
        let data_dir = std::env::var("ARAGS_DATA_DIR").ok();
        let model_dir = std::env::var("ARAGS_EMBEDDER_MODEL_DIR").ok();
        self.with_overrides(addr, data_dir, model_dir)
    }

    /// Pure core of [`Self::with_env_overrides`] (testable without touching
    /// process state; Rust 2024 makes env mutation unsafe).
    #[must_use]
    pub fn with_overrides(
        mut self,
        addr: Option<String>,
        data_dir: Option<String>,
        model_dir: Option<String>,
    ) -> Self {
        if let Some(addr) = addr {
            self.listen_addr = addr;
        }
        if let Some(dir) = data_dir {
            self.data_dir = PathBuf::from(dir);
        }
        if let Some(dir) = model_dir {
            self.embedder.model_dir = Some(PathBuf::from(dir));
        }
        self
    }

    /// Path to the TLS certificate, if configured.
    #[must_use]
    pub fn tls_cert(&self) -> Option<&PathBuf> {
        self.tls_cert.as_ref()
    }

    /// Path to the TLS private key, if configured.
    #[must_use]
    pub fn tls_key(&self) -> Option<&PathBuf> {
        self.tls_key.as_ref()
    }

    /// Path to the mTLS client CA bundle, if configured.
    #[must_use]
    pub fn mtls_ca(&self) -> Option<&PathBuf> {
        self.mtls_ca.as_ref()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            data_dir: default_data_dir(),
            tls_cert: None,
            tls_key: None,
            mtls_ca: None,
            pool_size: default_pool_size(),
            flush_interval_ms: default_flush_interval_ms(),
            max_batch_size: default_max_batch_size(),
            embedder: EmbedderConfig::default(),
            search: SearchConfig::default(),
            qa_cache: QaCacheConfig::default(),
            maintenance: MaintenanceConfig::default(),
            history: HistoryConfig::default(),
            exploration: ExplorationConfig::default(),
            rlm: RlmConfig::default(),
        }
    }
}

/// Semantic query-answer cache (plan 017) tunables. All thresholds and
/// dimensionalities are configurable without a rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaCacheConfig {
    /// Chunks digested on a brand-new question (client). Upper bound.
    #[serde(default = "default_novel_k")]
    pub novel_k: usize,

    /// Provenance chunks returned alongside a cached answer.
    #[serde(default = "default_provenance_k")]
    pub provenance_k: usize,

    /// At/above this similarity a hit is a high-confidence near-exact match.
    #[serde(default = "default_sim_high")]
    pub sim_high: f32,

    /// Below this similarity the query is treated as brand new (full digest).
    #[serde(default = "default_sim_floor")]
    pub sim_floor: f32,

    /// Descending similarity boundaries for widening tiers.
    #[serde(default = "default_tier_steps")]
    pub tier_steps: Vec<f32>,

    /// Minimum provenance Jaccard for a hit to pass the secondary check.
    #[serde(default = "default_jaccard_min")]
    pub jaccard_min: f32,

    /// Dimensionality of the question embedding space.
    #[serde(default = "default_question_dims")]
    pub question_vector_dims: usize,

    /// Max cached entries kept per project before weighted-LRU eviction.
    #[serde(default = "default_max_entries")]
    pub max_entries_per_project: usize,

    /// Age half-life (ms) for weighted-LRU eviction scoring.
    #[serde(default = "default_eviction_lambda_ms")]
    pub eviction_lambda_ms: i64,

    /// Background eviction interval (ms). 0 disables the worker.
    #[serde(default = "default_eviction_interval_ms")]
    pub eviction_interval_ms: u64,
}

impl Default for QaCacheConfig {
    fn default() -> Self {
        Self {
            novel_k: default_novel_k(),
            provenance_k: default_provenance_k(),
            sim_high: default_sim_high(),
            sim_floor: default_sim_floor(),
            tier_steps: default_tier_steps(),
            jaccard_min: default_jaccard_min(),
            question_vector_dims: default_question_dims(),
            max_entries_per_project: default_max_entries(),
            eviction_lambda_ms: default_eviction_lambda_ms(),
            eviction_interval_ms: default_eviction_interval_ms(),
        }
    }
}

fn default_novel_k() -> usize {
    20
}
fn default_provenance_k() -> usize {
    5
}
fn default_sim_high() -> f32 {
    0.90
}
fn default_sim_floor() -> f32 {
    0.40
}
fn default_tier_steps() -> Vec<f32> {
    vec![0.90, 0.80, 0.70, 0.60, 0.50]
}
fn default_jaccard_min() -> f32 {
    0.5
}
fn default_question_dims() -> usize {
    arags_embedding::embedder::minilm::HIDDEN_SIZE
}
fn default_max_entries() -> usize {
    1_000
}
fn default_eviction_lambda_ms() -> i64 {
    7 * 24 * 60 * 60 * 1_000
}
fn default_eviction_interval_ms() -> u64 {
    60_000
}

#[cfg(test)]
mod testing;

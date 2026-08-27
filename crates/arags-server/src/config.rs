use std::path::PathBuf;

use serde::Deserialize;

pub(crate) mod embedder;
pub(crate) mod exploration;
pub(crate) mod maintenance;
pub(crate) mod qa_cache;
pub(crate) mod quorum;
pub(crate) mod rlm;
pub(crate) mod search;
pub(crate) mod server_impl;

pub use embedder::EmbedderConfig;
pub use exploration::{ExplorationConfig, ValidationMode};
pub use maintenance::{HistoryConfig, MaintenanceConfig};
pub use qa_cache::QaCacheConfig;
pub use quorum::{FusionStrategy, QuorumConfig};
pub use rlm::RlmConfig;
pub use search::SearchConfig;

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

    /// Dedicated rayon thread count for **index (Phase-2) embedding** (issue
    /// `agnostic-rlm-rs-6690`). The index embedder runs inside a *capped* rayon
    /// pool so a large `arags index` cannot saturate every core and starve a
    /// concurrent `arags search --tier auto`. Defaults to `num_cpus - 2`
    /// (minimum 1), deliberately leaving at least 2 cores (or 1 when only 1–2
    /// exist) free for query serving. Override with `ARAGS_INDEX_EMBED_THREADS`.
    #[serde(default = "default_index_embed_threads")]
    pub index_embed_threads: usize,

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

    /// Quorum / security (Cluster B keystone, issue `agnostic-rlm-rs-a5d7`):
    /// fan-out of volunteer jobs, cosine-similarity agreement threshold, the
    /// fusion strategy used to merge agreeing candidates, and the strikes
    /// budget before a volunteer is deprioritized/banned. The actual decision
    /// logic lives in later issues (`6d97`/`64af`); here only the data model,
    /// config and the candidate-submission storage API are wired.
    #[serde(default)]
    pub quorum: QuorumConfig,

    /// Explorations dataset (plan 022): confidence + feedback knobs.
    #[serde(default)]
    pub exploration: ExplorationConfig,

    /// Per-user rate limiting on mutating RPCs (issue `agnostic-rlm-rs-7222`).
    /// `enabled = false` makes every check a no-op pass. Missing section →
    /// defaults.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Retired (`is_active = 0`) chunk history retention window in days (issue
    /// `agnostic-rlm-rs-8dcc`). The maintenance ticker permanently purges
    /// superseded chunks older than this many days; `0` keeps history forever.
    #[serde(default = "default_chunk_retention_days")]
    pub chunk_retention_days: u64,
}

fn default_chunk_retention_days() -> u64 {
    7
}

fn default_listen_addr() -> String {
    "127.0.0.1:50051".to_string()
}

fn default_pool_size() -> u32 {
    4
}

fn default_index_embed_threads() -> usize {
    num_cpus::get().saturating_sub(2).max(1)
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

/// Per-user rate-limiting configuration (issue `agnostic-rlm-rs-7222`).
///
/// A fixed-window limiter keyed by authenticated username gates every mutating
/// RPC. When `enabled` is `false` the limiter is a no-op pass (the default
/// config still constructs, but all checks return `true`).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct RateLimitConfig {
    /// Whether rate-limiting is enforced. `false` → always allow.
    #[serde(default = "default_rl_enabled")]
    pub enabled: bool,

    /// Maximum number of allowed requests within a single window.
    #[serde(default = "default_rl_max")]
    pub max_requests_per_window: u32,

    /// Window length in seconds. When a call arrives after the window has
    /// elapsed, the per-user counter resets.
    #[serde(default = "default_rl_window")]
    pub window_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: default_rl_enabled(),
            max_requests_per_window: default_rl_max(),
            window_secs: default_rl_window(),
        }
    }
}

fn default_rl_enabled() -> bool {
    true
}

fn default_rl_max() -> u32 {
    60
}

fn default_rl_window() -> u64 {
    60
}

#[cfg(test)]
mod testing;

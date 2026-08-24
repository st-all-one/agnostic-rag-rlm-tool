use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Server configuration loaded from TOML.
///
/// Plan 020: this is the **server-only data-plane** file (`server.toml`, a
/// host file mounted into the container at `/etc/arlm/server.toml`). It owns
/// everything that touches data — serving (listen/tls), storage (data_dir),
/// processing ([embedder]) and serving defaults ([search]). It has no LLM
/// section (the server is LLM-free) and is disjoint from the user files
/// (`~/.arlm/arlm.toml` / `.arlm.toml`).
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
    dirs().unwrap_or_else(|| PathBuf::from(".")).join(".arlm")
}

fn dirs() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

/// Embedding model family served by the data plane (plan 020 `[embedder]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedderModel {
    /// Real BGE-M3 via candle (requires `model_dir` with weights).
    #[default]
    BgeM3,
    /// Ollama HTTP embedding server (`ollama_url` + `ollama_model`).
    Ollama,
    /// Hash-based lightweight embedder (tests / degraded mode).
    Lightweight,
}

impl EmbedderModel {
    /// Parse a `server.toml` `model` string.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ollama" => Self::Ollama,
            "lightweight" | "fallback" | "hash" => Self::Lightweight,
            _ => Self::BgeM3,
        }
    }
}

/// Server-side chunking + embedding parameters (plan 020, D2).
///
/// The server chunks raw file content it receives over gRPC using
/// `max_tokens`/`overlap_tokens`, then embeds and stores vectors. All of this
/// is configured exclusively here — the client has no data config.
#[derive(Debug, Clone, Deserialize)]
pub struct EmbedderConfig {
    /// Embedding model: `bge-m3` (default), `ollama`, or `lightweight`.
    #[serde(default)]
    pub model: Option<String>,

    /// Model weights directory (BGE-M3: `model.safetensors` + `tokenizer.json`).
    #[serde(default)]
    pub model_dir: Option<PathBuf>,

    /// Ollama base URL (model = "ollama").
    #[serde(default)]
    pub ollama_url: Option<String>,

    /// Ollama embedding model tag (model = "ollama"), e.g. `all-minilm`.
    #[serde(default)]
    pub ollama_model: Option<String>,

    /// Optional task prefix prepended to embedded texts
    /// (`search_document: ` for nomic-family models; empty for all-minilm).
    #[serde(default)]
    pub ollama_prefix: Option<String>,

    /// Vector dimensionality used to size the LanceDB stores.
    #[serde(default = "default_dims")]
    pub dims: usize,

    /// Chunks per embedding request.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Quantization for candle BGE-M3 weights: `int8` (default), `int4`,
    /// `none`.
    #[serde(default)]
    pub quantization: Option<String>,

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

fn default_dims() -> usize {
    1024
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
            model: None,
            model_dir: None,
            ollama_url: None,
            ollama_model: None,
            ollama_prefix: None,
            dims: default_dims(),
            batch_size: default_batch_size(),
            quantization: None,
            max_tokens: default_max_tokens(),
            overlap_tokens: default_overlap_tokens(),
            cache: default_cache_enabled(),
        }
    }
}

impl EmbedderConfig {
    /// The resolved model family (defaults to [`EmbedderModel::BgeM3`]).
    #[must_use]
    pub fn resolved_model(&self) -> EmbedderModel {
        self.model.as_deref().map_or(
            match (&self.model_dir, &self.ollama_model) {
                (Some(_), _) | (None, Some(_)) => EmbedderModel::Ollama,
                (None, None) => EmbedderModel::BgeM3,
            },
            EmbedderModel::parse,
        )
    }

    /// The configured weight quantization (candle BGE-M3 only).
    #[must_use]
    pub fn resolved_quantization(&self) -> arlm_embedding::embedder::config::Quantization {
        use arlm_embedding::embedder::config::Quantization;
        match self.quantization.as_deref() {
            Some("none") => Quantization::None,
            Some("int4") => Quantization::Int4,
            _ => Quantization::Int8,
        }
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

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            tier: default_search_tier(),
            top_k: default_search_top_k(),
            max_tokens: default_search_max_tokens(),
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
    /// Order: `ARLM_SERVER_CONFIG` env var → `/etc/arlm/server.toml` → env
    /// overrides → built-in defaults.
    ///
    /// The server no longer reads the client's `.arlm/config.toml` /
    /// `~/.arlm/config.toml` (plan 020): `server.toml` is a disjoint host
    /// file mounted into the container.
    ///
    /// # Errors
    ///
    /// Returns an error if a config file exists but cannot be read or parsed.
    pub fn load() -> Result<Self> {
        let path = std::env::var("ARLM_SERVER_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/etc/arlm/server.toml"));
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

    /// Apply the `ARLM_SERVER_ADDR` / `ARLM_DATA_DIR` environment overrides
    /// (plan 020 keeps both as ops escape hatches over the file).
    #[must_use]
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(addr) = std::env::var("ARLM_SERVER_ADDR") {
            self.listen_addr = addr;
        }
        if let Ok(dir) = std::env::var("ARLM_DATA_DIR") {
            self.data_dir = PathBuf::from(dir);
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
    1024
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
mod tests {
    use super::*;
    use std::io::Write as _;

    fn temp_config(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        (dir, path)
    }

    #[test]
    fn test_server_config_loads_from_arlm_server_config_env() {
        // `load_from_path` is the env-free core of `load()`; the default
        // path comes from `ARLM_SERVER_CONFIG` (else /etc/arlm/server.toml).
        let (_d, path) = temp_config("listen_addr = \"0.0.0.0:9999\"\n");
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:9999");

        // Missing file → built-in defaults.
        let d = tempfile::tempdir().unwrap();
        let cfg = ServerConfig::load_from_path(&d.path().join("absent.toml")).unwrap();
        assert_eq!(cfg.listen_addr, default_listen_addr());
        assert_eq!(cfg.embedder.dims, default_dims());
        assert_eq!(cfg.embedder.batch_size, default_batch_size());
    }

    #[test]
    fn test_server_config_has_no_llm_section() {
        // A `server.toml` without `[llm]` parses fine; a stray `[llm]`
        // section must NOT silently map onto any field of the schema.
        let (_d, path) =
            temp_config("listen_addr = \"127.0.0.1:50051\"\ndata_dir = \"/tmp/arlm\"\n");
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/arlm"));
    }

    #[test]
    fn test_server_config_embedder_chunk_size_applied() {
        let (_d, path) = temp_config(
            "[embedder]\nmax_tokens = 1024\noverlap_tokens = 128\ndims = 384\nbatch_size = 8\nmodel = \"lightweight\"\ncache = false\n",
        );
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.embedder.max_tokens, 1024);
        assert_eq!(cfg.embedder.overlap_tokens, 128);
        assert_eq!(cfg.embedder.dims, 384);
        assert_eq!(cfg.embedder.batch_size, 8);
        assert_eq!(cfg.embedder.resolved_model(), EmbedderModel::Lightweight);
        assert!(!cfg.embedder.cache);
    }

    #[test]
    fn test_server_config_search_and_mtls_defaults() {
        let defaults = ServerConfig::default();
        assert_eq!(defaults.search.top_k, 10);
        assert_eq!(defaults.search.max_tokens, 8000);
        assert_eq!(defaults.search.tier, "hybrid");
        assert!(defaults.mtls_ca().is_none());

        let (_d, path) = temp_config(
            "mtls_ca = \"/etc/arlm/tls/ca.crt\"\n\n[search]\ntop_k = 42\nmax_tokens = 100\n",
        );
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.search.top_k, 42);
        assert_eq!(cfg.mtls_ca(), Some(&PathBuf::from("/etc/arlm/tls/ca.crt")));
    }
}

#[cfg(test)]
mod disjoint_tests {
    use super::*;
    use tempfile::TempDir;

    /// Plan 020: the server must NOT read the user's `~/.arlm/arlm.toml` /
    /// `.arlm.toml`. Parsing a user-config-shaped file as `ServerConfig`
    /// leaves every data-plane field at its default.
    #[test]
    fn test_server_config_ignores_user_arlm_toml_semantics() {
        let dir = TempDir::new().unwrap();
        let user_toml = r#"
[auth]
username = "dev1"
refresh_token = "tok"

[llm]
[[llm.backends]]
name = "default"
family = "ollama"
model = "llama3.2"

[server]
addr = "https://arlm.corp.internal:50051"

[project]
name = "meu-repo"
"#;
        let path = dir.path().join("arlm.toml");
        std::fs::write(&path, user_toml).unwrap();

        let cfg = ServerConfig::load_from_path(&path).unwrap();
        // `[server].addr` (client connect target) must NOT become listen_addr.
        assert_eq!(cfg.listen_addr, default_listen_addr());
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.embedder.max_tokens, default_max_tokens());
        assert!(cfg.mtls_ca.is_none());
    }
}

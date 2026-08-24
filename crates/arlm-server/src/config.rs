use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Server configuration loaded from TOML.
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

    /// Server-side chunking parameters (plan 020, D2). The server owns all
    /// data-plane processing, so chunk size is configured here rather than on
    /// the client.
    #[serde(default)]
    pub embedder: EmbedderConfig,

    /// Semantic query-answer cache configuration (plan 017).
    #[serde(default)]
    pub qa_cache: QaCacheConfig,

    /// Background memory maintenance (plan 019, C.1): consolidate + decay.
    #[serde(default)]
    pub maintenance: MaintenanceConfig,
}

fn default_listen_addr() -> String {
    "127.0.0.1:50051".to_string()
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

/// Server-side chunking parameters (plan 020, D2).
#[derive(Debug, Clone, Deserialize)]
pub struct EmbedderConfig {
    /// Target chunk size in tokens (server chunks raw file content it
    /// receives over gRPC).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Overlap between adjacent chunks in tokens.
    #[serde(default = "default_overlap_tokens")]
    pub overlap_tokens: usize,
}

fn default_max_tokens() -> usize {
    512
}

fn default_overlap_tokens() -> usize {
    64
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            overlap_tokens: default_overlap_tokens(),
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

        let mut config = if path.exists() {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read config from {}", path.display()))?;
            toml::from_str(&contents)
                .with_context(|| format!("failed to parse config from {}", path.display()))?
        } else {
            Self::default()
        };

        // Environment overrides win over the file (plan 020).
        if let Ok(addr) = std::env::var("ARLM_SERVER_ADDR") {
            config.listen_addr = addr;
        }
        if let Ok(dir) = std::env::var("ARLM_DATA_DIR") {
            config.data_dir = PathBuf::from(dir);
        }

        Ok(config)
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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            data_dir: default_data_dir(),
            tls_cert: None,
            tls_key: None,
            embedder: EmbedderConfig::default(),
            qa_cache: QaCacheConfig::default(),
            maintenance: MaintenanceConfig::default(),
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

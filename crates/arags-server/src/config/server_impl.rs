//! `ServerConfig` loading, env-overrides, and accessor methods.

use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::ServerConfig;
use super::default_chunk_retention_days;
use super::default_data_dir;
use super::default_flush_interval_ms;
use super::default_index_embed_threads;
use super::default_listen_addr;
use super::default_max_batch_size;
use super::default_pool_size;

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
    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Apply the `ARAGS_SERVER_ADDR` / `ARAGS_DATA_DIR` /
    /// `ARAGS_EMBEDDER_MODEL_DIR` / `ARAGS_EMBEDDER_KIND` /
    /// `ARAGS_EMBEDDER_OLLAMA_URL` / `ARAGS_EMBEDDER_OLLAMA_MODEL` /
    /// `ARAGS_EMBEDDER_LLAMACPP_MODEL` / `ARAGS_EMBEDDER_LLAMACPP_GPU_LAYERS`
    /// environment overrides (plan 020 keeps them as ops escape hatches over
    /// the file; the model dir one lets container images bake or mount
    /// checkpoints without a config file). `ARAGS_INDEX_EMBED_THREADS` caps the
    /// index-embed rayon pool (issue `agnostic-rlm-rs-6690`).
    #[must_use]
    pub fn with_env_overrides(self) -> Self {
        let addr = std::env::var("ARAGS_SERVER_ADDR").ok();
        let data_dir = std::env::var("ARAGS_DATA_DIR").ok();
        let model_dir = std::env::var("ARAGS_EMBEDDER_MODEL_DIR").ok();
        let kind = std::env::var("ARAGS_EMBEDDER_KIND").ok();
        let ollama_url = std::env::var("ARAGS_EMBEDDER_OLLAMA_URL").ok();
        let ollama_model = std::env::var("ARAGS_EMBEDDER_OLLAMA_MODEL").ok();
        let index_embed_threads = std::env::var("ARAGS_INDEX_EMBED_THREADS").ok();
        let llama_cpp_model = std::env::var("ARAGS_EMBEDDER_LLAMACPP_MODEL").ok();
        let llama_cpp_gpu_layers = std::env::var("ARAGS_EMBEDDER_LLAMACPP_GPU_LAYERS").ok();
        let mut s = self.with_overrides(
            addr,
            data_dir,
            model_dir,
            kind,
            ollama_url,
            ollama_model,
            index_embed_threads,
        );
        if let Some(m) = llama_cpp_model {
            s.embedder.llama_cpp_model = Some(PathBuf::from(m));
        }
        if let Some(g) = llama_cpp_gpu_layers {
            if let Ok(n) = g.parse::<u32>() {
                s.embedder.llama_cpp_gpu_layers = Some(n);
            }
        }
        s
    }

    /// Pure core of [`Self::with_env_overrides`] (testable without touching
    /// process state; Rust 2024 makes env mutation unsafe).
    #[must_use]
    pub fn with_overrides(
        mut self,
        addr: Option<String>,
        data_dir: Option<String>,
        model_dir: Option<String>,
        kind: Option<String>,
        ollama_url: Option<String>,
        ollama_model: Option<String>,
        index_embed_threads: Option<String>,
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
        if let Some(kind) = kind {
            self.embedder.kind = Some(kind);
        }
        if let Some(url) = ollama_url {
            self.embedder.ollama_url = Some(url);
        }
        if let Some(model) = ollama_model {
            self.embedder.ollama_model = Some(model);
        }
        if let Some(threads) = index_embed_threads {
            if let Ok(n) = threads.parse::<usize>() {
                if n >= 1 {
                    self.index_embed_threads = n;
                }
            }
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
            index_embed_threads: default_index_embed_threads(),
            flush_interval_ms: default_flush_interval_ms(),
            max_batch_size: default_max_batch_size(),
            embedder: super::EmbedderConfig::default(),
            search: super::SearchConfig::default(),
            qa_cache: super::QaCacheConfig::default(),
            maintenance: super::MaintenanceConfig::default(),
            history: super::HistoryConfig::default(),
            exploration: super::ExplorationConfig::default(),
            rate_limit: super::RateLimitConfig::default(),
            rlm: super::RlmConfig::default(),
            quorum: super::QuorumConfig::default(),
            chunk_retention_days: default_chunk_retention_days(),
        }
    }
}

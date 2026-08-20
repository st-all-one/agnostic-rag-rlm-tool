use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use arlm_llm::{BackendKind, LlmBackend, get_backend};
use serde::Deserialize;
use std::sync::Arc;

/// Server configuration loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Address to listen on (e.g., "127.0.0.1:50051").
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// Data directory for SQLite and LanceDB.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Maximum number of connections in the SQLite pool.
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Flush interval for the write queue in milliseconds.
    #[serde(default = "default_flush_interval_ms")]
    pub flush_interval_ms: u64,

    /// Maximum batch size for the write queue.
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,

    /// LLM backend configuration (used by the run engine and summarization).
    #[serde(default)]
    pub llm: LlmConfig,

    /// Optional PEM certificate path. Enables TLS when set together with
    /// `tls_key`.
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,

    /// Optional PEM private key path. Enables TLS when set together with
    /// `tls_cert`.
    #[serde(default)]
    pub tls_key: Option<PathBuf>,
}

/// LLM backend configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    /// Backend name: openai | anthropic | ollama | gemini | deepseek | mimo.
    #[serde(default = "default_llm_backend")]
    pub backend: String,

    /// Model identifier (e.g., "gpt-4o-mini", "claude-3-5-sonnet").
    #[serde(default = "default_llm_model")]
    pub model: String,

    /// API key. Falls back to the backend-specific env var when absent.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Optional base URL override (e.g., for proxies / local gateways).
    #[serde(default)]
    pub base_url: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            backend: default_llm_backend(),
            model: default_llm_model(),
            api_key: None,
            base_url: None,
        }
    }
}

fn default_llm_backend() -> String {
    "ollama".to_string()
}

fn default_llm_model() -> String {
    "qwen2.5-coder:7b".to_string()
}

fn default_listen_addr() -> String {
    "127.0.0.1:50051".to_string()
}

fn default_data_dir() -> PathBuf {
    dirs().unwrap_or_else(|| PathBuf::from(".")).join(".arlm")
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

fn dirs() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

impl ServerConfig {
    /// Load configuration from default locations.
    ///
    /// Order: local `.arlm/config.toml` → global `~/.arlm/config.toml` →
    /// env vars → defaults.
    ///
    /// # Errors
    ///
    /// Returns an error if a config file exists but cannot be read or parsed.
    pub fn load() -> Result<Self> {
        // Try local config first
        let local_config = std::env::current_dir()
            .ok()
            .map(|p| p.join(".arlm/config.toml"))
            .filter(|p| p.exists());

        // Then global config
        let global_config = dirs()
            .map(|d| d.join(".arlm/config.toml"))
            .filter(|p| p.exists());

        if let Some(path) = local_config {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read config from {}", path.display()))?;
            let config: ServerConfig = toml::from_str(&contents)
                .with_context(|| format!("failed to parse config from {}", path.display()))?;
            return Ok(config);
        }

        if let Some(path) = global_config {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read config from {}", path.display()))?;
            let config: ServerConfig = toml::from_str(&contents)
                .with_context(|| format!("failed to parse config from {}", path.display()))?;
            return Ok(config);
        }

        // Check env var
        if let Ok(addr) = std::env::var("ARLM_SERVER_ADDR") {
            return Ok(Self {
                listen_addr: addr,
                ..Self::default()
            });
        }

        Ok(Self::default())
    }

    /// Build the configured LLM backend.
    ///
    /// Falls back to the environment variable for the chosen backend
    /// (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`,
    /// `DEEPSEEK_API_KEY`, `MIMO_API_KEY`) when no key is set in config.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend kind is unknown or the backend
    /// requires an API key that is not available.
    pub fn build_llm(&self) -> Result<Arc<dyn LlmBackend + Send + Sync>> {
        let kind = BackendKind::from_str(&self.llm.backend)
            .with_context(|| format!("unknown llm backend '{}'", self.llm.backend))?;

        let api_key = self.llm.api_key.clone().or_else(|| {
            env_key_for(&kind).and_then(|key| std::env::var(key).ok())
        });

        Ok(get_backend(&kind, api_key, self.llm.base_url.clone())
            .context("failed to build LLM backend")?)
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

fn env_key_for(kind: &BackendKind) -> Option<&'static str> {
    match kind {
        BackendKind::OpenAI => Some("OPENAI_API_KEY"),
        BackendKind::Anthropic => Some("ANTHROPIC_API_KEY"),
        BackendKind::Gemini => Some("GEMINI_API_KEY"),
        BackendKind::DeepSeek => Some("DEEPSEEK_API_KEY"),
        BackendKind::MiMo => Some("MIMO_API_KEY"),
        BackendKind::Ollama => None,
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            data_dir: default_data_dir(),
            pool_size: default_pool_size(),
            flush_interval_ms: default_flush_interval_ms(),
            max_batch_size: default_max_batch_size(),
            llm: LlmConfig::default(),
            tls_cert: None,
            tls_key: None,
        }
    }
}
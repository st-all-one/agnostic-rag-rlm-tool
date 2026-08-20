use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

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
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed.
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
                ..Default::default()
            });
        }

        Ok(Self::default())
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
        }
    }
}

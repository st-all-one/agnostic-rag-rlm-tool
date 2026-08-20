use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::util::data_dir;

/// Configuration file structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Default LLM backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,

    /// Default model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Default project path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,

    /// Default output format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Embedding configuration.
    #[serde(default)]
    pub embedding: EmbeddingConfig,

    /// Search configuration.
    #[serde(default)]
    pub search: SearchConfig,

    /// Agent configuration.
    #[serde(default)]
    pub agent: AgentConfig,

    /// Server configuration.
    #[serde(default)]
    pub server: ServerSection,
}

/// Server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerSection {
    /// Default gRPC server address (e.g. `127.0.0.1:50051` or `https://host:443`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
}

/// Embedding configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingConfig {
    /// Batch size for embedding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<usize>,

    /// Maximum tokens per chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,

    /// Overlap tokens between chunks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlap_tokens: Option<usize>,

    /// Enable embedding cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
}

/// Search configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchConfig {
    /// Default search tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,

    /// Default top K results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    /// Maximum tokens for context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// Agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    /// Default agent name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Maximum depth for RLM runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,

    /// Maximum nodes for RLM runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u32>,
}

impl Config {
    /// Load configuration from default path.
    pub fn load() -> Result<Self> {
        let path = config_path();
        Self::load_from(&path)
    }

    /// Load configuration from a specific path.
    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read config at {}", path.display()))?;
            let config: Config =
                toml::from_str(&content).with_context(|| "failed to parse config")?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Save configuration to file.
    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir at {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self).with_context(|| "failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write config at {}", path.display()))?;
        Ok(())
    }
}

/// Get the config file path.
#[must_use]
pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

//! Top-level LLM configuration file loading ([`LlmConfig`]).

use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::BackendConfig;
use crate::types::LlmError;

/// Top-level LLM configuration file.
///
/// Holds an ordered list of [`BackendConfig`] entries. This is the structure
/// that `config.toml` (typically at `~/.arags/config.toml`) deserializes into.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub backends: Vec<BackendConfig>,
}

impl FromStr for LlmConfig {
    type Err = LlmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        toml::from_str(s).map_err(|e| LlmError::Serialization(e.to_string()))
    }
}

impl LlmConfig {
    /// Load configuration from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::Backend`] if the file cannot be read, or
    /// [`LlmError::Serialization`] if its contents are invalid.
    pub fn from_file(path: &Path) -> Result<Self, LlmError> {
        let content = fs::read_to_string(path).map_err(|e| {
            LlmError::Backend(format!("cannot read config {}: {e}", path.display()))
        })?;
        content.parse()
    }

    /// The configured backends, in file order.
    #[must_use]
    pub fn backends(&self) -> &[BackendConfig] {
        &self.backends
    }
}

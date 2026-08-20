use std::path::PathBuf;

use anyhow::{Context, Result};
use arlm_proto::proto::arlm_service_client::ArlmServiceClient;
use tonic::transport::Channel;

/// Client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Server address (e.g., "127.0.0.1:50051").
    pub addr: String,
}

impl ClientConfig {
    /// Load client configuration from default locations.
    ///
    /// Resolution order:
    /// 1. .arlm/config.toml (local)
    /// 2. ~/.arlm/config.toml (global)
    /// 3. ARLM_SERVER_ADDR env var
    /// 4. Fallback: 127.0.0.1:50051
    pub fn load() -> Self {
        // Try local config
        if let Ok(cwd) = std::env::current_dir() {
            let local_config = cwd.join(".arlm/config.toml");
            if local_config.exists() {
                if let Ok(contents) = std::fs::read_to_string(&local_config) {
                    if let Ok(config) = toml::from_str::<ServerConfig>(&contents) {
                        return Self { addr: config.server.addr };
                    }
                }
            }
        }

        // Try global config
        if let Some(home) = dirs() {
            let global_config = home.join(".arlm/config.toml");
            if global_config.exists() {
                if let Ok(contents) = std::fs::read_to_string(&global_config) {
                    if let Ok(config) = toml::from_str::<ServerConfig>(&contents) {
                        return Self { addr: config.server.addr };
                    }
                }
            }
        }

        // Try env var
        if let Ok(addr) = std::env::var("ARLM_SERVER_ADDR") {
            return Self { addr };
        }

        // Fallback
        Self {
            addr: "127.0.0.1:50051".to_string(),
        }
    }
}

#[derive(serde::Deserialize)]
struct ServerConfig {
    server: ServerSection,
}

#[derive(serde::Deserialize)]
struct ServerSection {
    #[serde(default = "default_addr")]
    addr: String,
}

fn default_addr() -> String {
    "127.0.0.1:50051".to_string()
}

fn dirs() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

/// Create a gRPC client connected to the server.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
pub async fn create_client(config: &ClientConfig) -> Result<ArlmServiceClient<Channel>> {
    let addr = if config.addr.starts_with("http") {
        config.addr.clone()
    } else {
        format!("http://{}", config.addr)
    };

    let channel = Channel::from_shared(addr.clone())
        .context("invalid server address")?
        .connect()
        .await
        .context("failed to connect to server")?;

    Ok(ArlmServiceClient::new(channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_config_load() {
        let config = ClientConfig::load();
        assert!(!config.addr.is_empty());
    }
}

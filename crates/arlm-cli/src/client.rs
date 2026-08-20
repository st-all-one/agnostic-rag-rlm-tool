use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use arlm_proto::proto::arlm_service_client::ArlmServiceClient;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tracing::{info, warn};

/// Client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Server address (e.g., "127.0.0.1:50051" or "https://host:443").
    pub addr: String,
}

impl ClientConfig {
    /// Load client configuration from default locations.
    ///
    /// Resolution order:
    /// 1. .arlm/config.toml (local) — `server.addr`
    /// 2. ~/.arlm/config.toml (global) — `server.addr`
    /// 3. `ARLM_SERVER_ADDR` env var
    /// 4. Fallback: 127.0.0.1:50051
    #[must_use]
    pub fn load() -> Self {
        if let Some(addr) = read_server_addr_from_config() {
            return Self { addr };
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

/// Read the `[server] addr` value from the local/global config files.
#[must_use]
fn read_server_addr_from_config() -> Option<String> {
    // Try local config
    if let Ok(cwd) = std::env::current_dir() {
        let local_config = cwd.join(".arlm/config.toml");
        if let Some(addr) = read_addr_from_file(&local_config) {
            return Some(addr);
        }
    }

    // Try global config
    if let Some(home) = dirs() {
        let global_config = home.join(".arlm/config.toml");
        if let Some(addr) = read_addr_from_file(&global_config) {
            return Some(addr);
        }
    }

    None
}

/// Parse only the `server.addr` field out of a config TOML file.
#[must_use]
fn read_addr_from_file(path: &std::path::Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let contents = std::fs::read_to_string(path).ok()?;
    let config: crate::config::Config = toml::from_str(&contents).ok()?;
    config.server.addr
}

fn dirs() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

/// Validate that `addr` is a `host:port` pair.
fn validate_addr(addr: &str) -> Result<()> {
    let (host, port) = addr
        .rsplit_once(':')
        .with_context(|| format!("server address must be host:port, got: {addr}"))?;
    if host.is_empty() {
        anyhow::bail!("server address has an empty host: {addr}");
    }
    if port.is_empty() {
        anyhow::bail!("server address has an empty port: {addr}");
    }
    port.parse::<u16>()
        .with_context(|| format!("server port must be 0-65535, got: {port}"))?;
    Ok(())
}

/// Create a gRPC client connected to the server.
///
/// Supports plaintext (`http://` / host:port) and TLS (`https://` with native
/// root certificates). Connection failures are retried with exponential
/// backoff (3 attempts).
///
/// # Errors
///
/// Returns an error if the address is invalid or the connection cannot be
/// established after the retry budget is exhausted.
pub async fn create_client(config: &ClientConfig) -> Result<ArlmServiceClient<Channel>> {
    let raw = config.addr.trim();
    let (scheme, hostport) = if let Some(rest) = raw.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = raw.strip_prefix("http://") {
        ("http", rest)
    } else {
        ("http", raw)
    };

    validate_addr(hostport).with_context(|| format!("invalid server address: {raw}"))?;

    let uri = if scheme == "https" {
        raw.to_string()
    } else {
        format!("http://{hostport}")
    };

    let endpoint =
        Channel::from_shared(uri.clone()).with_context(|| format!("invalid server URI: {uri}"))?;

    let endpoint: Endpoint = if scheme == "https" {
        let tls = ClientTlsConfig::new().with_native_roots();
        endpoint.tls_config(tls)?
    } else {
        endpoint
    };

    let max_attempts: u32 = 3;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        match endpoint.connect().await {
            Ok(channel) => {
                info!(attempt, %raw, "connected to arlm-server");
                return Ok(ArlmServiceClient::new(channel));
            }
            Err(e) => {
                if attempt >= max_attempts {
                    return Err(anyhow::anyhow!(
                        "failed to connect to server at {raw} after {max_attempts} attempts: {e}"
                    ));
                }
                let backoff = Duration::from_millis(250 * 2u64.pow(attempt - 1));
                warn!(
                    attempt,
                    max_attempts,
                    error = %e,
                    backoff_ms = backoff.as_millis() as u64,
                    "server connection failed, retrying with backoff"
                );
                tokio::time::sleep(backoff).await;
            }
        }
    }
}

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
    /// Load the client configuration from the merged user config (global
    /// `~/.arlm/arlm.toml` + local `.arlm.toml`) and the `ARLM_SERVER_ADDR`
    /// env var override.
    #[must_use]
    pub fn load() -> Self {
        let addr = crate::user_config::load()
            .map_or_else(|_| "127.0.0.1:50051".to_string(), |c| c.server_addr());
        Self { addr }
    }
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
    let channel = connect_channel(config).await?;
    Ok(ArlmServiceClient::new(channel))
}

/// Establish a raw gRPC `Channel` to the server (no auth layer).
///
/// Supports plaintext (`http://` / host:port) and TLS (`https://` with native
/// root certificates). Connection failures are retried with exponential
/// backoff (3 attempts).
///
/// # Errors
///
/// Returns an error if the address is invalid or the connection cannot be
/// established after the retry budget is exhausted.
pub async fn connect_channel(config: &ClientConfig) -> Result<Channel> {
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
                return Ok(channel);
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

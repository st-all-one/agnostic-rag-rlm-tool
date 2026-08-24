use std::time::Duration;

use anyhow::{Context, Result};
use arags_proto::proto::arags_service_client::AragsServiceClient;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tracing::{info, warn};

use crate::user_config::EffectiveUserConfig;

/// Client connection configuration (plan 020).
///
/// TLS fields come from `[server]` in the merged user config: `tls_ca`
/// trusts a custom CA; `tls_cert`/`tls_key` present a client certificate
/// for mTLS servers configured with `mtls_ca`.
#[derive(Debug, Clone, Default)]
pub struct ClientConfig {
    /// Server address (e.g., "127.0.0.1:50051" or "https://host:443").
    pub addr: String,
    /// Optional PEM CA bundle to trust.
    pub tls_ca: Option<String>,
    /// Optional PEM client certificate (requires `tls_key`).
    pub tls_cert: Option<String>,
    /// Optional PEM client private key (requires `tls_cert`).
    pub tls_key: Option<String>,
}

impl ClientConfig {
    /// Load the client configuration from the merged user config (global
    /// `~/.arags/arags.toml` + local `.arags.toml`) and the `ARAGS_SERVER_ADDR`
    /// env var override.
    #[must_use]
    pub fn load() -> Self {
        let cfg = crate::user_config::load().ok();
        let addr = cfg.as_ref().map_or_else(
            || "127.0.0.1:50051".to_string(),
            EffectiveUserConfig::server_addr,
        );
        let server = cfg.map(|c| c.server);
        Self {
            addr,
            tls_ca: server.as_ref().and_then(|s| s.tls_ca.clone()),
            tls_cert: server.as_ref().and_then(|s| s.tls_cert.clone()),
            tls_key: server.as_ref().and_then(|s| s.tls_key.clone()),
        }
    }
}

/// Whether any TLS knob is configured (forces the TLS transport even for a
/// bare `host:port` address, e.g. internal mTLS endpoints without scheme).
#[must_use]
fn has_tls_config(config: &ClientConfig) -> bool {
    config.tls_ca.is_some() || config.tls_cert.is_some() || config.tls_key.is_some()
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
pub async fn create_client(config: &ClientConfig) -> Result<AragsServiceClient<Channel>> {
    let channel = connect_channel(config).await?;
    Ok(AragsServiceClient::new(channel))
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

    let endpoint: Endpoint = if scheme == "https" || has_tls_config(config) {
        let mut tls = ClientTlsConfig::new();
        if let Some(ca) = &config.tls_ca {
            // tonic 0.13 parses lazily; a bad PEM surfaces at handshake.
            tls = tls.ca_certificate(Certificate::from_pem(ca.as_bytes()));
        } else {
            tls = tls.with_native_roots();
        }
        if let (Some(cert), Some(key)) = (&config.tls_cert, &config.tls_key) {
            let identity = Identity::from_pem(cert.as_bytes(), key.as_bytes());
            info!("mTLS enabled: presenting client certificate");
            tls = tls.identity(identity);
        } else if config.tls_cert.is_some() || config.tls_key.is_some() {
            warn!(
                "[server] mTLS requires BOTH tls_cert and tls_key; continuing without client cert"
            );
        }
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
                info!(attempt, %raw, "connected to arags-server");
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

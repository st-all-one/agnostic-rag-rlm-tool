//! Client-side auth (plan 018).
//!
//! Wraps the generated gRPC client so every call automatically carries a
//! `Bearer` session token. The session token is obtained once from
//! `AuthRefresh` and proactively renewed on a background task (the refresh
//! token lives only in `~/.arags/arags.toml`); the CLI user never manages it manually.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{Request, Status};

use arags_proto::proto::arags_service_client::AragsServiceClient;
use arags_proto::proto::{AuthRefreshRequest, AuthRefreshResponse};

use crate::client::{self, ClientConfig};

/// Authenticated gRPC client type returned by [`connect`].
pub type AragsClient = AragsServiceClient<InterceptedService<Channel, BearerInterceptor>>;

/// Interceptor that attaches the current session token as a `Bearer` header.
#[derive(Clone)]
pub struct BearerInterceptor {
    token: Arc<Mutex<String>>,
}

impl Interceptor for BearerInterceptor {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        let token = self.token.lock().clone();
        if token.is_empty() {
            return Ok(req);
        }
        let value = MetadataValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| Status::internal("invalid session token"))?;
        req.metadata_mut().insert("authorization", value);
        Ok(req)
    }
}

/// Connect to the server, performing `AuthRefresh` (if a refresh token is
/// configured) and returning a client that auto-attaches and renews the
/// session token, together with a shared handle to the **current** session token.
///
/// The token handle is needed by callers that must *attest* their requests
/// (e.g. RLM volunteer submissions sign an HMAC over the session token — see
/// `arags_core::rlm_attestation`). When no `refresh_token` is configured the
/// returned client sends no auth header (the server will reject privileged RPCs
/// with `UNAUTHENTICATED`) and the token handle holds an empty string.
///
/// # Errors
///
/// Returns an error if the channel cannot be established or the initial
/// `AuthRefresh` fails.
pub fn connect(
    rt: &Runtime,
    client_config: &ClientConfig,
    auth: &crate::user_config::AuthConfig,
) -> Result<(AragsClient, Arc<Mutex<String>>)> {
    let channel = rt
        .block_on(client::connect_channel(client_config))
        .context("failed to connect to arags-server")?;

    let token = Arc::new(Mutex::new(String::new()));

    if let Some(refresh) = &auth.refresh_token {
        let refresh = refresh.clone();

        let mut refresh_client = AragsServiceClient::new(channel.clone());
        let session: AuthRefreshResponse = rt
            .block_on(refresh_client.auth_refresh(AuthRefreshRequest {
                refresh_token: refresh.clone(),
            }))
            .context("AuthRefresh failed")?
            .into_inner();
        *token.lock() = session.session_token;

        let renewal_token = token.clone();
        let mut renewal_client = AragsServiceClient::new(channel.clone());
        let renewal_refresh = refresh.clone();
        rt.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(4 * 60));
            loop {
                ticker.tick().await;
                match renewal_client
                    .auth_refresh(AuthRefreshRequest {
                        refresh_token: renewal_refresh.clone(),
                    })
                    .await
                {
                    Ok(resp) => {
                        *renewal_token.lock() = resp.into_inner().session_token;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "auth session renewal failed; will retry");
                    }
                }
            }
        });
    }

    let interceptor = BearerInterceptor {
        token: token.clone(),
    };
    let client = AragsServiceClient::new(InterceptedService::new(channel, interceptor));
    Ok((client, token))
}

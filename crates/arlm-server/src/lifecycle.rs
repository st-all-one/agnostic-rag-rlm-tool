use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use arlm_proto::proto::arlm_service_server::ArlmServiceServer;
use arlm_storage::{Storage, VectorStore};
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tracing::info;

use crate::config::ServerConfig;
use crate::grpc::ArlmGrpcService;
use crate::state::AppState;
use crate::timing::Timer;

/// Load config, open storage, wire the service and run the gRPC server.
///
/// Blocks until a shutdown signal is received.
///
/// # Errors
///
/// Returns an error if configuration, storage, the LLM backend or the server
/// setup fails.
pub async fn run() -> Result<()> {
    let _timer = Timer::new("server_startup");

    let config = ServerConfig::load().context("failed to load server config")?;

    info!(addr = %config.listen_addr, backend = %config.llm.backend, model = %config.llm.model, "starting arlm-server");

    let storage = Storage::open_pooled(&config.data_dir, config.pool_size)
        .context("failed to open storage")?;

    let llm = AppState::build_llm(&config).context("failed to configure LLM backend")?;

    let vector_store = match VectorStore::open(&config.data_dir).await {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "vector store unavailable, continuing without semantic search");
            None
        }
    };

    run_server(config, storage, llm, vector_store).await
}

/// Run the gRPC server with graceful shutdown.
///
/// # Errors
///
/// Returns an error if the server cannot be started or terminates uncleanly.
pub async fn run_server(
    config: ServerConfig,
    storage: Storage,
    llm: Arc<dyn arlm_llm::LlmBackend + Send + Sync>,
    vector_store: Option<Arc<VectorStore>>,
) -> Result<()> {
    let state = AppState::new(storage, config.clone(), llm, vector_store)?;

    let grpc_service = ArlmGrpcService::new(state);
    let addr = config
        .listen_addr
        .parse()
        .context("failed to parse listen address")?;

    let mut builder = Server::builder();

    if let (Some(cert), Some(key)) = (config.tls_cert(), config.tls_key()) {
        let identity = Identity::from_pem(&load_file(&cert)?, &load_file(&key)?);
        builder = builder.tls_config(ServerTlsConfig::new().identity(identity))?;
        info!(cert = %cert.display(), "gRPC server TLS enabled");
    } else {
        info!("gRPC server running without TLS (dev mode)");
    }

    info!(addr = %addr, "arlm-server listening");

    builder
        .add_service(ArlmServiceServer::new(grpc_service))
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    info!("arlm-server shut down gracefully");
    Ok(())
}

fn load_file(path: &PathBuf) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("failed to read TLS file {}", path.display()))
}

/// Wait for a shutdown signal (SIGINT or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");

        tokio::select! {
            _ = ctrl_c => {
                info!("received SIGINT, shutting down");
            }
            _ = sigterm.recv() => {
                info!("received SIGTERM, shutting down");
            }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        info!("received Ctrl+C, shutting down");
    }
}

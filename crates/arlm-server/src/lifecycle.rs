use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use arlm_proto::proto::arlm_service_client::ArlmServiceClient;
use arlm_proto::proto::arlm_service_server::ArlmServiceServer;
use arlm_storage::{QuestionVectorStore, Storage, VectorStore};
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tracing::{info, warn};

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

    // Single-mode storage: `arlm-storage`'s read paths (`get_chunk`,
    // `get_summary`, `search_summaries`, …) currently assume a single
    // connection via `Storage::conn()`. Opening single-mode keeps both the
    // `conn()`-based read helpers and the `connection()`-based pooled writes
    // (used by indexing) valid. Concurrent handlers serialize on the shared
    // connection mutex, which is acceptable for a local dev server.
    let storage = Storage::open(&config.data_dir).context("failed to open storage")?;

    let llm = AppState::build_llm(&config).context("failed to configure LLM backend")?;

    let vector_store = match VectorStore::open_with_dims(
        &config.data_dir,
        crate::state::embedder_dimension(),
    )
    .await
    {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "vector store unavailable, continuing without semantic search");
            None
        }
    };

    let question_vector_store = match arlm_storage::QuestionVectorStore::open(
        &config.data_dir,
        crate::state::embedder_dimension(),
    ) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "question vector store unavailable, semantic cache lookup disabled");
            None
        }
    };

    run_server(config, storage, llm, vector_store, question_vector_store).await
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
    question_vector_store: Option<Arc<QuestionVectorStore>>,
) -> Result<()> {
    let state = AppState::new(
        storage.clone(),
        config.clone(),
        llm,
        vector_store,
        question_vector_store,
    )?;

    let grpc_service = ArlmServiceServer::new(ArlmGrpcService::new(state));
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
        .add_service(grpc_service)
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    info!("arlm-server shut down gracefully");
    Ok(())
}

fn load_file(path: &PathBuf) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("failed to read TLS file {}", path.display()))
}

/// Query a running server's health over gRPC and print a summary.
///
/// Used by the `arlm-server status` subcommand (and the Docker HEALTHCHECK).
///
/// # Errors
///
/// Returns an error if the config cannot be loaded or the server is unreachable.
pub async fn status_check() -> anyhow::Result<()> {
    let config = ServerConfig::load().context("failed to load server config")?;
    let endpoint = format!("http://{}", config.listen_addr);

    let mut client = ArlmServiceClient::connect(endpoint)
        .await
        .context("failed to connect to arlm-server (is it running?)")?;

    let status = client
        .get_server_status(())
        .await
        .context("GetServerStatus RPC failed")?
        .into_inner();

    println!(
        "OK version={} uptime_s={} active_runs={} total_projects={} total_chunks={} total_summaries={}",
        status.version,
        status.uptime_seconds,
        status.active_runs,
        status.total_projects,
        status.total_chunks,
        status.total_summaries,
    );
    Ok(())
}

/// Wait for a shutdown signal (SIGINT or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = ctrl_c => {
                        info!("received SIGINT, shutting down");
                    }
                    _ = sigterm.recv() => {
                        info!("received SIGTERM, shutting down");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to install SIGTERM handler; waiting on Ctrl+C only");
                let _ = ctrl_c.await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        info!("received Ctrl+C, shutting down");
    }
}

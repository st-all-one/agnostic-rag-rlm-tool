use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::{get, post};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, instrument};

use crate::output;
use crate::util::{data_dir, project_name};

pub use crate::metrics::ArlmMetrics;
pub use arlm_core::events::{EventBus, RlmEvent};

pub use self::handlers::{
    context_handler, events_stream, health, index_handler, mcp_handler, metrics_handler,
    run_handler, search_handler, status_all, status_by_id,
};
pub use self::requests::{ContextRequest, IndexRequest, RunRequest, SearchRequest};
pub use self::state::AppState;
pub use self::status_logic::extract_run_id;

pub mod handlers;
pub mod index_logic;
pub mod requests;
pub mod response;
pub mod run_logic;
pub mod search_logic;
pub mod state;
pub mod status_logic;

/// Configuration for the `serve` subcommand.
pub struct ServeConfig<'a> {
    pub port: u16,
    pub host: &'a str,
    pub project: &'a Path,
    pub verbose: bool,
    pub mcp: bool,
}

/// Start the arlm HTTP server.
///
/// # Errors
/// Returns an error if the storage backend cannot be opened, the listen
/// address cannot be parsed, or the TCP listener fails to bind.
#[instrument(skip_all)]
pub async fn execute(config: ServeConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_serve");

    let pname = project_name(config.project);

    let _storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    info!(host = %config.host, port = %config.port, project = %pname, "starting arlm server");

    output::info(&format!(
        "Starting arlm server on {}:{}",
        config.host, config.port
    ));
    output::info(&format!("Project: {pname}"));

    let metrics = ArlmMetrics::new();
    let event_bus = EventBus::new();

    let state = Arc::new(AppState {
        project: config.project.to_path_buf(),
        project_name: pname,
        verbose: config.verbose,
        metrics,
        event_bus,
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let mut routes = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .route("/events/stream/{run_id}", get(events_stream))
        .route("/status", get(status_all))
        .route("/status/{id}", get(status_by_id))
        .route("/context", post(context_handler))
        .route("/search", post(search_handler))
        .route("/run", post(run_handler))
        .route("/index", post(index_handler));

    if config.mcp {
        routes = routes.route("/mcp", post(mcp_handler));
    }

    let routes = routes
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .context("failed to parse address")?;

    output::success(&format!("Server listening on http://{addr}"));
    println!("\nEndpoints:");
    println!("  GET  /health              - Health check");
    println!("  GET  /metrics             - Prometheus metrics");
    println!("  GET  /events/stream/:run_id - SSE event stream");
    println!("  POST /context             - Build context for a task");
    println!("  POST /search              - Search the project");
    println!("  POST /run                 - Run RLM recursively");
    println!("  GET  /status              - All indexed projects");
    println!("  GET  /status/:id          - Status of a specific run");
    println!("  POST /index               - Index a project directory");
    if config.mcp {
        println!("  POST /mcp                 - MCP (Model Context Protocol) endpoint");
    }
    println!("\nPress Ctrl+C to stop.\n");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("failed to bind TCP listener")?;

    axum::serve(listener, routes.into_make_service())
        .await
        .context("server error")?;

    Ok(())
}

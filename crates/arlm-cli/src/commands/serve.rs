use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use arlm_core::events::{EventBus, RlmEvent};
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::metrics::ArlmMetrics;
use crate::output;
use crate::util::{data_dir, project_name};

/// Shared application state.
#[derive(Clone)]
struct AppState {
    project: PathBuf,
    project_name: String,
    verbose: bool,
    metrics: ArlmMetrics,
    event_bus: EventBus,
}

pub struct ServeConfig<'a> {
    pub port: u16,
    pub host: &'a str,
    pub project: &'a Path,
    pub verbose: bool,
    pub mcp: bool,
}

pub async fn execute(config: ServeConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_serve");

    let pname = project_name(config.project).to_string();

    let _storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

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

#[derive(Serialize)]
struct ApiResponse<T: Serialize> {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn ok(data: T) -> Self {
        Self {
            status: "ok".to_string(),
            data: Some(data),
            error: None,
        }
    }

    fn err(msg: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            data: None,
            error: Some(msg.into()),
        }
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> axum::response::Response {
        let status = if self.status == "ok" {
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, Json(self)).into_response()
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "arlm",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::commands::mcp::JsonRpcRequest>,
) -> impl IntoResponse {
    let mcp_state = crate::commands::mcp::McpState {
        project: state.project.clone(),
        project_name: state.project_name.clone(),
        verbose: state.verbose,
    };
    let response = crate::commands::mcp::handle_jsonrpc(&req, &mcp_state);
    Json(response)
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.record_request();
    let body = state.metrics.render();
    let mut headers = axum::http::HeaderMap::new();
    if let Ok(val) = "text/plain; version=0.0.4; charset=utf-8".parse() {
        headers.insert(axum::http::header::CONTENT_TYPE, val);
    }
    (StatusCode::OK, headers, body)
}

#[derive(Deserialize)]
struct ContextRequest {
    task: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
    /// Agent name for metrics tracking.
    #[serde(default)]
    agent: Option<String>,
}

fn default_top_k() -> usize {
    10
}

async fn context_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ContextRequest>,
) -> impl IntoResponse {
    let result = handle_context(&state, &req).await;
    match result {
        Ok(data) => ApiResponse::ok(data).into_response(),
        Err(e) => {
            if state.verbose {
                output::error(&format!("context error: {e}"));
            }
            ApiResponse::<()>::err(e.to_string()).into_response()
        }
    }
}

async fn handle_context(state: &AppState, req: &ContextRequest) -> Result<serde_json::Value> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let options = arlm_search::SearchOptions {
        tier: arlm_search::SearchTier::Entity,
        top_k: req.top_k,
    };

    let results = hybrid
        .search(&req.task, None, buffer.id, &options, None, Some(&storage))
        .await
        .context("hybrid search failed")?;

    state.metrics.record_search(results.len() as u64);

    // Record agent metrics if agent name provided
    if let Some(ref agent) = req.agent {
        state.metrics.record_agent_request(agent, 0);
    }

    let context = arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt, None)
        .context("failed to build context")?;

    Ok(serde_json::json!({
        "task": req.task,
        "project": state.project_name,
        "context": context,
        "results_count": results.len(),
    }))
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
    file_pattern: Option<String>,
    min_score: Option<f32>,
    /// Agent name for metrics tracking.
    #[serde(default)]
    agent: Option<String>,
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let result = handle_search(&state, &req).await;
    match result {
        Ok(data) => ApiResponse::ok(data).into_response(),
        Err(e) => {
            if state.verbose {
                output::error(&format!("search error: {e}"));
            }
            ApiResponse::<()>::err(e.to_string()).into_response()
        }
    }
}

async fn handle_search(state: &AppState, req: &SearchRequest) -> Result<serde_json::Value> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let options = arlm_search::SearchOptions {
        tier: arlm_search::SearchTier::Entity,
        top_k: req.top_k,
    };

    let results = hybrid
        .search(&req.query, None, buffer.id, &options, None, Some(&storage))
        .await
        .context("hybrid search failed")?;

    let search_results =
        arlm_search::build_search_results(&storage, &results, None).context("failed to build results")?;

    let items: Vec<serde_json::Value> = search_results
        .iter()
        .filter(|r| req.min_score.is_none_or(|min| r.score >= min))
        .filter(|r| {
            #[allow(clippy::unnecessary_map_or)]
            req.file_pattern
                .as_ref()
                .map_or(true, |pat| r.file_path.contains(pat.as_str()))
        })
        .map(|r| {
            serde_json::json!({
                "chunk_id": r.chunk_id,
                "file": r.file_path,
                "line_start": r.line_start,
                "line_end": r.line_end,
                "score": r.score,
                "content": r.content,
                "language": r.language,
            })
        })
        .collect();

    state.metrics.record_search(items.len() as u64);

    // Record agent metrics if agent name provided
    if let Some(ref agent) = req.agent {
        state.metrics.record_agent_request(agent, 0);
    }

    Ok(serde_json::json!({
        "query": req.query,
        "results": items,
        "count": items.len(),
    }))
}

#[derive(Deserialize)]
struct RunRequest {
    task: String,
    #[serde(default = "default_depth")]
    depth: u32,
    #[serde(default = "default_max_nodes")]
    max_nodes: u32,
    backend: Option<String>,
    model: Option<String>,
    /// Agent name for metrics tracking.
    #[serde(default)]
    agent: Option<String>,
}

fn default_depth() -> u32 {
    3
}

fn default_max_nodes() -> u32 {
    50
}

async fn run_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let result = handle_run(&state, &req).await;
    match result {
        Ok(data) => ApiResponse::ok(data).into_response(),
        Err(e) => {
            if state.verbose {
                output::error(&format!("run error: {e}"));
            }
            ApiResponse::<()>::err(e.to_string()).into_response()
        }
    }
}

async fn handle_run(state: &AppState, req: &RunRequest) -> Result<serde_json::Value> {
    let backend_name = req.backend.as_deref().unwrap_or("ollama");
    let kind: arlm_llm::BackendKind = backend_name.parse().context("failed to parse backend")?;

    let api_key = std::env::var(match kind {
        arlm_llm::BackendKind::OpenAI => "OPENAI_API_KEY",
        arlm_llm::BackendKind::Anthropic => "ANTHROPIC_API_KEY",
        arlm_llm::BackendKind::Gemini => "GEMINI_API_KEY",
        arlm_llm::BackendKind::Ollama => "",
        arlm_llm::BackendKind::DeepSeek => "DEEPSEEK_API_KEY",
        arlm_llm::BackendKind::MiMo => "MIMO_API_KEY",
    })
    .ok();

    let llm_backend =
        arlm_llm::get_backend(&kind, api_key, None).context("failed to create LLM backend")?;

    let run_id = format!("run-{}", uuid::Uuid::now_v7().as_simple());

    let input = arlm_core::StartRunInput {
        run_id: std::sync::Arc::from(run_id.as_str()),
        task: req.task.clone(),
        backend: match kind {
            arlm_llm::BackendKind::OpenAI => arlm_core::RlmBackend::OpenAi,
            arlm_llm::BackendKind::Anthropic => arlm_core::RlmBackend::Anthropic,
            arlm_llm::BackendKind::Gemini => arlm_core::RlmBackend::Gemini,
            arlm_llm::BackendKind::Ollama => arlm_core::RlmBackend::Ollama,
            arlm_llm::BackendKind::DeepSeek => arlm_core::RlmBackend::DeepSeek,
            arlm_llm::BackendKind::MiMo => arlm_core::RlmBackend::MiMo,
        },
        model: req.model.clone(),
        project: state.project_name.clone(),
        max_depth: req.depth,
        max_nodes: req.max_nodes,
        ..Default::default()
    };

    let result = arlm_core::run_rlm_engine_with_events(input, llm_backend, state.event_bus.clone())
        .await
        .context("RLM engine failed")?;

    state.metrics.record_node();

    // Record agent metrics if agent name provided
    if let Some(ref agent) = req.agent {
        state.metrics.record_agent_request(agent, 0);
    }

    Ok(serde_json::json!({
        "run_id": result.run_id,
        "task": req.task,
        "result": result.final_output,
        "duration_ms": result.stats.duration_ms,
        "nodes_visited": result.stats.nodes_visited,
        "max_depth": result.stats.max_depth_seen,
    }))
}

#[derive(Deserialize)]
struct IndexRequest {
    path: Option<String>,
    #[serde(default = "default_chunk_size")]
    chunk_size: usize,
}

fn default_chunk_size() -> usize {
    512
}

async fn index_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IndexRequest>,
) -> impl IntoResponse {
    let result = handle_index(&state, &req);
    match result {
        Ok(data) => ApiResponse::ok(data).into_response(),
        Err(e) => {
            if state.verbose {
                output::error(&format!("index error: {e}"));
            }
            ApiResponse::<()>::err(e.to_string()).into_response()
        }
    }
}

fn handle_index(state: &AppState, req: &IndexRequest) -> Result<serde_json::Value> {
    let index_path = match &req.path {
        Some(p) => PathBuf::from(p)
            .canonicalize()
            .with_context(|| format!("failed to resolve path: {p}"))?,
        None => state.project.clone(),
    };

    let data_dir = data_dir();

    let storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?;

    let _buffer_id = if let Some(buf) = buffer {
        buf.id
    } else {
        storage
            .insert_buffer(&arlm_storage::sqlite::buffers::NewBuffer {
                name: state.project_name.clone(),
                path: index_path.to_string_lossy().to_string(),
            })
            .context("failed to create buffer")?
    };

    let knowledge = arlm_memory::KnowledgeEngine::new(storage);
    let opts = arlm_memory::knowledge::IndexOptions {
        max_chunk_bytes: req.chunk_size * 4,
        ..Default::default()
    };

    let result = knowledge
        .index_directory(&state.project_name, &index_path, &opts)
        .context("failed to index directory")?;

    Ok(serde_json::json!({
        "project": state.project_name,
        "path": index_path.display().to_string(),
        "files_processed": result.files_processed,
        "chunks_created": result.chunks_created,
        "duration_ms": result.duration_ms,
    }))
}

async fn status_all(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let result = handle_status_all(&state);
    match result {
        Ok(data) => ApiResponse::ok(data).into_response(),
        Err(e) => ApiResponse::<()>::err(e.to_string()).into_response(),
    }
}

fn handle_status_all(_state: &AppState) -> Result<serde_json::Value> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffers = storage.list_buffers().context("failed to list buffers")?;

    let items: Vec<serde_json::Value> = buffers
        .iter()
        .map(|b| {
            serde_json::json!({
                "id": b.id,
                "name": b.name,
                "path": b.path,
                "total_chunks": b.total_chunks,
                "total_files": b.total_files,
                "last_indexed_at": b.last_indexed_at,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "projects": items,
        "count": buffers.len(),
    }))
}

async fn status_by_id(
    State(state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let data = handle_status_by_id(&state, &run_id);
    ApiResponse::ok(data).into_response()
}

fn handle_status_by_id(_state: &AppState, run_id: &str) -> serde_json::Value {
    let storage = match arlm_storage::Storage::open(&data_dir()) {
        Ok(s) => s,
        Err(e) => {
            return serde_json::json!({
                "run_id": run_id,
                "status": "error",
                "message": format!("Failed to open storage: {e}"),
            });
        }
    };

    match storage.get_run(run_id) {
        Ok(Some(run)) => {
            let usage = storage.get_run_model_usage(run_id).unwrap_or_default();
            let models: Vec<serde_json::Value> = usage
                .iter()
                .map(|u| {
                    serde_json::json!({
                        "model": u.model,
                        "calls": u.calls,
                        "input_tokens": u.input_tokens,
                        "output_tokens": u.output_tokens,
                        "cost": u.cost,
                    })
                })
                .collect();

            serde_json::json!({
                "run_id": run.id,
                "task": run.task,
                "status": run.status,
                "agent": run.agent,
                "started_at": run.started_at,
                "finished_at": run.finished_at,
                "duration_ms": run.duration_ms,
                "total_cost": run.total_cost,
                "total_tokens": run.total_tokens,
                "models": models,
            })
        }
        Ok(None) => {
            serde_json::json!({
                "run_id": run_id,
                "status": "not_found",
                "message": "Run not found",
            })
        }
        Err(e) => {
            serde_json::json!({
                "run_id": run_id,
                "status": "error",
                "message": format!("Failed to query run: {e}"),
            })
        }
    }
}

fn extract_run_id(event: &RlmEvent) -> &str {
    match event {
        RlmEvent::RunStart { run_id, .. }
        | RlmEvent::NodeStart { run_id, .. }
        | RlmEvent::NodePlan { run_id, .. }
        | RlmEvent::NodeSolve { run_id, .. }
        | RlmEvent::NodeSynthesize { run_id, .. }
        | RlmEvent::CostUpdate { run_id, .. }
        | RlmEvent::CacheHit { run_id, .. }
        | RlmEvent::NodeEnd { run_id, .. }
        | RlmEvent::RunEnd { run_id, .. } => run_id,
    }
}

async fn events_stream(
    State(state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel(256);
    let event_bus = state.event_bus.clone();
    let dir = data_dir();

    // Replay past events from JSONL log if available
    let log_path = dir.join(format!("run_{run_id}.events.jsonl"));
    if let Ok(past_events) = arlm_core::jsonl_logger::JsonlEventLogger::replay(&log_path) {
        for event in &past_events {
            if extract_run_id(event) == run_id {
                if let Ok(json) = serde_json::to_string(event) {
                    let _ = tx.send(Ok(Event::default().data(json))).await;
                }
            }
        }
    }

    // Stream live events from the broadcast channel
    let mut rx_bus = event_bus.subscribe();
    let stream_run_id = run_id.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = rx_bus.recv() => {
                    match result {
                        Ok(event) => {
                            if extract_run_id(&event) == stream_run_id {
                                if let Ok(json) = serde_json::to_string(&event) {
                                    if tx.send(Ok(Event::default().data(json))).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream).keep_alive(axum::response::sse::KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn unique_project_name() -> String {
        format!("test-proj-{}", uuid::Uuid::now_v7().as_simple())
    }

    fn app_state(tmp: &tempfile::TempDir, project_name: &str) -> Arc<AppState> {
        Arc::new(AppState {
            project: tmp.path().join(project_name),
            project_name: project_name.to_string(),
            verbose: false,
            metrics: ArlmMetrics::new(),
            event_bus: EventBus::new(),
        })
    }

    fn app(state: Arc<AppState>) -> Router {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        Router::new()
            .route("/health", get(health))
            .route("/metrics", get(metrics_handler))
            .route("/events/stream/{run_id}", get(events_stream))
            .route("/status", get(status_all))
            .route("/status/{id}", get(status_by_id))
            .route("/context", post(context_handler))
            .route("/search", post(search_handler))
            .route("/run", post(run_handler))
            .route("/index", post(index_handler))
            .layer(cors)
            .with_state(state)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let proj = unique_project_name();
        let state = app_state(&tmp, &proj);
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "arlm");
    }

    #[tokio::test]
    async fn test_status_by_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let proj = unique_project_name();
        let state = app_state(&tmp, &proj);
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/status/run-abc123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["data"]["run_id"], "run-abc123");
    }

    #[tokio::test]
    async fn test_events_stream_returns_sse_content_type() {
        let tmp = tempfile::TempDir::new().unwrap();
        let proj = unique_project_name();
        let state = app_state(&tmp, &proj);
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/events/stream/run-test-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.contains("text/event-stream"),
            "expected text/event-stream, got {ct}"
        );
    }

    #[tokio::test]
    async fn test_events_stream_replays_past_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let proj = unique_project_name();
        let dir = data_dir();
        std::fs::create_dir_all(&dir).unwrap();

        let log_path = dir.join("run_run-replay.events.jsonl");
        let event = RlmEvent::RunStart {
            run_id: Arc::from("run-replay"),
            task: "replay test".to_string(),
            backend: "mock".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 10,
            max_budget: 1.0,
            started_at_ms: 1_700_000_000_000,
        };
        let line = serde_json::to_string(&event).unwrap();
        std::fs::write(&log_path, format!("{line}\n")).unwrap();

        let state = app_state(&tmp, &proj);
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/events/stream/run-replay")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("data:"),
            "expected data: line in SSE body, got: {text}"
        );
        assert!(
            text.contains("replay test"),
            "expected replay test task in body, got: {text}"
        );
    }

    #[tokio::test]
    async fn test_events_stream_no_log_is_ok() {
        let tmp = tempfile::TempDir::new().unwrap();
        let proj = unique_project_name();
        let state = app_state(&tmp, &proj);

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/events/stream/run-nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/event-stream"));
    }

    #[test]
    fn test_extract_run_id() {
        let event = RlmEvent::CostUpdate {
            run_id: Arc::from("run-42"),
            spent: 0.5,
            budget: 1.0,
        };
        assert_eq!(extract_run_id(&event), "run-42");
    }
}

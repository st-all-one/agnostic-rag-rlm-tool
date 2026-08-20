use std::convert::Infallible;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, instrument};

use crate::commands::serve::requests::{ContextRequest, IndexRequest, RunRequest, SearchRequest};
use crate::commands::serve::response::ApiResponse;
use crate::commands::serve::state::AppState;
use crate::commands::serve::{index_logic, run_logic, search_logic, status_logic};
use crate::output;

#[instrument(skip_all)]
pub async fn health() -> impl IntoResponse {
    debug!("health check");
    Json(serde_json::json!({
        "status": "ok",
        "service": "arlm",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[instrument(skip_all)]
pub async fn mcp_handler(
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

#[instrument(skip_all)]
pub async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.record_request();
    let body = state.metrics.render();
    let mut headers = axum::http::HeaderMap::new();
    if let Ok(val) = "text/plain; version=0.0.4; charset=utf-8".parse() {
        headers.insert(axum::http::header::CONTENT_TYPE, val);
    }
    (StatusCode::OK, headers, body)
}

#[instrument(skip_all)]
pub async fn context_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ContextRequest>,
) -> impl IntoResponse {
    debug!(task = %req.task, top_k = req.top_k, "context request");
    let result = search_logic::handle_context(&state, &req).await;
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

#[instrument(skip_all)]
pub async fn search_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    debug!(query = %req.query, top_k = req.top_k, "search request");
    let result = search_logic::handle_search(&state, &req).await;
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

#[instrument(skip_all)]
pub async fn run_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    debug!(task = %req.task, depth = req.depth, max_nodes = req.max_nodes, "run request");
    let result = run_logic::handle_run(&state, &req).await;
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

#[instrument(skip_all)]
pub async fn index_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IndexRequest>,
) -> impl IntoResponse {
    debug!(chunk_size = req.chunk_size, "index request");
    let result = index_logic::handle_index(&state, &req);
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

#[instrument(skip_all)]
pub async fn status_all(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let result = status_logic::handle_status_all(&state);
    match result {
        Ok(data) => ApiResponse::ok(data).into_response(),
        Err(e) => ApiResponse::<()>::err(e.to_string()).into_response(),
    }
}

#[instrument(skip_all)]
pub async fn status_by_id(
    State(state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    debug!(run_id = %run_id, "status by id");
    let data = status_logic::handle_status_by_id(&state, &run_id);
    ApiResponse::ok(data).into_response()
}

#[instrument(skip_all)]
pub async fn events_stream(
    State(state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    debug!(run_id = %run_id, "events stream");
    let (tx, rx) = mpsc::channel(256);
    let event_bus = state.event_bus.clone();
    let dir = crate::util::data_dir();

    // Replay past events from JSONL log if available
    let log_path = dir.join(format!("run_{run_id}.events.jsonl"));
    if let Ok(past_events) = arlm_core::jsonl_logger::JsonlEventLogger::replay(&log_path) {
        for event in &past_events {
            if status_logic::extract_run_id(event) == run_id {
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
                            if status_logic::extract_run_id(&event) == stream_run_id {
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

    let rx_stream = ReceiverStream::new(rx);
    Sse::new(rx_stream).keep_alive(axum::response::sse::KeepAlive::default())
}

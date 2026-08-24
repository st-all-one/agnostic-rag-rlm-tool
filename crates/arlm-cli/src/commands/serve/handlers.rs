use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use tracing::{debug, instrument};

use crate::commands::serve::requests::{ContextRequest, IndexRequest, SearchRequest};
use crate::commands::serve::response::ApiResponse;
use crate::commands::serve::state::AppState;
use crate::commands::serve::{index_logic, search_logic, status_logic};
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
    State(state): State<std::sync::Arc<AppState>>,
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
pub async fn metrics_handler(State(state): State<std::sync::Arc<AppState>>) -> impl IntoResponse {
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
    State(state): State<std::sync::Arc<AppState>>,
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
    State(state): State<std::sync::Arc<AppState>>,
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
pub async fn index_handler(
    State(state): State<std::sync::Arc<AppState>>,
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
pub async fn status_all(
    State(state): State<std::sync::Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let result = status_logic::handle_status_all(&state);
    match result {
        Ok(data) => (StatusCode::OK, Json(data)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#![allow(
    unsafe_code,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_borrow,
    clippy::unnecessary_literal_bound,
    clippy::float_cmp,
    clippy::duration_suboptimal_units,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use std::sync::Arc;

use arlm_cli::commands::serve::{
    AppState, ArlmMetrics, EventBus, RlmEvent, context_handler, events_stream, extract_run_id,
    health, index_handler, metrics_handler, run_handler, search_handler, status_all, status_by_id,
};
use arlm_cli::util::data_dir;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use tower::ServiceExt;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

fn unique_project_name() -> String {
    format!("test-proj-{}", Uuid::now_v7().as_simple())
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

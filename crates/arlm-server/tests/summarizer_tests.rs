#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::pedantic
)]

//! Unit tests for the summarizer (cost, progress, strategy) that were
//! extracted from inline `#[cfg(test)]` modules into the integration suite.

use arlm_server::summarizer::cost::{CostEstimate, estimate_cost, estimate_incremental_cost};
use arlm_server::summarizer::progress::{ProgressSnapshot, ProgressTracker};
use arlm_server::summarizer::strategy::{RawChunk, build_summary_prompt, parse_summary_response};

// ── Progress ────────────────────────────────────────────────────────────────

#[test]
fn test_progress_tracker() {
    let tracker = ProgressTracker::new();
    assert!(!tracker.is_running());

    tracker.start(100);
    assert!(tracker.is_running());

    tracker.update("file.rs", 50);
    let snapshot = tracker.progress();
    assert_eq!(snapshot.completed, 50);
    assert_eq!(snapshot.total, 100);
    assert!((snapshot.percentage() - 50.0).abs() < f64::EPSILON);

    tracker.finish();
    assert!(!tracker.is_running());
}

#[test]
fn test_progress_snapshot_fields() {
    let tracker = ProgressTracker::new();
    tracker.start(10);
    tracker.update("src/main.rs", 3);
    tracker.set_message("summarizing");
    let snapshot: ProgressSnapshot = tracker.progress();
    assert_eq!(snapshot.current_file, "src/main.rs");
    assert_eq!(snapshot.message, "summarizing");
}

// ── Cost ───────────────────────────────────────────────────────────────────

#[test]
fn test_estimate_cost() {
    let estimate: CostEstimate = estimate_cost(100, 5, 0.01);
    assert!(estimate.cost_usd > 0.0);
    assert!(estimate.llm_calls > 0);
    assert!(estimate.duration_seconds > 0.0);
}

#[test]
fn test_estimate_incremental_cost() {
    let full = estimate_cost(100, 5, 0.01);
    let incremental = estimate_incremental_cost(10, 100, 5, 0.01);
    assert!(incremental.cost_usd < full.cost_usd);
}

// ── Strategy ───────────────────────────────────────────────────────────────

#[test]
fn test_build_summary_prompt_file() {
    let chunks = vec![RawChunk {
        id: 1,
        content: "fn main() {}".to_string(),
        file_path: "src/main.rs".to_string(),
    }];

    let prompt = build_summary_prompt(&chunks, "file").unwrap();
    assert!(prompt.contains("File: src/main.rs"));
    assert!(prompt.contains("fn main() {}"));
}

#[test]
fn test_build_summary_prompt_module() {
    let chunks = vec![
        RawChunk {
            id: 1,
            content: "fn a() {}".to_string(),
            file_path: "src/auth/mod.rs".to_string(),
        },
        RawChunk {
            id: 2,
            content: "fn b() {}".to_string(),
            file_path: "src/auth/handlers.rs".to_string(),
        },
    ];

    let prompt = build_summary_prompt(&chunks, "module").unwrap();
    assert!(prompt.contains("Files:"));
    assert!(prompt.contains("src/auth/mod.rs"));
}

#[test]
fn test_build_summary_prompt_empty() {
    let chunks = vec![];
    assert!(build_summary_prompt(&chunks, "file").is_err());
}

#[test]
fn test_parse_summary_response() {
    let response = "This is a summary of the code.";
    let result = parse_summary_response(response).unwrap();
    assert_eq!(result, "This is a summary of the code.");
}

#[test]
fn test_parse_summary_response_empty() {
    let response = "";
    assert!(parse_summary_response(response).is_err());
}

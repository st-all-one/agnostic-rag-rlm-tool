//! Integration tests validating the generated protobuf/tonic types.
//!
//! These tests assert the contract emitted by `build.rs` from the `.proto`
//! sources: key messages, enums, and field accessors must exist and behave
//! as the downstream `arlm-server`/`arlm-cli` crates expect.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_proto::proto::*;

#[test]
fn test_run_result_fields_and_cost() {
    let stats = RunStats {
        nodes_visited: 7,
        max_depth_reached: 3,
        total_tokens: 1_024,
        total_cost_usd: 0.42,
        duration_ms: 1_250.0,
    };

    let run = RunResult {
        run_id: "run-1".into(),
        status: RunStatus::StatusCompleted as i32,
        answer: "done".into(),
        stats: Some(stats),
        total_cost: 0.42,
    };

    assert_eq!(run.run_id, "run-1");
    assert_eq!(run.status, RunStatus::StatusCompleted as i32);
    assert_eq!(run.stats.as_ref().unwrap().total_tokens, 1_024);
    assert!((run.total_cost - 0.42).abs() < f64::EPSILON);
}

#[test]
fn test_search_request_with_hybrid_tier() {
    let req = SearchRequest {
        project: "p".into(),
        query: "find auth".into(),
        max_results: 10,
        tier: SearchTier::TierHybrid as i32,
        include_summaries: true,
        include_raw: true,
    };

    assert_eq!(req.tier, SearchTier::TierHybrid as i32);
    assert_eq!(req.max_results, 10);
}

#[test]
fn test_session_info_fields() {
    let session = SessionInfo {
        session_id: "s-1".into(),
        project: "p".into(),
        title: "t".into(),
        created_at: None,
        turn_count: 2,
    };

    assert_eq!(session.session_id, "s-1");
    assert_eq!(session.turn_count, 2);
    assert!(session.created_at.is_none());
}

#[test]
fn test_add_session_turn_request_fields() {
    let req = AddSessionTurnRequest {
        session_id: "s-1".into(),
        query: "how?".into(),
        response: "answer".into(),
    };

    assert_eq!(req.session_id, "s-1");
    assert_eq!(req.query, "how?");
    assert_eq!(req.response, "answer");
}

#[test]
fn test_enum_variants_present() {
    assert_eq!(SearchTier::TierBm25 as i32, 0);
    assert_eq!(SearchTier::TierSemantic as i32, 1);
    assert_eq!(SearchTier::TierHybrid as i32, 2);
    assert_eq!(SearchTier::TierEntity as i32, 3);

    assert_eq!(RunStatus::StatusPending as i32, 0);
    assert_eq!(RunStatus::StatusRunning as i32, 1);
    assert_eq!(RunStatus::StatusCompleted as i32, 2);
    assert_eq!(RunStatus::StatusFailed as i32, 3);
    assert_eq!(RunStatus::StatusCancelled as i32, 4);

    assert_eq!(SummaryScope::ScopeFile as i32, 0);
    assert_eq!(SummaryScope::ScopeModule as i32, 1);
    assert_eq!(SummaryScope::ScopeProject as i32, 2);
}

#[test]
fn test_service_modules_resolve() {
    // Ensure the generated tonic service plumbing exists for downstream use.
    // `ArlmServiceClient` is the concrete client referenced by `arlm-cli`.
    let client_type = std::any::type_name::<
        arlm_service_client::ArlmServiceClient<tonic::transport::Channel>,
    >();
    assert!(client_type.contains("ArlmServiceClient"));
}

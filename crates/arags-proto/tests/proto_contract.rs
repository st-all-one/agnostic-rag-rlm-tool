//! Integration tests validating the generated protobuf/tonic types.
//!
//! These tests assert the contract emitted by `build.rs` from the `.proto`
//! sources: key messages, enums, and field accessors must exist and behave
//! as the downstream `arags-server`/`arags-cli` crates expect.
//!
//! NOTE: messages tied to the removed legacy RLM run/summarize pipeline
//! (`RunResult`, `RunStatus`, `RunStats`, `SummaryScope`, `SessionInfo`, …)
//! are intentionally absent — they were deleted in plans 019/020.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arags_proto::proto::*;

#[test]
fn test_search_request_with_hybrid_tier() {
    let req = SearchRequest {
        project: "p".into(),
        query: "find auth".into(),
        max_results: 10,
        tier: SearchTier::TierHybrid as i32,
    };

    assert_eq!(req.tier, SearchTier::TierHybrid as i32);
    assert_eq!(req.max_results, 10);
}

#[test]
fn test_enum_variants_present() {
    // Plan 020: `UNSPECIFIED = 0` is the wire default so the server can apply
    // its `[search].tier` default; explicit tiers start at 1.
    assert_eq!(SearchTier::Unspecified as i32, 0);
    assert_eq!(SearchTier::TierBm25 as i32, 1);
    assert_eq!(SearchTier::TierSemantic as i32, 2);
    assert_eq!(SearchTier::TierHybrid as i32, 3);
    assert_eq!(SearchTier::TierEntity as i32, 4);

    assert_eq!(InvalidateMode::Stale as i32, 0);
    assert_eq!(InvalidateMode::Delete as i32, 1);
}

#[test]
fn test_service_modules_resolve() {
    // Ensure the generated tonic service plumbing exists for downstream use.
    // `AragsServiceClient` is the concrete client referenced by `arags-cli`.
    let client_type = std::any::type_name::<
        arags_service_client::AragsServiceClient<tonic::transport::Channel>,
    >();
    assert!(client_type.contains("AragsServiceClient"));
}

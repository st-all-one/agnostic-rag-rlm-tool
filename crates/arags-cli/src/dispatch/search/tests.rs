//! Unit tests for the rethought query CLI surface (issue `agnostic-rlm-rs-7aa8`).
//!
//! These tests exercise the **pure decision logic** (no live gRPC server or
//! real LLM required) plus the mock-LLM harness from `b020`
//! (`digest_chunks`/`MockLlmBackend`) for the `ask` digest path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::cli::{Cli, Commands};
use crate::commands::test_helpers::{MockLlmBackend, clean_digest_reply};
use clap::Parser;
use tokio::runtime::Runtime;

#[test]
fn query_alias_is_deprecated_and_maps_to_ask_variant() {
    // The deprecated `query` alias must still parse and resolve to the
    // `Commands::Query` variant (which forwards to `ask`).
    let cli = Cli::parse_from(["arags", "query", "how does login work?"]);
    assert!(matches!(cli.command, Commands::Query { .. }));

    // `ask` is the new primary entrypoint.
    let cli = Cli::parse_from(["arags", "ask", "how does login work?"]);
    assert!(matches!(cli.command, Commands::Ask { .. }));

    // The deprecation message must point users at `ask`.
    let msg = query_deprecation_message();
    assert!(msg.contains("deprecated"), "message must flag deprecation");
    assert!(msg.contains("ask"), "message must redirect to `ask`");
}

#[test]
fn ask_without_cache_id_invokes_llm_digest() {
    // Default `ask` (no --cache-id) must imply the LLM digest path.
    assert!(
        ask_invokes_llm(None),
        "`ask` without --cache-id must invoke the LLM digest by default"
    );
    assert!(
        matches!(resolve_ask_action(None), AskAction::Digest),
        "default ask resolves to Digest"
    );
}

#[test]
fn ask_with_cache_id_avoids_llm() {
    // `--cache-id` must be a deterministic lookup (no LLM).
    assert!(
        !ask_invokes_llm(Some("018f3c-deadbeef")),
        "`ask --cache-id` must NOT invoke the LLM"
    );
    match resolve_ask_action(Some("018f3c-deadbeef".to_string())) {
        AskAction::CacheLookup(id) => assert_eq!(id, "018f3c-deadbeef"),
        AskAction::Digest => panic!("cache-id must route to CacheLookup, not Digest"),
    }
    // An empty cache-id falls back to the digest path (invalid lookup target).
    assert!(
        ask_invokes_llm(Some("")),
        "empty --cache-id falls back to LLM digest"
    );
}

#[test]
fn search_is_objective_and_never_invokes_llm() {
    assert!(
        !SEARCH_INVOKES_LLM,
        "`search` must never invoke the client LLM digest"
    );
    assert!(
        !SEARCH_CONTEXT_INVOKES_LLM,
        "`search --context` uses server BuildContext, no client LLM"
    );
}

#[test]
fn ask_digest_path_uses_mock_llm_digest() {
    // Re-use the b020 mock-LLM harness to prove the `ask` Digest path digests
    // through `digest_chunks` (the same routine `qa_cache::run_ask` calls on a
    // cache miss). A real `ask` call routes here when no `--cache-id` is given.
    let rt = Runtime::new().expect("failed to build tokio runtime");
    let backend = MockLlmBackend::new(clean_digest_reply());
    let out = crate::commands::qa_cache::digest_chunks(
        &rt,
        &backend,
        "Question: x\n\nContext:\n# f\n```\ncode\n```",
        "mock",
    )
    .expect("digest_chunks should succeed");
    assert_eq!(out, "The answer is 42, per the source chunks.");
}

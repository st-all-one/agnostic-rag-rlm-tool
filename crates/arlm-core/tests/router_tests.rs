#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::manual_range_contains
)]

use arlm_core::router::{DepthRouter, MAX_DEPTH};

#[test]
fn test_new_router() {
    let router = DepthRouter::new();
    assert_eq!(router.default_depth, 2);
}

#[test]
fn test_with_default_depth() {
    let router = DepthRouter::with_default_depth(3);
    assert_eq!(router.default_depth, 3);
}

#[test]
fn test_simple_query_shallow_depth() {
    let router = DepthRouter::new();
    let depth = router.suggest_depth("what is a hash map");
    assert!(depth <= 2, "simple query should route shallow, got {depth}");
}

#[test]
fn test_complex_query_deeper_depth() {
    let router = DepthRouter::new();
    let depth = router.suggest_depth(
        "how to implement async concurrency with atomic transactions and memory ownership patterns",
    );
    assert!(depth >= 2, "complex query should route deep, got {depth}");
}

#[test]
fn test_empty_query() {
    let router = DepthRouter::new();
    let depth = router.suggest_depth("");
    assert!(depth >= 1 && depth <= MAX_DEPTH);
}

#[test]
fn test_technical_query() {
    let router = DepthRouter::new();
    let depth = router.suggest_depth("explain trait lifetime generics atomic channel mutex");
    assert!(
        depth >= 2,
        "technical query should route deeper, got {depth}"
    );
}

#[test]
fn test_record_outcome() {
    let mut router = DepthRouter::new();
    router.record_outcome(2, true);
    router.record_outcome(2, true);
    router.record_outcome(2, false);
    assert_eq!(router.attempts(2), 3);
    assert_eq!(router.successes(2), 2);
}

#[test]
fn test_record_outcome_caps_at_max_depth() {
    let mut router = DepthRouter::new();
    router.record_outcome(100, true);
    assert_eq!(router.attempts(MAX_DEPTH), 1);
}

#[test]
fn test_history_influences_routing() {
    let mut router = DepthRouter::new();
    for _ in 0..10 {
        router.record_outcome(3, true);
    }
    for _ in 0..10 {
        router.record_outcome(1, false);
    }
    let depth = router.suggest_depth("get value from map");
    assert!(
        depth >= 2,
        "historical success should influence routing, got {depth}"
    );
}

#[test]
fn test_high_success_rate_adjusts_down() {
    let mut router = DepthRouter::new();
    for _ in 0..10 {
        router.record_outcome(3, true);
    }
    let adjustment = router.budget_adjustment();
    assert!(adjustment <= 0, "high success rate should adjust down");
}

#[test]
fn test_low_success_rate_adjusts_up() {
    let mut router = DepthRouter::new();
    for _ in 0..10 {
        router.record_outcome(3, false);
    }
    let adjustment = router.budget_adjustment();
    assert!(adjustment >= 0, "low success rate should adjust up or stay");
}

#[test]
fn test_best_performing_depth() {
    let mut router = DepthRouter::new();
    router.record_outcome(2, false);
    router.record_outcome(2, false);
    router.record_outcome(3, true);
    router.record_outcome(3, true);
    router.record_outcome(3, true);
    let best = router.best_performing_depth();
    assert_eq!(best, Some(3));
}

#[test]
fn test_best_performing_depth_none_when_no_data() {
    let router = DepthRouter::new();
    assert!(router.best_performing_depth().is_none());
}

#[test]
fn test_complexity_score_simple() {
    let score = DepthRouter::complexity_score("hello");
    assert!(
        score < 0.3,
        "simple query should have low score, got {score}"
    );
}

#[test]
fn test_complexity_score_complex() {
    let score = DepthRouter::complexity_score(
        "how to refactor async concurrency with atomic memory ownership lifetime traits",
    );
    assert!(
        score > 0.5,
        "complex query should have high score, got {score}"
    );
}

#[test]
fn test_suggest_depth_never_exceeds_max() {
    let router = DepthRouter::new();
    let depth = router.suggest_depth("why how explain analyze compare evaluate refactor optimize architect design integrate implement debug fix troubleshoot migrate transform redesign async concurrency trait lifecycle memory ownership borrow lifetime generics macro unsafe atomic channel mutex serialization pagination transaction");
    assert!(depth <= MAX_DEPTH);
}

#[test]
fn test_suggest_depth_at_least_one() {
    let router = DepthRouter::new();
    let depth = router.suggest_depth("");
    assert!(depth >= 1);
}

#[test]
fn test_default_trait() {
    let router = DepthRouter::default();
    assert_eq!(router.default_depth, 2);
}

#[test]
fn test_select_model_depth_0() {
    let router = DepthRouter::new();
    assert_eq!(router.select_model(0, None), "gpt-4o");
}

#[test]
fn test_select_model_depth_0_custom() {
    let router = DepthRouter::new();
    assert_eq!(
        router.select_model(0, Some("claude-sonnet")),
        "claude-sonnet"
    );
}

#[test]
fn test_select_model_depth_1() {
    let router = DepthRouter::new();
    assert_eq!(router.select_model(1, None), "gpt-4o-mini");
}

#[test]
fn test_select_model_deep() {
    let router = DepthRouter::new();
    assert_eq!(router.select_model(10, None), "gpt-4o-mini");
}

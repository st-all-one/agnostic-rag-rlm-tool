#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_borrow,
    clippy::unnecessary_literal_bound,
    clippy::duration_suboptimal_units,
    unsafe_code
)]

use arlm_cli::metrics::ArlmMetrics;

#[test]
fn test_new_metrics_zero() {
    let m = ArlmMetrics::new();
    let rendered = m.render();
    assert!(rendered.contains("arlm_requests_total 0"));
    assert!(rendered.contains("arlm_search_results_total 0"));
    assert!(rendered.contains("arlm_cache_hits_total 0"));
    assert!(rendered.contains("arlm_nodes_total 0"));
}

#[test]
fn test_record_request() {
    let m = ArlmMetrics::new();
    m.record_request();
    m.record_request();
    let rendered = m.render();
    assert!(rendered.contains("arlm_requests_total 2"));
}

#[test]
fn test_record_search() {
    let m = ArlmMetrics::new();
    m.record_search(5);
    let rendered = m.render();
    assert!(rendered.contains("arlm_search_results_total 5"));
}

#[test]
fn test_record_cache_hit() {
    let m = ArlmMetrics::new();
    m.record_cache_hit();
    m.record_cache_hit();
    m.record_cache_hit();
    let rendered = m.render();
    assert!(rendered.contains("arlm_cache_hits_total 3"));
}

#[test]
fn test_record_node() {
    let m = ArlmMetrics::new();
    m.record_node();
    let rendered = m.render();
    assert!(rendered.contains("arlm_nodes_total 1"));
}

#[test]
fn test_render_prometheus_format() {
    let m = ArlmMetrics::new();
    m.record_request();
    let rendered = m.render();
    assert!(rendered.starts_with("# HELP"));
    assert!(rendered.contains("# TYPE arlm_requests_total counter"));
    assert!(rendered.contains("# TYPE arlm_search_results_total counter"));
    assert!(rendered.contains("# TYPE arlm_cache_hits_total counter"));
    assert!(rendered.contains("# TYPE arlm_nodes_total counter"));
}

#[test]
fn test_metrics_clone_shares_state() {
    let m1 = ArlmMetrics::new();
    let m2 = m1.clone();
    m1.record_request();
    let rendered = m2.render();
    assert!(rendered.contains("arlm_requests_total 1"));
}

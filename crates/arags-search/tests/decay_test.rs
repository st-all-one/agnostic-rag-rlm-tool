#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_wrap,
    clippy::float_cmp
)]

use arags_search::decay::DecayConfig;
use std::time::SystemTime;

#[test]
fn test_decay_config_default() {
    let cfg = DecayConfig::default();
    assert!((cfg.lambda - 0.01).abs() < f64::EPSILON);
    assert!(cfg.enabled);
}

#[test]
fn test_decay_config_new() {
    let cfg = DecayConfig::new(0.05);
    assert!((cfg.lambda - 0.05).abs() < f64::EPSILON);
    assert!(cfg.enabled);
}

#[test]
fn test_decay_config_disabled() {
    let cfg = DecayConfig::disabled();
    assert!(!cfg.enabled);
}

#[test]
fn test_score_no_decay_when_disabled() {
    let cfg = DecayConfig::disabled();
    let score = cfg.score(1.0, 100.0);
    assert!((score - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_score_zero_age() {
    let cfg = DecayConfig::default();
    let score = cfg.score(0.8, 0.0);
    assert!((score - 0.8).abs() < f32::EPSILON);
}

#[test]
fn test_score_negative_age_treated_as_zero() {
    let cfg = DecayConfig::default();
    let score = cfg.score(0.8, -5.0);
    assert!((score - 0.8).abs() < f32::EPSILON);
}

#[test]
fn test_score_decay_factor_one_hour() {
    let cfg = DecayConfig::new(0.01);
    let score = cfg.score(1.0, 1.0);
    // exp(-0.01 * 1) ≈ 0.99005
    assert!((score - 0.99005).abs() < 0.001);
}

#[test]
fn test_score_decay_factor_69_hours() {
    let cfg = DecayConfig::new(0.01);
    let score = cfg.score(1.0, 69.0);
    // exp(-0.01 * 69) ≈ 0.5002 ≈ 50%
    assert!((score - 0.5002).abs() < 0.01);
}

#[test]
fn test_score_decay_factor_large_age() {
    let cfg = DecayConfig::new(0.01);
    let score = cfg.score(1.0, 1000.0);
    // exp(-0.01 * 1000) = exp(-10) ≈ 0.000045
    assert!(score < 0.001);
}

#[test]
fn test_score_higher_lambda_faster_decay() {
    let cfg_slow = DecayConfig::new(0.01);
    let cfg_fast = DecayConfig::new(0.1);
    let age = 10.0;
    let score_slow = cfg_slow.score(1.0, age);
    let score_fast = cfg_fast.score(1.0, age);
    assert!(score_slow > score_fast);
}

#[test]
fn test_age_hours() {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let ts = now - 7200; // 2 hours ago
    let age = DecayConfig::age_hours(ts);
    assert!((age - 2.0).abs() < 0.01);
}

#[test]
fn test_age_hours_future_timestamp() {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let age = DecayConfig::age_hours(now + 1000);
    assert!(
        age < 0.001,
        "future timestamp should yield ~0 age, got {age}"
    );
}

#[test]
fn test_refresh_sql_empty() {
    let sql = DecayConfig::refresh_sql(&[]);
    assert!(sql.is_empty());
}

#[test]
fn test_refresh_sql_single_id() {
    let sql = DecayConfig::refresh_sql(&[42]);
    assert_eq!(
        sql,
        "UPDATE chunks SET last_accessed_at = unixepoch() WHERE id IN (?1)"
    );
}

#[test]
fn test_refresh_sql_multiple_ids() {
    let sql = DecayConfig::refresh_sql(&[1, 2, 3]);
    assert_eq!(
        sql,
        "UPDATE chunks SET last_accessed_at = unixepoch() WHERE id IN (?1, ?2, ?3)"
    );
}

#[test]
fn test_age_hours_sql() {
    let sql = DecayConfig::age_hours_sql();
    assert_eq!(sql, "(unixepoch() - last_accessed_at) / 3600.0");
}

#[test]
fn test_score_preserves_zero_base() {
    let cfg = DecayConfig::new(0.01);
    let score = cfg.score(0.0, 100.0);
    assert!((score - 0.0).abs() < f32::EPSILON);
}

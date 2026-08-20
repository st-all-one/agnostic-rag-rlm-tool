#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_core::budget::{CostBudget, ErrorBudget, RunBudget, TimeBudget, TokenBudget};
use std::time::Duration;

#[test]
fn test_run_budget_new() {
    let budget = RunBudget::new(1.0, 100_000, 5, 60_000);
    let summary = budget.summary();
    assert!((summary.budget_remaining - 1.0).abs() < f64::EPSILON);
    assert_eq!(summary.tokens_remaining, 100_000);
    assert_eq!(summary.errors_remaining, 5);
    assert!(summary.time_remaining_ms > 0);
}

#[test]
fn test_run_budget_check_passes() {
    let budget = RunBudget::new(1.0, 100_000, 5, 60_000);
    assert!(budget.check().is_ok());
}

#[test]
fn test_run_budget_check_fails_on_timeout() {
    let budget = RunBudget::new(1.0, 100_000, 5, 0);
    std::thread::sleep(Duration::from_millis(10));
    assert!(budget.check().is_err());
}

#[test]
fn test_run_budget_record_call() {
    let budget = RunBudget::new(1.0, 100_000, 5, 60_000);
    let usage = arlm_llm::UsageSummary {
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
    };
    budget.record_call("gpt-4o", &usage);
    let summary = budget.summary();
    assert!(summary.tokens_remaining < 100_000);
}

#[test]
fn test_run_budget_record_error() {
    let budget = RunBudget::new(1.0, 100_000, 2, 60_000);
    budget.record_error();
    assert_eq!(budget.summary().errors_remaining, 1);
    budget.record_error();
    assert_eq!(budget.summary().errors_remaining, 0);
    assert!(budget.check().is_err());
}

#[test]
fn test_cost_budget_spend() {
    let cb = CostBudget::new(1.0);
    assert!((cb.remaining() - 1.0).abs() < f64::EPSILON);
    cb.spend(0.3);
    assert!((cb.remaining() - 0.7).abs() < 0.01);
}

#[test]
fn test_token_budget() {
    let tb = TokenBudget::new(1000);
    assert_eq!(tb.remaining(), 1000);
    tb.add(300);
    assert_eq!(tb.remaining(), 700);
    tb.add(800);
    assert_eq!(tb.remaining(), 0);
    assert!(tb.check().is_err());
}

#[test]
fn test_error_budget() {
    let eb = ErrorBudget::new(2);
    assert_eq!(eb.remaining(), 2);
    eb.add_one();
    assert_eq!(eb.remaining(), 1);
    eb.add_one();
    assert_eq!(eb.remaining(), 0);
    assert!(eb.check().is_err());
}

#[test]
fn test_time_budget_remaining() {
    let tb = TimeBudget::new(60_000);
    assert!(tb.remaining_ms() > 50_000);
}

#[test]
fn test_time_budget_expired() {
    let tb = TimeBudget::new(0);
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(tb.remaining_ms(), 0);
    assert!(tb.check().is_err());
}

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::manual_range_contains
)]

use arlm_core::token_counter::{TokenCounter, get_context_limit};

#[test]
fn test_estimate_empty() {
    assert_eq!(TokenCounter::estimate(""), 0);
}

#[test]
fn test_estimate_single_word() {
    let est = TokenCounter::estimate("hello");
    assert!(est >= 1 && est <= 2);
}

#[test]
fn test_estimate_multiple_words() {
    let est = TokenCounter::estimate("this is a test sentence");
    assert!(est >= 5 && est <= 10);
}

#[test]
fn test_record_and_total() {
    let tc = TokenCounter::new(1000);
    assert_eq!(tc.total_used(), 0);
    tc.record(100, 50);
    assert_eq!(tc.total_used(), 150);
    assert_eq!(tc.prompt_tokens(), 100);
    assert_eq!(tc.completion_tokens(), 50);
}

#[test]
fn test_remaining() {
    let tc = TokenCounter::new(1000);
    assert_eq!(tc.remaining(), 1000);
    tc.record(300, 200);
    assert_eq!(tc.remaining(), 500);
}

#[test]
fn test_is_within_budget() {
    let tc = TokenCounter::new(100);
    assert!(tc.is_within_budget());
    tc.record(50, 50);
    assert!(!tc.is_within_budget());
}

#[test]
fn test_check_budget_ok() {
    let tc = TokenCounter::new(100);
    tc.record(30, 20);
    assert!(tc.check_budget().is_ok());
}

#[test]
fn test_check_budget_exhausted() {
    let tc = TokenCounter::new(100);
    tc.record(50, 50);
    assert!(tc.check_budget().is_err());
}

#[test]
fn test_reset() {
    let tc = TokenCounter::new(1000);
    tc.record(100, 200);
    assert_eq!(tc.total_used(), 300);
    tc.reset();
    assert_eq!(tc.total_used(), 0);
    assert_eq!(tc.remaining(), 1000);
}

#[test]
fn test_multiple_records() {
    let tc = TokenCounter::new(1000);
    tc.record(100, 50);
    tc.record(200, 100);
    tc.record(50, 25);
    assert_eq!(tc.prompt_tokens(), 350);
    assert_eq!(tc.completion_tokens(), 175);
    assert_eq!(tc.total_used(), 525);
    assert_eq!(tc.remaining(), 475);
}

#[test]
fn test_saturating_sub() {
    let tc = TokenCounter::new(100);
    tc.record(80, 50);
    assert_eq!(tc.remaining(), 0);
}

#[test]
fn test_max_tokens_accessor() {
    let tc = TokenCounter::new(500);
    assert_eq!(tc.max_tokens(), 500);
}

#[test]
fn test_estimate_long_text() {
    let text = "The quick brown fox jumps over the lazy dog. \
                 This is a longer sentence with many words to estimate tokens.";
    let est = TokenCounter::estimate(text);
    assert!(est > 20);
}

#[test]
fn test_get_context_limit_gpt4o() {
    assert_eq!(get_context_limit("gpt-4o"), 128_000);
    assert_eq!(get_context_limit("gpt-4o-mini"), 128_000);
}

#[test]
fn test_get_context_limit_claude() {
    assert_eq!(get_context_limit("claude-4-sonnet"), 1_000_000);
    assert_eq!(get_context_limit("claude-3.5-sonnet"), 200_000);
    assert_eq!(get_context_limit("claude-3-opus"), 200_000);
}

#[test]
fn test_get_context_limit_gemini() {
    assert_eq!(get_context_limit("gemini-2.5-pro"), 1_000_000);
    assert_eq!(get_context_limit("gemini-1.5-pro"), 2_000_000);
}

#[test]
fn test_get_context_limit_deepseek() {
    assert_eq!(get_context_limit("deepseek-v3"), 131_072);
    assert_eq!(get_context_limit("deepseek-r1"), 131_072);
}

#[test]
fn test_get_context_limit_mimo() {
    assert_eq!(get_context_limit("mimo"), 131_072);
}

#[test]
fn test_get_context_limit_unknown_fallback() {
    assert_eq!(get_context_limit("some-unknown-model"), 128_000);
}

#[test]
fn test_get_context_limit_longest_match() {
    assert_eq!(get_context_limit("gpt-4o-2024-08-06"), 128_000);
}

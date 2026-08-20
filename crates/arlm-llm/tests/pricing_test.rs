#![allow(
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

use arlm_llm::types::UsageSummary;
use arlm_llm::{ModelPricing, PricingTable, pricing::estimate_default};

#[test]
fn test_model_pricing_cost() {
    let pricing = ModelPricing::new(10.0, 30.0);
    let usage = UsageSummary {
        prompt_tokens: 1_000_000,
        completion_tokens: 1_000_000,
        total_tokens: 2_000_000,
        cost_usd: 0.0,
    };
    let cost = pricing.cost_usd(&usage);
    assert!((cost - 40.0).abs() < f64::EPSILON);
}

#[test]
fn test_model_pricing_cost_partial() {
    let pricing = ModelPricing::new(10.0, 30.0);
    let usage = UsageSummary {
        prompt_tokens: 100_000,
        completion_tokens: 50_000,
        total_tokens: 150_000,
        cost_usd: 0.0,
    };
    let cost = pricing.cost_usd(&usage);
    assert!((cost - 2.5).abs() < f64::EPSILON);
}

#[test]
fn test_pricing_table_default() {
    let table = PricingTable::default();
    assert!(table.get("gpt-4o").is_some());
    assert!(table.get("claude-sonnet-4-20250514").is_some());
    assert!(table.get("unknown-model").is_none());
}

#[test]
fn test_pricing_table_register() {
    let mut table = PricingTable::default();
    table.register("custom-model".to_string(), ModelPricing::new(5.0, 10.0));
    assert!(table.get("custom-model").is_some());
}

#[test]
fn test_pricing_table_estimate_cost() {
    let table = PricingTable::default();
    let usage = UsageSummary {
        prompt_tokens: 1_000_000,
        completion_tokens: 1_000_000,
        total_tokens: 2_000_000,
        cost_usd: 0.0,
    };
    let cost = table.estimate_cost("gpt-4o", &usage);
    assert!((cost - 12.5).abs() < f64::EPSILON);
}

#[test]
fn test_pricing_table_unknown_model() {
    let table = PricingTable::default();
    let usage = UsageSummary {
        prompt_tokens: 1_000_000,
        completion_tokens: 1_000_000,
        total_tokens: 2_000_000,
        cost_usd: 0.0,
    };
    let cost = table.estimate_cost("unknown-model", &usage);
    assert!((cost - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_ollama_zero_cost() {
    let table = PricingTable::default();
    let usage = UsageSummary {
        prompt_tokens: 1_000_000,
        completion_tokens: 1_000_000,
        total_tokens: 2_000_000,
        cost_usd: 0.0,
    };
    let cost = table.estimate_cost("llama3", &usage);
    assert!((cost - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_estimate_default_matches_table() {
    let usage = UsageSummary {
        prompt_tokens: 1_000_000,
        completion_tokens: 1_000_000,
        total_tokens: 2_000_000,
        cost_usd: 0.0,
    };
    let table_cost = PricingTable::default().estimate_cost("gpt-4o", &usage);
    let default_cost = estimate_default("gpt-4o", &usage);
    assert!((table_cost - default_cost).abs() < f64::EPSILON);
}

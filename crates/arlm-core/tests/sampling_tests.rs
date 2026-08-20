#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(clippy::float_cmp)]

use arlm_core::sampling::SamplingArgs;
use arlm_core::types::Action;
use arlm_llm::{CompletionRequest, Message, Role};

#[test]
fn test_for_solve_action() {
    let args = SamplingArgs::for_node_type(Action::Solve);
    assert!((args.temperature - 0.3).abs() < f32::EPSILON);
    assert!((args.top_p - 0.9).abs() < f32::EPSILON);
    assert!(args.top_k.is_none());
}

#[test]
fn test_for_decompose_action() {
    let args = SamplingArgs::for_node_type(Action::Decompose);
    assert!((args.temperature - 0.1).abs() < f32::EPSILON);
    assert!((args.top_p - 0.85).abs() < f32::EPSILON);
    assert!(args.top_k.is_none());
}

#[test]
fn test_with_seed() {
    let args = SamplingArgs::for_node_type(Action::Solve).with_seed(42);
    assert_eq!(args.seed(), Some(42));
}

#[test]
fn test_apply_to_request_sets_temperature() {
    let args = SamplingArgs {
        temperature: 0.5,
        top_p: 0.9,
        top_k: Some(40),
        seed: None,
    };
    let req = CompletionRequest {
        model: "test".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: "hi".to_string(),
        }],
        temperature: None,
        max_tokens: None,
        stop: None,
    };
    let updated = args.apply_to_request(req);
    assert!((updated.temperature.unwrap() - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_apply_to_request_preserves_existing_temperature() {
    let args = SamplingArgs {
        temperature: 0.5,
        top_p: 0.9,
        top_k: None,
        seed: None,
    };
    let req = CompletionRequest {
        model: "test".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: "hi".to_string(),
        }],
        temperature: Some(0.9),
        max_tokens: None,
        stop: None,
    };
    let updated = args.apply_to_request(req);
    assert!((updated.temperature.unwrap() - 0.9).abs() < f32::EPSILON);
}

#[test]
fn test_serialization_roundtrip() {
    let args = SamplingArgs::for_node_type(Action::Decompose);
    let json = serde_json::to_string(&args).unwrap();
    let deserialized: SamplingArgs = serde_json::from_str(&json).unwrap();
    assert_eq!(args.temperature, deserialized.temperature);
    assert_eq!(args.top_p, deserialized.top_p);
}

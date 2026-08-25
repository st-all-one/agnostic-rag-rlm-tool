//! Unit tests for the volunteer synthesis pipeline (pure parts).
//!
//! The gRPC loop itself needs a live server; here we cover the deterministic
//! pieces: prompt selection per level, request construction, payload
//! validation and the short-summary rejection gate.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn system_prompt_varies_by_level() {
    assert_eq!(system_prompt_for(1), SYSTEM_L1);
    assert_eq!(system_prompt_for(2), SYSTEM_L2);
    // Anything above 3 falls back to the project-level template.
    assert_eq!(system_prompt_for(3), SYSTEM_L3);
    assert_eq!(system_prompt_for(99), SYSTEM_L3);
}

#[test]
fn build_request_shapes_messages_and_sampling() {
    let payload = RlmJobPayload {
        texts: vec!["alpha".into(), "beta".into()],
        subject_kind: "file".into(),
        ..RlmJobPayload::default()
    };
    let req = build_request(1, "src/main.rs", &payload, 512);

    assert!(req.model.is_empty(), "model resolved later by caller");
    assert_eq!(req.messages.len(), 2);
    assert_eq!(req.messages[0].role, arags_llm::types::Role::System);
    assert_eq!(req.messages[0].content, SYSTEM_L1);
    assert_eq!(req.messages[1].role, arags_llm::types::Role::User);
    // User body carries subject, kind and every numbered input.
    let body = &req.messages[1].content;
    assert!(body.contains("Subject: src/main.rs"));
    assert!(body.contains("Kind: file"));
    assert!(body.contains("--- input 1 ---"));
    assert!(body.contains("alpha"));
    assert!(body.contains("--- input 2 ---"));
    assert!(body.contains("beta"));
    assert_eq!(req.temperature, Some(0.2));
    assert_eq!(req.max_tokens, Some(512));
}

#[test]
fn parse_inputs_rejects_empty_and_malformed() {
    assert!(parse_inputs("not json", 7).is_err());
    // Valid JSON but no inputs at all.
    let empty = serde_json::to_string(&RlmJobPayload::default()).unwrap();
    assert!(parse_inputs(&empty, 7).is_err());
}

#[test]
fn parse_inputs_accepts_hashes_only_or_texts_only() {
    let hashes = r#"{"hashes":["h1"]}"#;
    assert!(parse_inputs(hashes, 1).is_ok());
    let texts = r#"{"texts":["t1","t2"]}"#;
    let p = parse_inputs(texts, 2).unwrap();
    assert_eq!(p.texts.len(), 2);
}

#[test]
fn summary_gate_refuses_short_or_blank_output() {
    assert!(!summary_acceptable(""));
    assert!(!summary_acceptable("   \n\t"));
    assert!(!summary_acceptable("too short"));
    // Exactly at the threshold counts as acceptable.
    let ok = "a".repeat(MIN_SUMMARY_CHARS);
    assert!(summary_acceptable(&ok));
    assert!(summary_acceptable(&format!("{ok} more")));
}

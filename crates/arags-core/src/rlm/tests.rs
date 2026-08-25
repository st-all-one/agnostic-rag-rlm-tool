use super::*;

#[test]
fn payload_round_trips_and_tolerates_missing_fields() {
    let full = RlmJobPayload {
        chunk_ids: vec![1, 2],
        hashes: vec!["h".into()],
        texts: vec!["t".into()],
        template_version: "v1".into(),
        subject_kind: "file".into(),
        ..RlmJobPayload::default()
    };
    let json = serde_json::to_string(&full).expect("serialize");
    let back: RlmJobPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.hashes, vec!["h".to_string()]);
    assert!(back.node_ids.is_empty());

    // Legacy/partial payload: every field optional.
    let partial: RlmJobPayload = serde_json::from_str(r#"{"hashes":["x"]}"#).expect("partial");
    assert_eq!(partial.hashes, vec!["x".to_string()]);
    assert_eq!(partial.template_version, "");
    assert_eq!(partial.subject_kind, "");
}

#[test]
fn empty_vectors_are_skipped_when_serializing() {
    let json = serde_json::to_string(&RlmJobPayload::default()).expect("serialize");
    assert_eq!(json, r#"{"template_version":"","subject_kind":""}"#);
}

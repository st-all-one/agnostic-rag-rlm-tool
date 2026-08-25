use super::*;

#[test]
fn payload_round_trips_and_tolerates_missing_fields() {
    let full = ExplorationPayload {
        goal: "anexos compartilhados".into(),
        summary: "resumo".into(),
        body_markdown: "# Mapa".into(),
        files: vec!["src/a.rs".into()],
        created_by: "agent-1".into(),
        model: "qwen2.5:7b".into(),
        template_version: TEMPLATE_VERSION_V1.into(),
    };
    let json = serde_json::to_string(&full).expect("serialize");
    let back: ExplorationPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.goal, full.goal);
    assert_eq!(back.files, vec!["src/a.rs".to_string()]);

    // Legacy/partial payload: every field optional.
    let partial: ExplorationPayload = serde_json::from_str(r#"{"goal":"x"}"#).expect("partial");
    assert!(partial.files.is_empty());
    assert_eq!(partial.template_version, "");
}

#[test]
fn payload_skips_empty_vectors_when_serializing() {
    let json = serde_json::to_string(&ExplorationPayload::default()).expect("serialize");
    assert!(
        !json.contains("files"),
        "empty vectors must be omitted: {json}"
    );
}

#[test]
fn status_and_role_constants_are_stable_wire_values() {
    assert_eq!(STATUS_FRESH, "fresh");
    assert_eq!(STATUS_STALE, "stale");
    assert_eq!(STATUS_RETIRED, "retired");
    assert_eq!(ROLE_CITED, "cited");
    assert_eq!(ROLE_CONTEXT, "context");
    assert_eq!(TEMPLATE_VERSION_V1, "v1");
}

#[test]
fn classify_applies_double_thresholds() {
    let cfg = ConfidenceConfig::default();
    assert_eq!(cfg.classify(0.95), HitClass::Strong);
    assert_eq!(cfg.classify(cfg.hit_high), HitClass::Strong);
    assert_eq!(
        cfg.classify(f32::midpoint(cfg.hit_high, cfg.hit_low)),
        HitClass::Related
    );
    assert_eq!(cfg.classify(cfg.hit_low), HitClass::Related);
    assert_eq!(cfg.classify(0.0), HitClass::None);
}

#[test]
fn score_is_zero_for_irrelevant_similarity_even_with_feedback() {
    let cfg = ConfidenceConfig::default();
    let score = confidence_score(0.0, 0, 0, 100, 0, &cfg);
    // feedback_weight * 1.0 is the only surviving term; must stay tiny.
    assert!(score <= cfg.feedback_weight + 1e-6, "score = {score}");
}

#[test]
fn balanced_feedback_is_a_noop_but_positive_feedback_boosts() {
    let cfg = ConfidenceConfig::default();
    let base = confidence_score(0.8, 0, 0, 0, 0, &cfg);
    let balanced = confidence_score(0.8, 0, 0, 5, 5, &cfg);
    assert!((base - balanced).abs() < 1e-6);

    let boosted = confidence_score(0.8, 0, 0, 10, 0, &cfg);
    assert!(boosted > base, "confirms must raise the score");
    let busted = confidence_score(0.8, 0, 0, 0, 10, &cfg);
    assert!(busted < base, "contradictions must lower the score");
}

#[test]
fn drift_and_age_decay_respect_floors() {
    let cfg = ConfidenceConfig::default();
    // Huge drift/age saturates at floors instead of zeroing the similarity.
    let floor_product = (cfg.drift_floor * cfg.age_floor).min(1.0);
    let saturated = confidence_score(0.9, u32::MAX, u32::MAX, 0, 0, &cfg);
    assert!(
        saturated >= 0.9 * floor_product - 1e-6,
        "score = {saturated}"
    );
}

#[test]
fn non_finite_similarity_falls_back_to_zero() {
    let cfg = ConfidenceConfig::default();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let score = confidence_score(bad, 0, 0, 0, 0, &cfg);
        assert!(
            (0.0..=1.0).contains(&score),
            "NaN/inf must clamp, got {score}"
        );
    }
}

#[test]
fn claim_text_prefers_conexoes_then_mapa_then_whole_body() {
    assert!(claim_text("").is_empty());
    assert_eq!(claim_text("só texto"), "só texto");

    let doc = "## Mapa\nresumo geral\n\n## Conexões\n- a -> b: via storage\n\n## Evidências\nx";
    assert_eq!(claim_text(doc), "- a -> b: via storage");

    let no_conex = "## Mapa\ndescrição do mapa\n\n## Evidências\ny";
    assert_eq!(claim_text(no_conex), "descrição do mapa");

    // Section header present but empty falls through to the next candidate.
    let empty_conex = "## Conexões\n\n## Mapa\nfallback";
    assert_eq!(claim_text(empty_conex), "fallback");
}

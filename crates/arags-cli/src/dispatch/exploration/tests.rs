//! Unit tests for the exploration contract parser and hit rendering
//! (plan 022). The parser is the local gate before any network call, so its
//! error messages must be precise.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::parse_contract;
use super::render_hits;
use crate::output::Format;
use arags_proto::proto::ExplorationHit;

const VALID: &str = "---\ngoal: anexos compartilhados\nfiles: src/a.rs, src/b.rs\nmodel: qwen2.5:7b\n---\n\n## Mapa\nO bucket é compartilhado.\n\n## Conexões\n- a -> b\n\n## Evidências\n- a.rs:88\n\n## Limitações\njob noturno não verificado.\n";

#[test]
fn parse_contract_happy_path_with_explicit_summary() {
    let doc = VALID.replace(
        "goal: anexos compartilhados",
        "goal: anexos compartilhados\nsummary: resumo curto",
    );
    let c = parse_contract(&doc).expect("valid contract");
    assert_eq!(c.goal, "anexos compartilhados");
    assert_eq!(c.summary, "resumo curto");
    assert_eq!(
        c.files,
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
    );
    assert_eq!(c.model, "qwen2.5:7b");
    assert!(c.body_markdown.contains("## Conexões"));
}

#[test]
fn parse_contract_derives_summary_from_mapa_first_paragraph() {
    let c = parse_contract(VALID).expect("valid contract");
    assert_eq!(c.summary, "O bucket é compartilhado.");
}

#[test]
fn parse_contract_rejects_missing_pieces() {
    for (doc, expected) in [
        ("", "empty"),
        ("## Mapa\nx\n", "'---'"),
        ("---\ngoal: x\n---\nbody", "files:"),
        ("---\nfiles: a.rs\n---\nbody", "goal:"),
        ("---\ngoal: x\nfiles: a.rs\n---\n## Mapa\ny\n", "Conexões"),
    ] {
        let err = parse_contract(doc).unwrap_err();
        assert!(err.contains(expected), "doc {doc:?} → {err}");
    }
}

#[test]
fn parse_contract_tolerates_unknown_header_keys_and_spaces() {
    let doc = "---\ntitle: x\ngoal:   g   \nfiles:  a.rs ,b.rs \n---\n## Mapa\nm\n## Conexões\nc\n## Evidências\ne\n## Limitações\nl";
    let c = parse_contract(doc).expect("unknown keys ignored");
    assert_eq!(c.goal, "g");
    assert_eq!(c.files.len(), 2);
}

#[test]
fn render_hits_text_and_json_shapes() {
    let hits = vec![ExplorationHit {
        exploration_id: "e1".into(),
        goal: "g".into(),
        summary: "s".into(),
        confidence: 0.9,
        similarity: 0.95,
        status: "stale".into(),
        stale_reason: vec!["src/a.rs".into()],
        confirmed: 2,
        contradicted: 0,
        created_by: "agent".into(),
        model: "m".into(),
        ..ExplorationHit::default()
    }];

    let text = render_hits(&hits, "q", Format::Markdown);
    assert!(text.contains("# g [stale]"));
    assert!(text.contains("confidence=0.90"));
    assert!(text.contains("stale: src/a.rs"));
    assert!(text.contains("id: e1"));

    let json = render_hits(&hits, "q", Format::FullJson);
    assert!(json.contains("\"exploration_id\": \"e1\""));

    assert_eq!(
        render_hits(&[], "q", Format::Markdown),
        "no exploration maps for \"q\"\n"
    );
}

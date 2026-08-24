#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arags_memory::persist::*;
use tempfile::TempDir;

fn setup() -> (PersistEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = PersistEngine::new(tmp.path()).unwrap();
    (engine, tmp)
}

#[test]
fn test_new_creates_wiki_dirs() {
    let tmp = TempDir::new().unwrap();
    let engine = PersistEngine::new(tmp.path()).unwrap();

    assert!(engine.wiki_root().is_dir());
    assert!(engine.wiki_root().join("searches").is_dir());
    assert!(engine.wiki_root().join("analyses").is_dir());
    assert!(engine.wiki_root().join("decisions").is_dir());
    assert!(engine.wiki_root().join("sessions").is_dir());
    assert!(engine.wiki_root().join("trajectories").is_dir());
    assert!(engine.wiki_root().join("_global").is_dir());
}

#[test]
fn test_persist_search() {
    let (engine, _tmp) = setup();

    let result = engine
        .persist_search(&SearchPersistOptions {
            query: "bug de login".to_string(),
            tier: "entity".to_string(),
            project: "my-project".to_string(),
            entities: vec!["jwt".to_string(), "session".to_string()],
            tags: vec!["auth".to_string()],
            body: "## Resultado\n\nSome content here.".to_string(),
        })
        .unwrap();

    assert!(result.path.exists());
    assert!(result.relative_path.starts_with("searches/"));

    let content = std::fs::read_to_string(&result.path).unwrap();
    assert!(content.starts_with("---\n"));
    assert!(content.contains("title:"));
    assert!(content.contains("query: bug de login"));
    assert!(content.contains("tier: entity"));
    assert!(content.contains("project: my-project"));
    assert!(content.contains("entities:"));
    assert!(content.contains("- jwt"));
    assert!(content.contains("- session"));
    assert!(content.contains("## Resultado"));
}

#[test]
fn test_persist_analysis() {
    let (engine, _tmp) = setup();

    let result = engine
        .persist_analysis(&AnalysisPersistOptions {
            title: "Auth Architecture".to_string(),
            project: "proj".to_string(),
            tags: vec!["architecture".to_string()],
            body: "# Analysis\n\nDetailed analysis.".to_string(),
        })
        .unwrap();

    assert!(result.path.exists());
    assert!(result.relative_path.starts_with("analyses/"));
    assert!(result.relative_path.contains("001-auth-architecture"));

    let content = std::fs::read_to_string(&result.path).unwrap();
    assert!(content.contains("title: Auth Architecture"));
}

#[test]
fn test_persist_analysis_auto_sequence() {
    let (engine, _tmp) = setup();

    let r1 = engine
        .persist_analysis(&AnalysisPersistOptions {
            title: "First".to_string(),
            project: "proj".to_string(),
            tags: Vec::new(),
            body: "body".to_string(),
        })
        .unwrap();

    let r2 = engine
        .persist_analysis(&AnalysisPersistOptions {
            title: "Second".to_string(),
            project: "proj".to_string(),
            tags: Vec::new(),
            body: "body".to_string(),
        })
        .unwrap();

    assert!(r1.relative_path.contains("001-"));
    assert!(r2.relative_path.contains("002-"));
}

#[test]
fn test_persist_decision() {
    let (engine, _tmp) = setup();

    let result = engine
        .persist_decision(&DecisionPersistOptions {
            title: "Use Postgres".to_string(),
            tags: vec!["database".to_string()],
            body: "# Decision\n\nWe chose Postgres.".to_string(),
            supersedes: None,
        })
        .unwrap();

    assert!(result.path.exists());
    assert!(result.relative_path.starts_with("decisions/"));
    assert!(result.relative_path.contains("001-use-postgres"));

    let content = std::fs::read_to_string(&result.path).unwrap();
    assert!(content.contains("title: Use Postgres"));
}

#[test]
fn test_persist_decision_with_supersedes() {
    let (engine, _tmp) = setup();

    let result = engine
        .persist_decision(&DecisionPersistOptions {
            title: "Use Postgres v2".to_string(),
            tags: Vec::new(),
            body: "Updated decision.".to_string(),
            supersedes: Some("decisions/001-use-postgres.md".to_string()),
        })
        .unwrap();

    let content = std::fs::read_to_string(&result.path).unwrap();
    assert!(content.contains("supersedes: decisions/001-use-postgres.md"));
}

#[test]
fn test_persist_session() {
    let (engine, _tmp) = setup();

    let result = engine
        .persist_session(&SessionPersistOptions {
            session_id: "s_abc123".to_string(),
            project: "proj".to_string(),
            body: "# Session\n\nSession content.".to_string(),
        })
        .unwrap();

    assert!(result.path.exists());
    assert!(result.relative_path.contains("sessions/s_abc123.md"));
}

#[test]
fn test_persist_trajectory() {
    let (engine, _tmp) = setup();

    let result = engine
        .persist_trajectory(&TrajectoryPersistOptions {
            run_id: "run_xyz".to_string(),
            project: "proj".to_string(),
            body: "# Run\n\nTrajectory content.".to_string(),
        })
        .unwrap();

    assert!(result.path.exists());
    assert!(result.relative_path.contains("trajectories/run_xyz.md"));
}

#[test]
fn test_persist_raw() {
    let (engine, _tmp) = setup();

    let result = engine
        .persist_raw("rules/no-unwrap.md", "# Rule\n\nNo unwrap allowed.")
        .unwrap();

    assert!(result.path.exists());
    assert_eq!(result.relative_path, "rules/no-unwrap.md");

    let content = std::fs::read_to_string(&result.path).unwrap();
    assert_eq!(content, "# Rule\n\nNo unwrap allowed.");
}

#[test]
fn test_read_page() {
    let (engine, _tmp) = setup();

    let content = "---\ntitle: Hello\ncreated: '2024-01-15T10:30:00Z'\nupdated: '2024-01-15T10:30:00Z'\nsalience: 1.0\n---\n\nWorld.";
    engine.persist_raw("test.md", content).unwrap();

    let (fm, body) = engine.read_page("test.md").unwrap();
    assert_eq!(fm.title, "Hello");
    assert_eq!(body, "World.");
}

#[test]
fn test_list_pages_empty() {
    let (engine, _tmp) = setup();
    let pages = engine.list_pages(WikiScope::Analyses).unwrap();
    assert!(pages.is_empty());
}

#[test]
fn test_list_pages() {
    let (engine, _tmp) = setup();

    engine
        .persist_analysis(&AnalysisPersistOptions {
            title: "A".to_string(),
            project: "proj".to_string(),
            tags: Vec::new(),
            body: "body".to_string(),
        })
        .unwrap();

    engine
        .persist_analysis(&AnalysisPersistOptions {
            title: "B".to_string(),
            project: "proj".to_string(),
            tags: Vec::new(),
            body: "body".to_string(),
        })
        .unwrap();

    let pages = engine.list_pages(WikiScope::Analyses).unwrap();
    assert_eq!(pages.len(), 2);
    assert!(pages[0].contains("001-a"));
    assert!(pages[1].contains("002-b"));
}

#[test]
fn test_sanitize_slug() {
    assert_eq!(sanitize_slug("Hello World"), "hello-world");
    assert_eq!(sanitize_slug("bug de login"), "bug-de-login");
    assert_eq!(sanitize_slug("  spaces  "), "spaces");
    assert_eq!(sanitize_slug("special/chars\\here"), "special-chars-here");
    assert_eq!(sanitize_slug("already-ok"), "already-ok");
    assert_eq!(sanitize_slug("UPPER"), "upper");
    assert_eq!(sanitize_slug("a__b--c"), "a-b-c");
    assert_eq!(sanitize_slug(""), "");
}

#[test]
fn test_frontmatter_roundtrip() {
    let fm = Frontmatter {
        title: "Test Page".to_string(),
        created: "2024-01-15T10:30:00Z".to_string(),
        updated: "2024-01-15T10:30:00Z".to_string(),
        query: Some("test query".to_string()),
        tier: Some("entity".to_string()),
        project: Some("proj".to_string()),
        entities: vec!["e1".to_string()],
        tags: vec!["t1".to_string()],
        pinned: true,
        expires_at: None,
        salience: 0.8,
        access_count: 5,
        supersedes: Some("old/path.md".to_string()),
    };

    let rendered = render_markdown(&fm, "body content");
    assert!(rendered.starts_with("---\n"));

    let (parsed_fm, body) = parse_markdown(&rendered).unwrap();
    assert_eq!(parsed_fm.title, "Test Page");
    assert_eq!(parsed_fm.query, Some("test query".to_string()));
    assert!(parsed_fm.pinned);
    assert_eq!(parsed_fm.salience, 0.8);
    assert_eq!(parsed_fm.access_count, 5);
    assert_eq!(parsed_fm.supersedes, Some("old/path.md".to_string()));
    assert_eq!(body, "body content");
}

#[test]
fn test_pinned_survives_in_frontmatter() {
    let (engine, _tmp) = setup();

    engine
        .persist_decision(&DecisionPersistOptions {
            title: "Important".to_string(),
            tags: Vec::new(),
            body: "Body".to_string(),
            supersedes: None,
        })
        .unwrap();

    let pages = engine.list_pages(WikiScope::Decisions).unwrap();
    let (fm, _) = engine.read_page(&pages[0]).unwrap();
    assert!(!fm.pinned);
    assert_eq!(fm.salience, 1.0);
}

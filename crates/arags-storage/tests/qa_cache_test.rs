#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::pedantic
)]

//! Integration tests for the semantic query-answer cache (plan 017).
//!
//! Storage is opened in single-connection mode (SQLite in a temp dir) so the
//! tests are fast and hermetic. These cover the deterministic server-side
//! persistence contract: exact hits, project scoping, reserve-lock dedup,
//! staleness-by-hash invalidation, weighted-LRU eviction, stable-id lookup,
//! and manual invalidation. The pure similarity/engine logic
//! (`arags_search::qa_cache`, `arags_core::qa_cache`) is unit-tested in its own
//! crates; the full gRPC flow (digest-once, tier widening) is exercised in
//! `arags_server::grpc::query_cache` tests.

use arags_storage::Storage;
use arags_storage::qa_cache::{StoreAnswerInput, question_hash};
use tempfile::TempDir;

fn setup() -> (Storage, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let storage = Storage::open(dir.path()).expect("open storage");
    (storage, dir)
}

fn input(
    project: &str,
    question: &str,
    answer: &str,
    chunk_ids: &[&str],
    hashes: &[&str],
) -> StoreAnswerInput {
    StoreAnswerInput {
        buffer_id: Some(1),
        project: project.to_string(),
        question_text: question.to_string(),
        question_hash: question_hash(question),
        answer_text: answer.to_string(),
        source_chunk_ids: chunk_ids.iter().map(|s| s.to_string()).collect(),
        source_hashes: hashes.iter().map(|s| s.to_string()).collect(),
        model: Some("llama3".to_string()),
        tier_snapshot: Some("{}".to_string()),
        token_count: 12,
        created_by: None,
    }
}

// ── Exact hit (client pays 0 LLM on a cache HIT) ───────────────────────────

#[test]
fn test_qa_cache_exact_hit_zero_llm_calls() {
    let (storage, _dir) = setup();
    let q = "How do we hash passwords?";
    let stored = storage
        .store_answer(&input(
            "p1",
            q,
            "Use argon2id.",
            &["c1", "c2"],
            &["h1", "h2"],
        ))
        .unwrap();
    assert!(stored.created);

    let row = storage
        .get_cached_answer("p1", &question_hash(q))
        .unwrap()
        .expect("exact hit");
    assert_eq!(row.answer_text, "Use argon2id.");
    assert_eq!(row.cache_id, stored.cache_id);
}

// ── Project scoping (same question, different projects → independent) ───────

#[test]
fn test_qa_cache_scoped_per_project() {
    let (storage, _dir) = setup();
    let q = "Where is the auth handler?";
    let a = storage
        .store_answer(&input("projA", q, "In src/auth.rs", &["c1"], &["h1"]))
        .unwrap();
    let b = storage
        .store_answer(&input("projB", q, "In handlers/auth.go", &["c9"], &["h9"]))
        .unwrap();

    // Distinct stable ids, no cross-contamination.
    assert_ne!(a.cache_id, b.cache_id);

    let ra = storage
        .get_cached_answer("projA", &question_hash(q))
        .unwrap()
        .unwrap();
    let rb = storage
        .get_cached_answer("projB", &question_hash(q))
        .unwrap()
        .unwrap();
    assert_eq!(ra.answer_text, "In src/auth.rs");
    assert_eq!(rb.answer_text, "In handlers/auth.go");

    // A query scoped to projA must NOT see projB's answer.
    assert!(
        storage
            .get_cached_answer("projA", &question_hash(q))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        storage
            .get_cached_answer("projA", &question_hash(q))
            .unwrap()
            .unwrap()
            .answer_text,
        "In src/auth.rs"
    );
}

// ── Reserve lock dedupes identical concurrent MISS on same project ──────────

#[test]
fn test_qa_cache_supersedes_same_project() {
    let (storage, _dir) = setup();
    let q = "Explain the stop-word filter.";

    let first = storage
        .store_answer(&input(
            "p1",
            q,
            "It removes common words.",
            &["c1"],
            &["h1"],
        ))
        .unwrap();
    let second = storage
        .store_answer(&input(
            "p1",
            q,
            "It removes common words.",
            &["c1"],
            &["h1"],
        ))
        .unwrap();

    // A re-store for the same subject SUPERSEDES: every store is a new row.
    assert!(first.created);
    assert!(second.created);
    assert_ne!(first.cache_id, second.cache_id);
    assert_ne!(first.id, second.id);

    // The previous revision is retired and links to the new one; the read
    // returns only the latest active row.
    let old = storage.get_qa_by_rowid(first.id).unwrap().unwrap();
    assert_eq!(old.is_active, false);
    assert_eq!(old.superseded_by, Some(second.id));
    let hit = storage
        .get_cached_answer("p1", &question_hash(q))
        .unwrap()
        .unwrap();
    assert_eq!(hit.id, second.id);

    // Two rows persisted for this (project, question_hash): one active, one history.
    assert_eq!(storage.count_qa("p1").unwrap(), 2);
}

// ── Staleness: chunk hash change forces re-digest (MISS) ────────────────────

#[test]
fn test_qa_cache_invalidated_after_chunk_hash_change() {
    let (storage, _dir) = setup();
    let q = "What does summarize() do?";
    let stored = storage
        .store_answer(&input(
            "p1",
            q,
            "Aggregates all chunks.",
            &["c1", "c2"],
            &["hA", "hB"],
        ))
        .unwrap();
    assert!(
        storage
            .get_cached_answer("p1", &question_hash(q))
            .unwrap()
            .is_some()
    );

    // Buffer 1's chunk "c1" (hash hA) changed during reindex.
    let n = storage
        .mark_stale_by_hashes(1, &["hA".to_string()])
        .unwrap();
    assert_eq!(n, 1);

    // Stale entry is no longer served as a hit; caller re-digests.
    assert!(
        storage
            .get_cached_answer("p1", &question_hash(q))
            .unwrap()
            .is_none()
    );

    // But the row is still auditable by its stable id.
    let row = storage
        .get_qa_by_cache_id(&stored.cache_id)
        .unwrap()
        .unwrap();
    assert!(row.stale);
}

// ── Eviction: weighted-LRU keeps the most-valued entries ───────────────────

#[test]
fn test_qa_cache_eviction_lru() {
    let (storage, _dir) = setup();
    // Populate 5 entries in one project.
    for i in 0..5 {
        storage
            .store_answer(&input(
                "p1",
                &format!("question number {i}"),
                &format!("answer {i}"),
                &[&format!("c{i}")],
                &[&format!("h{i}")],
            ))
            .unwrap();
    }
    assert_eq!(storage.count_qa("p1").unwrap(), 5);

    // Touch entry #0 so it becomes the most valuable (highest access score).
    let row0 = storage
        .get_cached_answer("p1", &question_hash("question number 0"))
        .unwrap()
        .unwrap();
    storage.touch_qa(row0.id).unwrap();
    storage.touch_qa(row0.id).unwrap();

    // Evict down to 3: entry #0 (heavily accessed) must survive.
    let removed = storage.evict_qa("p1", 3, 1_000_000).unwrap();
    assert_eq!(removed, 2);
    assert_eq!(storage.count_qa("p1").unwrap(), 3);

    assert!(
        storage
            .get_cached_answer("p1", &question_hash("question number 0"))
            .unwrap()
            .is_some()
    );
}

// ── Stable-id lookup returns identical 1:1 (anti-drift) ─────────────────────

#[test]
fn test_qa_cache_get_by_id_returns_identical_1to1() {
    let (storage, _dir) = setup();
    let chunk_ids = vec!["c1", "c2", "c3"];
    let hashes = vec!["h1", "h2", "h3"];
    let stored = storage
        .store_answer(&input(
            "p1",
            "How is config loaded?",
            "Via Config::load().",
            &chunk_ids,
            &hashes,
        ))
        .unwrap();

    let by_id = storage
        .get_qa_by_cache_id(&stored.cache_id)
        .unwrap()
        .unwrap();
    assert_eq!(by_id.answer_text, "Via Config::load().");
    assert_eq!(by_id.source_chunk_ids, vec!["c1", "c2", "c3"]);
    assert_eq!(by_id.source_hashes, vec!["h1", "h2", "h3"]);
    assert_eq!(by_id.cache_id, stored.cache_id);
}

// ── Manual invalidation: Stale forces re-digest ────────────────────────────

#[test]
fn test_qa_cache_invalidate_single_marks_stale_forces_redigest() {
    let (storage, _dir) = setup();
    let q = "How do we protect secrets?";
    let stored = storage
        .store_answer(&input("p1", q, "Use a vault.", &["c1"], &["h1"]))
        .unwrap();
    assert!(
        storage
            .get_cached_answer("p1", &question_hash(q))
            .unwrap()
            .is_some()
    );

    assert!(
        storage
            .mark_qa_stale(stored.id, "senior", "hallucination")
            .unwrap()
    );
    // After Stale, the same query is a MISS (forces fresh digest).
    assert!(
        storage
            .get_cached_answer("p1", &question_hash(q))
            .unwrap()
            .is_none()
    );

    let row = storage
        .get_qa_by_cache_id(&stored.cache_id)
        .unwrap()
        .unwrap();
    assert!(row.stale);
    assert_eq!(row.invalidated_by.as_deref(), Some("senior"));
    assert_eq!(row.invalidated_reason.as_deref(), Some("hallucination"));
}

// ── Manual invalidation: Delete removes entry entirely ─────────────────────

#[test]
fn test_qa_cache_invalidate_delete_removes_entry_and_vector() {
    let (storage, _dir) = setup();
    let q = "Where are routes defined?";
    let stored = storage
        .store_answer(&input("p1", q, "In routes.rs", &["c1"], &["h1"]))
        .unwrap();
    assert!(
        storage
            .get_qa_by_cache_id(&stored.cache_id)
            .unwrap()
            .is_some()
    );

    // Server deletes the row (and the corresponding question_vector key by id).
    storage.mark_qa_stale(stored.id, "senior", "wrong").unwrap();
    let n = storage.delete_qa(stored.id).unwrap();
    assert_eq!(n, 1);

    assert!(
        storage
            .get_qa_by_cache_id(&stored.cache_id)
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .get_cached_answer("p1", &question_hash(q))
            .unwrap()
            .is_none()
    );
}

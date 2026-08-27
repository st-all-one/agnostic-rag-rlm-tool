//! Behavioral tests for the query-answer cache (plan 017), backed by `SQLite` via `Storage`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use anyhow::Context;
use arags_storage::Storage;
use arags_storage::qa_cache::{StoreAnswerInput, question_hash};

fn temp_storage() -> Storage {
    let dir = tempfile::tempdir().unwrap();
    Storage::open(dir.path()).unwrap()
}

fn input(project: &str, buffer_id: i64, question: &str, hashes: Vec<String>) -> StoreAnswerInput {
    StoreAnswerInput {
        buffer_id: Some(buffer_id),
        project: project.into(),
        question_text: question.into(),
        question_hash: question_hash(question),
        answer_text: "A".into(),
        source_chunk_ids: vec![],
        source_hashes: hashes,
        model: None,
        tier_snapshot: None,
        token_count: 1,
        created_by: None,
    }
}

#[test]
fn store_and_exact_hit() {
    let storage = temp_storage();
    let inp = StoreAnswerInput {
        buffer_id: Some(1),
        project: "p1".into(),
        question_text: "How do we hash passwords?".into(),
        question_hash: question_hash("How do we hash passwords?"),
        answer_text: "Use argon2id.".into(),
        source_chunk_ids: vec!["c1".into()],
        source_hashes: vec!["h1".into()],
        model: Some("llama3".into()),
        tier_snapshot: None,
        token_count: 10,
        created_by: None,
    };
    let stored = storage.store_answer(&inp).expect("store");
    assert!(stored.created);

    // Storing the same question again SUPERSEDES (new row) rather than reusing.
    let mut again_inp = inp.clone();
    again_inp.answer_text = "Use bcrypt.".into();
    let again = storage.store_answer(&again_inp).expect("store again");
    assert!(again.created);
    assert_ne!(again.cache_id, stored.cache_id);
    assert_ne!(again.id, stored.id);

    // The exact-hit read returns only the latest ACTIVE revision.
    let hit = storage
        .get_cached_answer("p1", &question_hash("How do we hash passwords?"))
        .expect("get")
        .expect("some");
    assert_eq!(hit.answer_text, "Use bcrypt.");
    assert_eq!(hit.source_chunk_ids, vec!["c1".to_string()]);

    // The previous revision is retired and links to the new one.
    let old = storage
        .get_qa_by_rowid(stored.id)
        .expect("old")
        .expect("some");
    assert_eq!(old.is_active, false);
    assert_eq!(old.superseded_by, Some(again.id));

    // Different project is independent.
    assert!(
        storage
            .get_cached_answer("p2", &question_hash("How do we hash passwords?"))
            .expect("get other")
            .is_none()
    );
}

#[test]
fn store_answer_populates_created_by() {
    let storage = temp_storage();
    let mut inp = StoreAnswerInput {
        buffer_id: Some(1),
        project: "p1".into(),
        question_text: "Who wrote this?".into(),
        question_hash: question_hash("Who wrote this?"),
        answer_text: "alice did.".into(),
        source_chunk_ids: vec!["c1".into()],
        source_hashes: vec!["h1".into()],
        model: Some("llama3".into()),
        tier_snapshot: None,
        token_count: 10,
        created_by: Some("alice".into()),
    };
    let stored = storage.store_answer(&inp).expect("store");
    assert!(stored.created);

    let (cb, m): (Option<String>, Option<String>) = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT created_by, model FROM qa_cache WHERE id = ?1",
                rusqlite::params![stored.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .context("read qa_cache authorship")
        })
        .unwrap();
    assert_eq!(cb.as_deref(), Some("alice"));
    assert_eq!(m.as_deref(), Some("llama3"));

    // A re-store SUPERSEDES: a fresh row keeps the new author and the old row
    // keeps its original author (authorship is per-revision).
    inp.created_by = Some("mallory".into());
    let again = storage.store_answer(&inp).expect("store again");
    assert!(again.created);
    assert_ne!(again.id, stored.id);

    let cb_new: Option<String> = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT created_by FROM qa_cache WHERE id = ?1",
                rusqlite::params![again.id],
                |r| r.get(0),
            )
            .context("read new qa_cache authorship")
        })
        .unwrap();
    assert_eq!(cb_new.as_deref(), Some("mallory"));

    let cb_old: Option<String> = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT created_by FROM qa_cache WHERE id = ?1",
                rusqlite::params![stored.id],
                |r| r.get(0),
            )
            .context("read old qa_cache authorship")
        })
        .unwrap();
    assert_eq!(cb_old.as_deref(), Some("alice"));
}

#[test]
fn stale_and_delete() {
    let storage = temp_storage();
    let stored = storage
        .store_answer(&input("p1", 1, "Q", vec!["h1".into()]))
        .unwrap();
    assert!(
        storage
            .mark_qa_stale(stored.id, "admin", "alucinacao")
            .unwrap()
    );
    // Stale entries are not returned by exact lookup.
    assert!(
        storage
            .get_cached_answer("p1", &question_hash("Q"))
            .unwrap()
            .is_none()
    );
    assert_eq!(storage.delete_qa(stored.id).unwrap(), 1);
    assert!(storage.get_qa_by_rowid(stored.id).unwrap().is_none());
}

#[test]
fn lifecycle_marks_stale_by_hash() {
    let storage = temp_storage();
    storage
        .store_answer(&input("p1", 7, "Q", vec!["h1".into(), "h2".into()]))
        .unwrap();
    let n = storage
        .mark_stale_by_hashes(7, &["h2".into()])
        .expect("mark stale");
    assert_eq!(n, 1);
    assert!(
        storage
            .get_cached_answer("p1", &question_hash("Q"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn eviction_keeps_top_entries() {
    let storage = temp_storage();
    for i in 0..5 {
        storage
            .store_answer(&input("p1", 1, &format!("Q{i}"), vec!["h1".into()]))
            .unwrap();
    }
    // Touch Q0 so it has higher access count.
    let q0 = storage
        .get_cached_answer("p1", &question_hash("Q0"))
        .unwrap()
        .unwrap();
    storage.touch_qa(q0.id).unwrap();
    storage.touch_qa(q0.id).unwrap();

    let removed = storage.evict_qa("p1", 3, 1_000_000).unwrap();
    assert_eq!(removed, 2);
    assert!(
        storage
            .get_cached_answer("p1", &question_hash("Q0"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn supersede_qa_creates_new_active_row_and_history() {
    let storage = temp_storage();
    let base = StoreAnswerInput {
        buffer_id: Some(1),
        project: "p1".into(),
        question_text: "What is the timeout?".into(),
        question_hash: question_hash("What is the timeout?"),
        answer_text: "30s".into(),
        source_chunk_ids: vec![],
        source_hashes: vec!["h1".into()],
        model: Some("llama3".into()),
        tier_snapshot: None,
        token_count: 1,
        created_by: Some("alice".into()),
    };

    let v1 = storage.store_answer(&base).expect("v1");
    let mut v2_in = base.clone();
    v2_in.answer_text = "60s".into();
    let v2 = storage.store_answer(&v2_in).expect("v2");
    let mut v3_in = base.clone();
    v3_in.answer_text = "90s".into();
    let v3 = storage.store_answer(&v3_in).expect("v3");

    // (a) exactly one ACTIVE row remains; the earlier two are retired.
    let active_count: i64 = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM qa_cache WHERE project = 'p1' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .context("count active")
        })
        .unwrap();
    assert_eq!(active_count, 1);

    let old = storage.get_qa_by_rowid(v1.id).unwrap().unwrap();
    assert_eq!(old.is_active, false);
    assert_eq!(old.superseded_by, Some(v2.id));
    let mid = storage.get_qa_by_rowid(v2.id).unwrap().unwrap();
    assert_eq!(mid.is_active, false);
    assert_eq!(mid.superseded_by, Some(v3.id));

    // (b) the exact-hit read returns only the latest active revision.
    let latest = storage
        .get_cached_answer("p1", &question_hash("What is the timeout?"))
        .unwrap()
        .unwrap();
    assert_eq!(latest.id, v3.id);
    assert_eq!(latest.answer_text, "90s");

    // (c) the history getter walks the full chain oldest -> newest.
    let history = storage.get_answer_history(v1.id).unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].id, v1.id);
    assert_eq!(history[0].answer_text, "30s");
    assert_eq!(history[1].id, v2.id);
    assert_eq!(history[2].id, v3.id);
    assert_eq!(history[2].answer_text, "90s");
}

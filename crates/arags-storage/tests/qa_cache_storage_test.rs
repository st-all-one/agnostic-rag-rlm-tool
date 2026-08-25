//! Behavioral tests for the query-answer cache (plan 017), backed by `SQLite` via `Storage`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
    };
    let stored = storage.store_answer(&inp).expect("store");
    assert!(stored.created);

    // Idempotent reuse on identical question.
    let again = storage.store_answer(&inp).expect("store again");
    assert!(!again.created);
    assert_eq!(again.cache_id, stored.cache_id);

    let hit = storage
        .get_cached_answer("p1", &question_hash("How do we hash passwords?"))
        .expect("get")
        .expect("some");
    assert_eq!(hit.answer_text, "Use argon2id.");
    assert_eq!(hit.source_chunk_ids, vec!["c1".to_string()]);

    // Different project is independent.
    assert!(
        storage
            .get_cached_answer("p2", &question_hash("How do we hash passwords?"))
            .expect("get other")
            .is_none()
    );
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

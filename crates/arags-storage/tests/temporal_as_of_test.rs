#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

//! Time-travel (`as_of_epoch`) read tests for all four knowledge spaces
//! (plan 021). Each space keeps a supersede chain; the as-of getters must
//! return the revision that was ACTIVE at a given epoch (greatest epoch <= T).

use arags_storage::Storage;
use arags_storage::explorations::PersistExplorationInput;
use arags_storage::qa_cache::{StoreAnswerInput, question_hash};
use arags_storage::sqlite::rlm::NewRlmNode;
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

fn create_buffer(storage: &Storage) -> i64 {
    let conn = storage.conn();
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO buffers (name, path) VALUES ('test', '/test')",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn set_epoch(storage: &Storage, table: &str, id: i64, epoch: i64) {
    let conn = storage.conn();
    let conn = conn.lock();
    let col = if table == "explorations" {
        "epoch_created"
    } else {
        "epoch"
    };
    let sql = format!("UPDATE {table} SET {col} = ?1 WHERE id = ?2");
    conn.execute(&sql, [epoch, id]).unwrap();
}

#[test]
fn time_travel_search_returns_version_active_at_epoch() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_buffer(&storage);
    let conn = storage.conn();

    // Two revisions of the same physical chunk, linked by supersede.
    let c1 = conn.lock();
    c1.execute(
        "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, \
         line_end, hash, language, chunk_type, token_count, status, created_at, \
         last_accessed_at, epoch, created_by, model, version, is_active, superseded_by) \
         VALUES (?1, 'src/main.rs', 0, 0, 10, 20, X'00', 'rust', 'fn', 1, 'indexed', \
                 unixepoch(), unixepoch(), 100, 'alice', 'gpt', 1, 0, 2)",
        [buffer_id],
    )
    .unwrap();
    let id1 = c1.last_insert_rowid();
    c1.execute(
        "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, \
         line_end, hash, language, chunk_type, token_count, status, created_at, \
         last_accessed_at, epoch, created_by, model, version, is_active, superseded_by) \
         VALUES (?1, 'src/main.rs', 0, 0, 10, 20, X'01', 'rust', 'fn', 1, 'indexed', \
                 unixepoch(), unixepoch(), 200, 'bob', 'claude', 2, 1, NULL)",
        [buffer_id],
    )
    .unwrap();
    let id2 = c1.last_insert_rowid();
    c1.execute(
        "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, 'old text')",
        [id1],
    )
    .unwrap();
    c1.execute(
        "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, 'new text')",
        [id2],
    )
    .unwrap();
    drop(c1);

    // Live candidate is id2; time-traveling before id2's epoch returns id1.
    let as_of_old = storage.get_chunk_as_of(id2, 150).unwrap();
    assert_eq!(
        as_of_old.unwrap().id,
        id1,
        "pre-supersede revision active at T=150"
    );
    let as_of_new = storage.get_chunk_as_of(id2, 250).unwrap();
    assert_eq!(
        as_of_new.unwrap().id,
        id2,
        "current revision active at T=250"
    );
    let before_any = storage.get_chunk_as_of(id2, 50).unwrap();
    assert!(before_any.is_none(), "no revision existed before T=50");

    // Content of the as-of revision matches the superseded text.
    let ch = storage.get_chunk_as_of(id2, 150).unwrap().unwrap();
    assert_eq!(
        storage.get_chunk_content(ch.id).unwrap().unwrap(),
        "old text"
    );
}

#[test]
fn time_travel_query_returns_superseded_answer_as_of() {
    let (storage, _tmp) = setup_storage();
    let qh = question_hash("what is the meaning of life?");
    let v1 = storage
        .store_answer(&StoreAnswerInput {
            buffer_id: None,
            project: "p1".into(),
            question_text: "what is the meaning of life?".into(),
            question_hash: qh.clone(),
            answer_text: "42 (old)".into(),
            source_chunk_ids: Vec::new(),
            source_hashes: Vec::new(),
            model: Some("gpt".into()),
            tier_snapshot: Some("{}".into()),
            token_count: 0,
            created_by: Some("alice".into()),
        })
        .unwrap();
    let v2 = storage
        .store_answer(&StoreAnswerInput {
            buffer_id: None,
            project: "p1".into(),
            question_text: "what is the meaning of life?".into(),
            question_hash: qh.clone(),
            answer_text: "43 (new)".into(),
            source_chunk_ids: Vec::new(),
            source_hashes: Vec::new(),
            model: Some("claude".into()),
            tier_snapshot: Some("{}".into()),
            token_count: 0,
            created_by: Some("bob".into()),
        })
        .unwrap();

    // Force distinct epochs on the two revisions (writes default to 0).
    set_epoch(&storage, "qa_cache", v1.id, 100);
    set_epoch(&storage, "qa_cache", v2.id, 200);

    let at_old = storage
        .get_cached_answer_as_of("p1", &qh, None, 150)
        .unwrap();
    assert_eq!(at_old.unwrap().answer_text, "42 (old)");
    let at_new = storage
        .get_cached_answer_as_of("p1", &qh, None, 250)
        .unwrap();
    assert_eq!(at_new.unwrap().answer_text, "43 (new)");
    let before = storage
        .get_cached_answer_as_of("p1", &qh, None, 50)
        .unwrap();
    assert!(before.is_none());
}

#[test]
fn time_travel_rlm_summary_as_of() {
    let (storage, _tmp) = setup_storage();
    let n1 = storage
        .store_rlm_node(&NewRlmNode {
            buffer_id: None,
            project: "p1".into(),
            level: 1,
            subject: "src/main.rs".into(),
            summary_text: "old summary".into(),
            source_hashes: Vec::new(),
            model: Some("gpt".into()),
            volunteer_username: Some("vol".into()),
            created_by: Some("alice".into()),
            template_version: Some("v1".into()),
            token_count: 0,
        })
        .unwrap();
    let _n2 = storage
        .store_rlm_node(&NewRlmNode {
            buffer_id: None,
            project: "p1".into(),
            level: 1,
            subject: "src/main.rs".into(),
            summary_text: "new summary".into(),
            source_hashes: Vec::new(),
            model: Some("claude".into()),
            volunteer_username: Some("vol".into()),
            created_by: Some("bob".into()),
            template_version: Some("v1".into()),
            token_count: 0,
        })
        .unwrap();

    set_epoch(&storage, "rlm_nodes", n1.0, 100);
    // n2 default epoch 0 → bump it above n1 so as_of ordering is meaningful.
    set_epoch(&storage, "rlm_nodes", n1.0, 100);
    let n2_id = storage
        .get_rlm_node_by_subject("p1", 1, "src/main.rs")
        .unwrap()
        .unwrap()
        .id;
    set_epoch(&storage, "rlm_nodes", n2_id, 200);

    let at_old = storage
        .get_rlm_node_as_of("p1", 1, "src/main.rs", 150)
        .unwrap();
    assert_eq!(at_old.unwrap().summary_text, "old summary");
    let at_new = storage
        .get_rlm_node_as_of("p1", 1, "src/main.rs", 250)
        .unwrap();
    assert_eq!(at_new.unwrap().summary_text, "new summary");
}

#[test]
fn time_travel_exploration_as_of() {
    let (storage, _tmp) = setup_storage();
    storage
        .persist_exploration(&PersistExplorationInput {
            project: "p1".into(),
            buffer_id: None,
            goal: "understand auth".into(),
            body_markdown: "## Mapa\n\nold map".into(),
            summary: "old summary".into(),
            anchors: Vec::new(),
            created_by: "alice".into(),
            model: Some("gpt".into()),
            template_version: "v1".into(),
            token_count: 0,
        })
        .unwrap();
    let v2 = storage
        .persist_exploration(&PersistExplorationInput {
            project: "p1".into(),
            buffer_id: None,
            goal: "understand auth".into(),
            body_markdown: "## Mapa\n\nnew map".into(),
            summary: "new summary".into(),
            anchors: Vec::new(),
            created_by: "bob".into(),
            model: Some("claude".into()),
            template_version: "v1".into(),
            token_count: 0,
        })
        .unwrap();

    // v1's row id is v2.id - 1 (sequential inserts in the same tx); fetch it.
    let v1_id = v2.id - 1;
    set_epoch(&storage, "explorations", v1_id, 100);
    set_epoch(&storage, "explorations", v2.id, 200);

    let at_old = storage
        .get_exploration_as_of("p1", "understand auth", 150)
        .unwrap();
    assert_eq!(at_old.as_ref().unwrap().summary, "old summary");
    let at_new = storage
        .get_exploration_as_of("p1", "understand auth", 250)
        .unwrap();
    assert_eq!(at_new.unwrap().summary, "new summary");
    let by_id_old = storage
        .get_exploration_as_of_by_id(&at_old.unwrap().exploration_id, 150)
        .unwrap();
    assert_eq!(by_id_old.unwrap().summary, "old summary");
}

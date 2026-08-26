#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Plan 023 trust-pipeline tests for the QA cache (provenance drift and
//! cross-project isolation), split from `tests.rs` to respect the 300-line
//! gate.

use super::tests::{authed_state, bearer, temp_storage};
use super::*;
use crate::config::ServerConfig;
use arags_storage::{tokens::NewToken, tokens::Role};

/// Regression (agnostic-rlm-rs-3c84): the question-vector space is global, so
/// a near-hit candidate from another project must never be served.
#[tokio::test]
async fn near_hit_from_other_project_is_not_served() {
    let storage = temp_storage();
    let (_, refresh) = arags_storage::tokens::create_token(
        &storage,
        &NewToken {
            username: "dev1".into(),
            role: Role::NonAdmin,
            created_by: "system".into(),
        },
    )
    .expect("create token");
    let (session, _, _, _) =
        arags_storage::tokens::create_session(&storage, &refresh).expect("create session");
    // Wire a real question-vector store so the near-hit path runs; identical
    // question text guarantees similarity 1.0 even with the hash embedder.
    let vdir = tempfile::tempdir().expect("vtempdir");
    let qv_store = std::sync::Arc::new(
        arags_storage::QuestionVectorStore::open(vdir.path(), crate::state::embedder_dimension())
            .expect("question vector store"),
    );
    let state = AppState::with_vector_stores(
        storage.clone(),
        ServerConfig::default(),
        None,
        Some(qv_store),
        None,
        None,
    )
    .expect("app state");
    let auth = bearer(&session);

    // Store an answer under project "other" with a question that shares no
    // exact hash with anything in "p1".
    let mut store = Request::new(StoreAnswerRequest {
        project: "other".into(),
        question: "How do we hash passwords?".into(),
        answer: "Use argon2id.".into(),
        source_chunk_ids: vec![],
        source_hashes: vec![],
        model: "llama3".into(),
        token_count: 12,
        buffer_id: 0,
        cache_id: String::new(),
    });
    *store.metadata_mut() = bearer(&session);
    handle_store_answer(&state, store)
        .await
        .expect("store other");

    // Querying "p1" with the very same question must MISS even though the
    // question-vector space holds the "other" project's entry at sim 1.0:
    // the exact hit filters by project, and the near-hit must too.
    let mut q = Request::new(QueryWithCacheRequest {
        project: "p1".into(),
        question: "How do we hash passwords?".into(),
        buffer_id: 0,
    });
    *q.metadata_mut() = auth;
    let resp = handle_query_with_cache(&state, q)
        .await
        .expect("query p1")
        .into_inner();
    assert!(!resp.hit, "cross-project near-hit must not be served");
}

/// Trust pipeline (agnostic-rlm-rs-ac7f): a cached answer whose provenance
/// chunks changed content is stale at read time — the first hit detects the
/// drift, marks the entry stale and falls through to MISS.
#[tokio::test]
async fn exact_hit_with_drifted_provenance_serves_miss() {
    let storage = temp_storage();
    // Seed one chunk with a known content hash ("hash-a" → hex).
    storage
        .connection()
        .expect("conn")
        .execute(|c| {
            c.execute("INSERT INTO buffers (name, path) VALUES ('p1', '/p1')", [])?;
            c.execute(
                "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, \
                 line_start, line_end, hash) VALUES (1, 'src/a.rs', 0, 1, 1, 1, ?1)",
                rusqlite::params![b"hash-a"],
            )?;
            Ok(())
        })
        .expect("seed");
    let chunk_id: i64 = storage
        .connection()
        .expect("conn")
        .execute(|c| Ok(c.last_insert_rowid()))
        .expect("rowid");

    let (_, refresh) = arags_storage::tokens::create_token(
        &storage,
        &NewToken {
            username: "dev1".into(),
            role: Role::NonAdmin,
            created_by: "system".into(),
        },
    )
    .expect("create token");
    let (session, _, _, _) =
        arags_storage::tokens::create_session(&storage, &refresh).expect("create session");
    let state = authed_state(&storage);
    // Store an answer citing the chunk's current hash.
    let mut store = Request::new(StoreAnswerRequest {
        project: "p1".into(),
        question: "How do we hash passwords?".into(),
        answer: "Use argon2id.".into(),
        source_chunk_ids: vec![chunk_id.to_string()],
        source_hashes: vec![hex::encode(b"hash-a")],
        model: "llama3".into(),
        token_count: 12,
        buffer_id: 0,
        cache_id: String::new(),
    });
    *store.metadata_mut() = bearer(&session);
    handle_store_answer(&state, store).await.expect("store");

    // First query: provenance intact → hit.
    let query = || {
        let mut q = Request::new(QueryWithCacheRequest {
            project: "p1".into(),
            question: "How do we hash passwords?".into(),
            buffer_id: 0,
        });
        *q.metadata_mut() = bearer(&session);
        q
    };
    let hit = handle_query_with_cache(&state, query())
        .await
        .expect("query")
        .into_inner();
    assert!(hit.hit, "intact provenance must serve the cached answer");

    // The cited file changes on re-index.
    storage
        .connection()
        .expect("conn")
        .execute(|c| {
            c.execute(
                "UPDATE chunks SET hash = ?1 WHERE id = ?2",
                rusqlite::params![b"hash-a-new", chunk_id],
            )?;
            Ok(())
        })
        .expect("rewrite chunk");

    // Second query: drift detected → MISS (and the entry is marked stale).
    let after = handle_query_with_cache(&state, query())
        .await
        .expect("query after drift")
        .into_inner();
    assert!(
        !after.hit,
        "drifted provenance must not serve a stale answer"
    );
}

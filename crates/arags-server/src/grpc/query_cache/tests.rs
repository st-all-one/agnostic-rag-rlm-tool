#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::str::FromStr;

use tonic::Request;
use tonic::metadata::{MetadataMap, MetadataValue};

use arags_proto::proto::{
    GetAnswerByIdRequest, InvalidateCacheRequest, InvalidateMode, QueryWithCacheRequest,
    StoreAnswerRequest,
};

use crate::config::ServerConfig;
use crate::state::AppState;
use arags_storage::{Storage, tokens::NewToken, tokens::Role};

pub(crate) fn bearer(token: &str) -> MetadataMap {
    let mut md = MetadataMap::new();
    let value = MetadataValue::<tonic::metadata::Ascii>::from_str(&format!("Bearer {token}"))
        .expect("valid metadata value");
    md.insert("authorization", value);
    md
}

pub(crate) fn temp_storage() -> Storage {
    let dir = tempfile::tempdir().expect("tempdir");
    Storage::open(dir.path()).expect("open storage")
}

pub(crate) fn authed_state(storage: &Storage) -> AppState {
    AppState::new(storage.clone(), ServerConfig::default(), None, None).expect("app state")
}

#[tokio::test]
async fn non_admin_cannot_invalidate() {
    let storage = temp_storage();
    storage
        .put_cached_result("h1", "proj", "answer")
        .expect("seed cache");

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
    let mut req = Request::new(InvalidateCacheRequest {
        project: "proj".into(),
        ..Default::default()
    });
    *req.metadata_mut() = bearer(&session);

    let err = handle_invalidate_cache(&state, req)
        .await
        .expect_err("non-admin must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);

    assert!(
        storage
            .get_cached_result("h1", "proj")
            .expect("get")
            .is_some(),
        "cache should be untouched after denied invalidation"
    );
}

#[tokio::test]
async fn admin_can_invalidate_and_audit() {
    let storage = temp_storage();
    storage
        .put_cached_result("h1", "proj", "answer")
        .expect("seed cache");
    storage
        .put_cached_result("h2", "other", "answer2")
        .expect("seed cache");

    let (_, refresh) = arags_storage::tokens::create_token(
        &storage,
        &NewToken {
            username: "admin1".into(),
            role: Role::Admin,
            created_by: "system".into(),
        },
    )
    .expect("create token");
    let (session, _, _, _) =
        arags_storage::tokens::create_session(&storage, &refresh).expect("create session");

    let state = authed_state(&storage);

    let mut req = Request::new(InvalidateCacheRequest {
        project: "proj".into(),
        ..Default::default()
    });
    *req.metadata_mut() = bearer(&session);
    let resp = handle_invalidate_cache(&state, req)
        .await
        .expect("admin invalidation")
        .into_inner();
    assert_eq!(resp.invalidated, 1);
    assert_eq!(resp.invalidated_by, "admin1");
    assert!(
        storage
            .get_cached_result("h1", "proj")
            .expect("get")
            .is_none(),
        "proj entry should be gone"
    );
    assert!(
        storage
            .get_cached_result("h2", "other")
            .expect("get")
            .is_some(),
        "other project untouched"
    );

    let mut req2 = Request::new(InvalidateCacheRequest {
        project: String::new(),
        ..Default::default()
    });
    *req2.metadata_mut() = bearer(&session);
    let resp2 = handle_invalidate_cache(&state, req2)
        .await
        .expect("admin full purge")
        .into_inner();
    assert_eq!(resp2.invalidated, 1);
    assert!(
        storage
            .get_cached_result("h2", "other")
            .expect("get")
            .is_none(),
        "all entries purged"
    );
}

#[tokio::test]
async fn missing_session_is_unauthenticated() {
    let storage = temp_storage();
    let state = authed_state(&storage);
    let req = Request::new(InvalidateCacheRequest {
        project: String::new(),
        ..Default::default()
    });
    let err = handle_invalidate_cache(&state, req)
        .await
        .expect_err("no bearer must be unauthenticated");
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

/// Plan 017 end-to-end: store → cache hit → direct lookup → stale
/// invalidation forces a re-digest (MISS) on the next query.
#[tokio::test]
async fn query_store_answer_hit_get_invalidate() {
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
    let state = authed_state(&storage);
    let auth = bearer(&session);

    // Store a digested answer.
    let mut store = Request::new(StoreAnswerRequest {
        project: "p1".into(),
        question: "How do we hash passwords?".into(),
        answer: "Use argon2id.".into(),
        source_chunk_ids: vec![],
        source_hashes: vec![],
        model: "llama3".into(),
        token_count: 12,
        buffer_id: 0,
        cache_id: String::new(),
    });
    *store.metadata_mut() = auth.clone();
    let cache_id = handle_store_answer(&state, store)
        .await
        .expect("store")
        .into_inner()
        .cache_id;
    assert!(!cache_id.is_empty());

    // Exact cache hit (zero LLM on the client).
    let mut q = Request::new(QueryWithCacheRequest {
        project: "p1".into(),
        question: "How do we hash passwords?".into(),
        buffer_id: 0,
        as_of_epoch: 0,
    });
    *q.metadata_mut() = auth.clone();
    let hit = handle_query_with_cache(&state, q)
        .await
        .expect("query")
        .into_inner();
    assert!(hit.hit);
    assert_eq!(hit.answer_text, "Use argon2id.");

    // Direct, deterministic lookup by stable id (anti-drift).
    let mut g = Request::new(GetAnswerByIdRequest {
        cache_id: cache_id.clone(),
        project: "p1".into(),
    });
    *g.metadata_mut() = auth.clone();
    let got = handle_get_answer_by_id(&state, g)
        .await
        .expect("get")
        .into_inner();
    assert!(got.found);
    assert_eq!(got.answer_text, "Use argon2id.");

    // Admin stale-invalidates the entry.
    let (_, admin_refresh) = arags_storage::tokens::create_token(
        &storage,
        &NewToken {
            username: "admin1".into(),
            role: Role::Admin,
            created_by: "system".into(),
        },
    )
    .expect("create admin token");
    let (admin_session, _, _, _) =
        arags_storage::tokens::create_session(&storage, &admin_refresh).expect("admin session");
    let mut inv = Request::new(InvalidateCacheRequest {
        project: String::new(),
        cache_id: cache_id.clone(),
        mode: InvalidateMode::Stale as i32,
        similarity_radius: 0.0,
    });
    *inv.metadata_mut() = bearer(&admin_session);
    let invr = handle_invalidate_cache(&state, inv)
        .await
        .expect("invalidate")
        .into_inner();
    assert!(invr.invalidated >= 1);
    assert_eq!(invr.invalidated_by, "admin1");

    // After stale, the exact query becomes a MISS (forces re-digest).
    let mut q2 = Request::new(QueryWithCacheRequest {
        project: "p1".into(),
        question: "How do we hash passwords?".into(),
        buffer_id: 0,
        as_of_epoch: 0,
    });
    *q2.metadata_mut() = auth.clone();
    let miss = handle_query_with_cache(&state, q2)
        .await
        .expect("query2")
        .into_inner();
    assert!(!miss.hit);
}

/// Plan 017: hard delete removes the entry entirely (GetAnswerById 404).
#[tokio::test]
async fn admin_delete_removes_entry() {
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
    let state = authed_state(&storage);

    let mut store = Request::new(StoreAnswerRequest {
        project: "p1".into(),
        question: "Q?".into(),
        answer: "A".into(),
        source_chunk_ids: vec![],
        source_hashes: vec![],
        model: String::new(),
        token_count: 0,
        buffer_id: 0,
        cache_id: String::new(),
    });
    *store.metadata_mut() = bearer(&session);
    let cache_id = handle_store_answer(&state, store)
        .await
        .expect("store")
        .into_inner()
        .cache_id;

    let (_, admin_refresh) = arags_storage::tokens::create_token(
        &storage,
        &NewToken {
            username: "admin1".into(),
            role: Role::Admin,
            created_by: "system".into(),
        },
    )
    .expect("create admin token");
    let (admin_session, _, _, _) =
        arags_storage::tokens::create_session(&storage, &admin_refresh).expect("admin session");

    let mut inv = Request::new(InvalidateCacheRequest {
        project: String::new(),
        cache_id: cache_id.clone(),
        mode: InvalidateMode::Delete as i32,
        similarity_radius: 0.0,
    });
    *inv.metadata_mut() = bearer(&admin_session);
    let invr = handle_invalidate_cache(&state, inv)
        .await
        .expect("invalidate")
        .into_inner();
    assert!(invr.invalidated >= 1);

    let mut g = Request::new(GetAnswerByIdRequest {
        cache_id: cache_id.clone(),
        project: "p1".into(),
    });
    *g.metadata_mut() = bearer(&session);
    let got = handle_get_answer_by_id(&state, g)
        .await
        .expect("get")
        .into_inner();
    assert!(!got.found);
}

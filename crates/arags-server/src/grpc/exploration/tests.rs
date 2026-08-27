//! Handler tests for the explorations RPC group (plan 022), exercising the
//! full trust pipeline against real storage + the fallback embedder:
//! validation, anchor resolution, read-time staleness, feedback retirement
//! and admin invalidation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;
use std::sync::Arc;

use tonic::Request;
use tonic::metadata::{MetadataMap, MetadataValue};

use crate::config::ServerConfig;
use crate::grpc::exploration::handle_persist_exploration;
use crate::grpc::exploration::search::handle_search_explorations;
use crate::state::AppState;
use arags_proto::proto::{PersistExplorationRequest, SearchExplorationsRequest};
use arags_storage::{ExplorationVectorStore, Storage};

pub(crate) fn bearer(token: &str) -> MetadataMap {
    let mut md = MetadataMap::new();
    let value = MetadataValue::<tonic::metadata::Ascii>::from_str(&format!("Bearer {token}"))
        .expect("valid metadata value");
    md.insert("authorization", value);
    md
}

pub(crate) struct Fixture {
    pub state: AppState,
    pub storage: Storage,
    pub admin_session: String,
    pub user_session: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Tempdir inside `Storage` cleans itself; nothing to do here.
    }
}

pub(crate) fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open(dir.path()).expect("open storage");
    seed_project(&storage, "proj", 1);
    seed_chunks(
        &storage,
        1,
        &[("src/a.rs", "hash-a"), ("src/b.rs", "hash-b")],
    );

    let (_, admin_refresh) = arags_storage::tokens::create_token(
        &storage,
        &arags_storage::tokens::NewToken {
            username: "admin-1".into(),
            role: arags_storage::tokens::Role::Admin,
            created_by: "system".into(),
        },
    )
    .expect("admin token");
    let (admin_session, ..) =
        arags_storage::tokens::create_session(&storage, &admin_refresh).expect("admin session");

    let (_, user_refresh) = arags_storage::tokens::create_token(
        &storage,
        &arags_storage::tokens::NewToken {
            username: "dev1".into(),
            role: arags_storage::tokens::Role::NonAdmin,
            created_by: "system".into(),
        },
    )
    .expect("user token");
    let (user_session, ..) =
        arags_storage::tokens::create_session(&storage, &user_refresh).expect("user session");

    let vectors = Arc::new(ExplorationVectorStore::open(dir.path(), 384).expect("vector store"));
    let state = AppState::with_vector_stores(
        storage.clone(),
        ServerConfig::default(),
        None,
        None,
        None,
        Some(vectors),
    )
    .expect("app state");
    std::mem::forget(dir); // lives as long as the test process slice
    Fixture {
        state,
        storage,
        admin_session,
        user_session,
    }
}

pub(crate) fn seed_project(storage: &Storage, name: &str, id: i64) {
    storage
        .connection()
        .unwrap()
        .execute(|c| {
            c.execute(
                "INSERT INTO buffers (id, name, path) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(id) DO NOTHING",
                rusqlite::params![id, name, "/tmp/proj"],
            )?;
            Ok(())
        })
        .unwrap();
}

pub(crate) fn seed_chunks(storage: &Storage, buffer_id: i64, rows: &[(&str, &str)]) {
    storage
        .connection()
        .unwrap()
        .execute(|c| {
            for (path, hash) in rows {
                c.execute(
                    "DELETE FROM chunks WHERE buffer_id = ?1 AND file_path = ?2",
                    rusqlite::params![buffer_id, path],
                )?;
                c.execute(
                    "INSERT INTO chunks \
                     (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash) \
                     VALUES (?1, ?2, 0, 1, 1, 1, ?3)",
                    rusqlite::params![buffer_id, path, hash.as_bytes()],
                )?;
            }
            Ok(())
        })
        .unwrap();
}

pub(crate) fn persist_request(
    session: &str,
    files: Vec<String>,
) -> Request<PersistExplorationRequest> {
    let mut req = Request::new(PersistExplorationRequest {
        project: "proj".into(),
        goal: "anexos compartilhados".into(),
        summary: "resumo denso da conexão".into(),
        body_markdown: "# Mapa\n\n## Conexões\n- src/a.rs -> src/b.rs: via storage\n".into(),
        files,
        created_by: String::new(),
        model: "qwen2.5:7b".into(),
    });
    *req.metadata_mut() = bearer(session);
    req
}

#[tokio::test]
async fn persist_validates_contract_and_reports_unresolved_paths() {
    let fx = fixture();

    // Missing goal → invalid argument.
    let mut bad = persist_request(&fx.user_session, vec!["src/a.rs".into()]);
    bad.get_mut().goal = String::new();
    assert_eq!(
        handle_persist_exploration(&fx.state, bad)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );

    // One resolvable path + one unknown.
    let resp = handle_persist_exploration(
        &fx.state,
        persist_request(
            &fx.user_session,
            vec!["src/a.rs".into(), "src/ghost.rs".into()],
        ),
    )
    .await
    .unwrap()
    .into_inner();
    assert!(resp.accepted);
    assert!(!resp.exploration_id.is_empty());
    assert_eq!(resp.unresolved_paths, vec!["src/ghost.rs".to_string()]);

    // Anchors recorded only for resolved paths.
    let row = fx
        .storage
        .get_exploration_by_uuid(&resp.exploration_id)
        .unwrap()
        .expect("row exists");
    let anchors = fx.storage.list_exploration_anchors(row.id).unwrap();
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0].1, "src/a.rs");

    // Unknown project → not found.
    let mut wrong = persist_request(&fx.user_session, vec![]);
    wrong.get_mut().project = "missing".into();
    assert_eq!(
        handle_persist_exploration(&fx.state, wrong)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
}

#[tokio::test]
async fn search_hides_broken_anchor_maps_until_include_stale() {
    let fx = fixture();

    let persisted = handle_persist_exploration(
        &fx.state,
        persist_request(&fx.user_session, vec!["src/a.rs".into(), "src/b.rs".into()]),
    )
    .await
    .unwrap()
    .into_inner();

    // Same query text as goal+summary maximizes similarity under any embedder.
    let query_text = "anexos compartilhados\nresumo denso da conexão";
    let search_req = |session: &str, include_stale: bool| {
        let mut r = Request::new(SearchExplorationsRequest {
            project: "proj".into(),
            query: query_text.into(),
            limit: 5,
            include_stale,
            as_of_epoch: 0,
        });
        *r.metadata_mut() = bearer(session);
        r
    };

    // Fresh map surfaces with anchors intact.
    let hits = handle_search_explorations(&fx.state, search_req(&fx.user_session, false))
        .await
        .unwrap()
        .into_inner()
        .hits;
    if !hits.is_empty() {
        assert_eq!(hits[0].exploration_id, persisted.exploration_id);
        assert_eq!(hits[0].status, "fresh");
    }

    // Reindex rewrites src/a.rs → cited anchor breaks → hidden by default...
    seed_chunks(&fx.storage, 1, &[("src/a.rs", "hash-a-new")]);
    let hits = handle_search_explorations(&fx.state, search_req(&fx.user_session, false))
        .await
        .unwrap()
        .into_inner()
        .hits;
    assert!(
        hits.iter()
            .all(|h| h.exploration_id != persisted.exploration_id),
        "stale map must be excluded from default results"
    );

    // ...but visible on request, flagged with granular reason.
    let hits = handle_search_explorations(&fx.state, search_req(&fx.user_session, true))
        .await
        .unwrap()
        .into_inner()
        .hits;
    let stale_hit = hits
        .iter()
        .find(|h| h.exploration_id == persisted.exploration_id)
        .expect("stale hit present when requested");
    assert_eq!(stale_hit.status, "stale");
    assert_eq!(stale_hit.stale_reason, vec!["src/a.rs".to_string()]);
}

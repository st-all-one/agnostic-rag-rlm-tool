//! Verify-on-hit grounding tests (plan 022.8). Uses a strict
//! `grounding_min_similarity` so both outcomes stay deterministic under any
//! embedder: exact corpus text grounds (distance 0), invented text does not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use tonic::Request;

use crate::config::{ServerConfig, ValidationMode};
use crate::grpc::exploration::handle_persist_exploration;
use crate::grpc::exploration::search::handle_search_explorations;
use crate::state::AppState;
use arags_proto::proto::{PersistExplorationRequest, SearchExplorationsRequest};
use arags_storage::{ExplorationVectorStore, Storage};

use super::tests::{bearer, seed_chunks, seed_project};

#[tokio::test]
async fn verify_on_hit_downgrades_maps_without_corpus_support() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open(dir.path()).expect("open storage");
    seed_project(&storage, "proj", 1);
    seed_chunks(&storage, 1, &[("src/a.rs", "hash-a")]);

    let (_, refresh) = arags_storage::tokens::create_token(
        &storage,
        &arags_storage::tokens::NewToken {
            username: "dev1".into(),
            role: arags_storage::tokens::Role::NonAdmin,
            created_by: "system".into(),
        },
    )
    .expect("token");
    let (session, ..) = arags_storage::tokens::create_session(&storage, &refresh).expect("session");

    // Chunk space seeded with ONE real concept; strict threshold makes both
    // outcomes deterministic under any embedder.
    let vectors = Arc::new(ExplorationVectorStore::open(dir.path(), 384).unwrap());
    let chunk_space = Arc::new(
        arags_storage::lance::vectors::VectorStore::open_with_dims(dir.path(), 384)
            .await
            .expect("chunk space"),
    );
    let mut config = ServerConfig::default();
    config.exploration.verify_on_hit = true;
    config.exploration.grounding_min_similarity = 0.95;
    // Fire-and-forget: `Review` mode with `require_review = false` keeps
    // non-admin maps `fresh` and surfacing (the new default `Quorum` holds
    // them pending for the quorum worker).
    config.exploration.validation_mode = ValidationMode::Review;

    let embedder_text = "pipeline de pagamento com idempotencia por chave";
    let state = AppState::with_vector_stores(
        storage.clone(),
        config,
        Some(chunk_space.clone()),
        None,
        None,
        Some(vectors),
    )
    .expect("state");
    let claim_vec = state.embedder.embed(embedder_text).expect("embed");
    chunk_space
        .insert_vectors(&[arags_storage::lance::vectors::VectorEntry {
            chunk_id: 1,
            buffer_id: 1,
            vector: claim_vec,
        }])
        .await
        .expect("insert vector");

    let persist = |goal: String, conexoes: String| {
        let mut r = Request::new(PersistExplorationRequest {
            project: "proj".into(),
            goal,
            summary: "s".into(),
            body_markdown: format!(
                "## Mapa
m

## Conexões
{conexoes}

## Evidências
e

## Limitações
l
"
            ),
            files: vec!["src/a.rs".into()],
            created_by: String::new(),
            model: "m".into(),
        });
        *r.metadata_mut() = bearer(&session);
        r
    };
    let search = |include_stale: bool| {
        let mut r = Request::new(SearchExplorationsRequest {
            project: "proj".into(),
            query: "q".into(),
            limit: 10,
            include_stale,
            as_of_epoch: 0,
        });
        *r.metadata_mut() = bearer(&session);
        r
    };

    // Grounded claim survives verification.
    let ok = handle_persist_exploration(
        &state,
        persist("mapa suportado".into(), embedder_text.into()),
    )
    .await
    .unwrap()
    .into_inner();
    let hits = handle_search_explorations(&state, search(false))
        .await
        .unwrap()
        .into_inner()
        .hits;
    assert!(
        hits.iter()
            .any(|h| h.exploration_id == ok.exploration_id && h.status == "fresh"),
        "grounded map must stay fresh"
    );

    // Unsupported claim (no such text anywhere in the corpus) is forced stale.
    let bad = handle_persist_exploration(
        &state,
        persist(
            "mapa alucinado".into(),
            "zzz conceito inexistente qqzz".into(),
        ),
    )
    .await
    .unwrap()
    .into_inner();
    let hits = handle_search_explorations(&state, search(false))
        .await
        .unwrap()
        .into_inner()
        .hits;
    assert!(hits.iter().all(|h| h.exploration_id != bad.exploration_id));

    let hits = handle_search_explorations(&state, search(true))
        .await
        .unwrap()
        .into_inner()
        .hits;
    let downgraded = hits
        .iter()
        .find(|h| h.exploration_id == bad.exploration_id)
        .expect("visible when asked");
    assert_eq!(downgraded.status, "stale");
    assert!(downgraded.stale_reason[0].starts_with("grounding weak"));
}

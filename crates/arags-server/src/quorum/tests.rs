#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use std::sync::Arc;

use arags_embedding::embedder::{Embedder, LightweightEmbedder};
use arags_storage::sqlite::rlm::{DEFAULT_RLM_LEASE_MS, NewRlmJob, rlm_job_key};
use arags_storage::{RlmVectorStore, Storage};

use crate::config::{FusionStrategy, ServerConfig};
use crate::state::AppState;

use super::{QuorumDecision, decide_rlm_quorum};

/// Build a state with a deterministic (weight-free) embedder and a real RLM
/// vector store, with the quorum fan-out set to `quorum_n`. `rlm.enabled` is
/// forced off so the decision worker does not trigger cascades during tests.
async fn fixture(quorum_n: usize) -> (tempfile::TempDir, Storage, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path()).unwrap();
    let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
    let rlm_vs = Arc::new(RlmVectorStore::open(dir.path(), dims).unwrap());
    let mut cfg = ServerConfig::default();
    cfg.rlm.enabled = false;
    cfg.quorum.n = quorum_n;
    cfg.quorum.quorum_sim_threshold = 0.85;
    cfg.quorum.fusion_strategy = FusionStrategy::Consensus;
    let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(LightweightEmbedder::new(dims));
    let state = AppState::with_embedder(
        storage.clone(),
        cfg,
        embedder,
        None,
        None,
        Some(rlm_vs),
        None,
    )
    .unwrap();
    (dir, storage, state)
}

fn subject_key(project: &str, level: i64, subject: &str) -> String {
    rlm_job_key(project, level, subject)
}

#[tokio::test]
async fn rlm_job_creates_n_independent_slots() {
    let (_dir, storage, _state) = fixture(3).await;
    let (id, _gen) = storage
        .enqueue_rlm_job(&NewRlmJob {
            buffer_id: Some(1),
            project: "proj".into(),
            level: 1,
            subject: "src/a.rs".into(),
            payload: "{}".into(),
            priority: 5,
            quorum_slots: 3,
        })
        .unwrap();
    assert!(id > 0);

    // Three physical, independently claimable slots sharing one group.
    let group: Vec<(i64, Option<i64>)> = storage
        .connection()
        .unwrap()
        .execute(|c| {
            let mut stmt = c.prepare(
                "SELECT id, generation_group_id FROM rlm_jobs \
                 WHERE project = 'proj' AND level = 1 AND subject = 'src/a.rs' \
                 ORDER BY id ASC",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
                })
                .unwrap();
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
        .unwrap();
    assert_eq!(group.len(), 3, "must fan out to 3 slots");
    let gid = group[0].1;
    assert!(gid.is_some());
    assert!(
        group.iter().all(|(_, g)| *g == gid),
        "slots share a group id"
    );

    // N distinct volunteers each claim a distinct slot.
    let a = storage
        .claim_rlm_job("alice", DEFAULT_RLM_LEASE_MS, None)
        .unwrap()
        .unwrap();
    let b = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None)
        .unwrap()
        .unwrap();
    let c = storage
        .claim_rlm_job("carol", DEFAULT_RLM_LEASE_MS, None)
        .unwrap()
        .unwrap();
    let claimed: std::collections::HashSet<i64> = [a.id, b.id, c.id].into();
    assert_eq!(claimed.len(), 3, "three distinct slots claimed");
    assert_ne!(a.id, b.id);
    assert_ne!(b.id, c.id);

    // Alice cannot claim a second slot in the same group (one slot per group).
    assert!(
        storage
            .claim_rlm_job("alice", DEFAULT_RLM_LEASE_MS, None)
            .unwrap()
            .is_none(),
        "a volunteer may hold at most one slot per group"
    );

    // Once all three are claimed, nothing remains claimable.
    assert!(
        storage
            .claim_rlm_job("dave", DEFAULT_RLM_LEASE_MS, None)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn rlm_quorum_accepts_fused_consensus() {
    let (_dir, storage, state) = fixture(3).await;
    let project = "proj";
    let level = 1i64;
    let subject = "src/a.rs";
    let key = subject_key(project, level, subject);

    // Two agreeing candidates (identical text => cosine 1.0) + one divergent.
    let agreeing = "The module parses JSON configuration files and exposes a typed API.";
    let divergent = "This file renders pixel shaders and GPU compute kernels for the renderer.";
    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "alice")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "bob")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, divergent, "mallory")
        .unwrap();

    let decision = decide_rlm_quorum(&state, project, level, subject)
        .await
        .unwrap();
    match decision {
        QuorumDecision::Accepted {
            fused_text,
            accepted_submission_ids,
            rejected_submission_ids,
        } => {
            assert_eq!(fused_text, agreeing, "consensus text is the agreeing text");
            assert_eq!(accepted_submission_ids.len(), 2, "two winners");
            assert_eq!(rejected_submission_ids.len(), 1, "one divergent rejected");
        }
        other => panic!("expected Accepted, got {other:?}"),
    }

    // The live node is published and approved.
    let node = storage
        .get_rlm_node_by_subject(project, level, subject)
        .unwrap()
        .unwrap();
    assert_eq!(node.review_status, "approved");
    assert_eq!(node.summary_text, agreeing);

    // The divergent candidate is rejected and its author took a strike.
    assert_eq!(
        storage
            .list_pending(project, "rlm_node", &key)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(storage.volunteer_strikes("mallory").unwrap(), 1);
}

#[tokio::test]
async fn rlm_quorum_rejects_when_all_diverge() {
    let (_dir, storage, state) = fixture(3).await;
    let project = "proj";
    let level = 1i64;
    let subject = "src/a.rs";
    let key = subject_key(project, level, subject);

    storage
        .insert_submission(project, "rlm_node", &key, "Alpha does X and Y.", "alice")
        .unwrap();
    storage
        .insert_submission(
            project,
            "rlm_node",
            &key,
            "Beta performs entirely unrelated work.",
            "bob",
        )
        .unwrap();
    storage
        .insert_submission(
            project,
            "rlm_node",
            &key,
            "Gamma handles a third distinct concern.",
            "carol",
        )
        .unwrap();

    let decision = decide_rlm_quorum(&state, project, level, subject)
        .await
        .unwrap();
    assert!(
        matches!(decision, QuorumDecision::Rejected { rejected_submission_ids } if rejected_submission_ids.len() == 3),
        "all three diverge -> reject all"
    );

    // No node was published, and every author took a strike.
    assert!(
        storage
            .get_rlm_node_by_subject(project, level, subject)
            .unwrap()
            .is_none(),
        "node must NOT be published on no-quorum"
    );
    assert_eq!(storage.volunteer_strikes("alice").unwrap(), 1);
    assert_eq!(storage.volunteer_strikes("bob").unwrap(), 1);
    assert_eq!(storage.volunteer_strikes("carol").unwrap(), 1);
}

#[tokio::test]
async fn rlm_quorum_pending_until_n_candidates() {
    let (_dir, storage, state) = fixture(3).await;
    let project = "proj";
    let level = 1i64;
    let subject = "src/a.rs";
    let key = subject_key(project, level, subject);

    storage
        .insert_submission(
            project,
            "rlm_node",
            &key,
            "First volunteer result.",
            "alice",
        )
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, "Second volunteer result.", "bob")
        .unwrap();

    let decision = decide_rlm_quorum(&state, project, level, subject)
        .await
        .unwrap();
    assert!(
        matches!(decision, QuorumDecision::Pending),
        "fewer than n candidates -> defer"
    );

    // Still pending, no node published, no strikes yet.
    assert_eq!(
        storage
            .list_pending(project, "rlm_node", &key)
            .unwrap()
            .len(),
        2
    );
    assert!(
        storage
            .get_rlm_node_by_subject(project, level, subject)
            .unwrap()
            .is_none()
    );
    assert_eq!(storage.volunteer_strikes("alice").unwrap(), 0);
}

#[tokio::test]
async fn rlm_quorum_is_idempotent_after_accept() {
    let (_dir, storage, state) = fixture(3).await;
    let project = "proj";
    let level = 1i64;
    let subject = "src/a.rs";
    let key = subject_key(project, level, subject);
    let agreeing = "Consistent summary text shared by the agreeing volunteers.";

    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "alice")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "bob")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, "totally different", "mallory")
        .unwrap();

    let first = decide_rlm_quorum(&state, project, level, subject)
        .await
        .unwrap();
    assert!(matches!(first, QuorumDecision::Accepted { .. }));

    // A second decision must not re-publish or double-strike (idempotent).
    let second = decide_rlm_quorum(&state, project, level, subject)
        .await
        .unwrap();
    assert!(
        matches!(second, QuorumDecision::Accepted { .. }),
        "second decision is idempotently Accepted"
    );
    assert_eq!(storage.volunteer_strikes("mallory").unwrap(), 1);
}

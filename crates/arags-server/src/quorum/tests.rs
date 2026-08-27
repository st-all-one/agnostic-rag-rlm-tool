#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use anyhow::Context;
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
        .enqueue_rlm_job(
            &NewRlmJob {
                buffer_id: Some(1),
                project: "proj".into(),
                level: 1,
                subject: "src/a.rs".into(),
                payload: "{}".into(),
                priority: 5,
                quorum_slots: 3,
            },
            &[],
        )
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
        .claim_rlm_job("alice", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    let b = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    let c = storage
        .claim_rlm_job("carol", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    let claimed: std::collections::HashSet<i64> = [a.id, b.id, c.id].into();
    assert_eq!(claimed.len(), 3, "three distinct slots claimed");
    assert_ne!(a.id, b.id);
    assert_ne!(b.id, c.id);

    // Alice cannot claim a second slot in the same group (one slot per group).
    assert!(
        storage
            .claim_rlm_job("alice", DEFAULT_RLM_LEASE_MS, None, 3)
            .unwrap()
            .is_none(),
        "a volunteer may hold at most one slot per group"
    );

    // Once all three are claimed, nothing remains claimable.
    assert!(
        storage
            .claim_rlm_job("dave", DEFAULT_RLM_LEASE_MS, None, 3)
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

#[tokio::test]
async fn total_divergence_triggers_reassignment_excluding_divergers() {
    let (_dir, storage, state) = fixture(3).await;
    // Register an extra non-diverging volunteer so the re-fan-out is not
    // declared exhausted by the "no non-banned volunteers remain" cap.
    storage.bump_trust_on_accept("dave").unwrap();
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
        matches!(decision, QuorumDecision::Rejected { .. }),
        "total divergence -> reject all"
    );

    // A fresh generation group was fanned out whose slots exclude the three
    // diverging volunteers.
    let exclusions: Vec<(i64, String)> = storage
        .connection()
        .unwrap()
        .execute(|c| {
            let mut stmt = c
                .prepare("SELECT generation_group_id, volunteer FROM rlm_job_exclusions")
                .context("prepare exclusions")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                .context("query exclusions")?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.context("map exclusion")?);
            }
            Ok(out)
        })
        .unwrap();
    assert!(!exclusions.is_empty(), "divergers must be excluded");
    let vols: std::collections::HashSet<&str> =
        exclusions.iter().map(|(_, v)| v.as_str()).collect();
    assert!(vols.contains("alice"));
    assert!(vols.contains("bob"));
    assert!(vols.contains("carol"));

    // New pending slots exist for the reassigned subject.
    assert!(
        !storage
            .get_live_rlm_job_by_key(project, level, subject)
            .unwrap()
            .is_none(),
        "reassigned generation group should have a live slot"
    );
}

#[tokio::test]
async fn total_divergence_is_capped_after_strikes_limit_rounds() {
    let (_dir, storage, mut state) = fixture(2).await;
    // Cap re-fan-outs at 2 rounds.
    state.config.quorum.strikes_limit = 2;
    // Extra non-diverging volunteers so the "no volunteers remain" cap does not
    // fire before the generation cap is reached (2 slots, 3 available).
    storage.bump_trust_on_accept("dave").unwrap();
    storage.bump_trust_on_accept("erin").unwrap();
    storage.bump_trust_on_accept("frank").unwrap();
    let project = "proj";
    let level = 1i64;
    let subject = "src/a.rs";
    let key = subject_key(project, level, subject);
    let workers = ["dave", "erin", "frank"];

    // Complete any pending slots from the previous round (a volunteer may hold
    // at most one slot per group, so rotate through the available workers).
    let complete_pending = |storage: &Storage, strikes_limit: u32| {
        let mut wi = 0usize;
        loop {
            let claimed = storage
                .claim_rlm_job(
                    workers[wi % workers.len()],
                    DEFAULT_RLM_LEASE_MS,
                    None,
                    strikes_limit,
                )
                .unwrap();
            match claimed {
                Some(j) => {
                    let who = workers[wi % workers.len()];
                    wi += 1;
                    assert!(storage.complete_rlm_job(j.id, who, j.generation).unwrap());
                }
                None => break,
            }
        }
    };

    // Several total-divergence rounds: each enqueues a fresh generation group
    // excluding the prior divergers, so the exclusion table grows by one group
    // per round until the generation cap is reached.
    for round in 0..4u32 {
        complete_pending(&storage, state.config.quorum.strikes_limit);
        storage
            .insert_submission(
                project,
                "rlm_node",
                &key,
                &format!("divergent answer number {round} from alice"),
                "alice",
            )
            .unwrap();
        storage
            .insert_submission(
                project,
                "rlm_node",
                &key,
                &format!("divergent answer number {round} from bob"),
                "bob",
            )
            .unwrap();
        let decision = decide_rlm_quorum(&state, project, level, subject)
            .await
            .unwrap();
        assert!(
            matches!(decision, QuorumDecision::Rejected { .. }),
            "round {round}: still rejected"
        );
    }

    // The generation cap (== strikes_limit) bounds the number of re-fan-outs:
    // we must NOT have created more exclusion groups than the cap allows.
    let groups: i64 = storage
        .connection()
        .unwrap()
        .execute(|c| {
            c.query_row(
                "SELECT COUNT(DISTINCT generation_group_id) FROM rlm_job_exclusions",
                [],
                |r| r.get(0),
            )
            .context("count exclusion groups")
        })
        .unwrap();
    assert!(
        groups <= i64::from(state.config.quorum.strikes_limit),
        "re-fan-outs must be capped at strikes_limit (got {groups} groups)"
    );
}

#[tokio::test]
async fn rlm_quorum_requires_2f_plus_1() {
    // n = 4 -> f = 1 -> need >= 3 mutually-agreeing candidates. We supply only
    // 2 agreeing + 2 divergent, so the 2f+1 bound is NOT met and the job must be
    // rejected (no consensus), not accepted with the 2-person clique.
    let (_dir, storage, state) = fixture(4).await;
    let project = "proj";
    let level = 1i64;
    let subject = "src/a.rs";
    let key = subject_key(project, level, subject);

    let agreeing = "Shared summary that two volunteers converged on.";
    let d1 = "First divergent and unrelated answer from a byzantine volunteer.";
    let d2 = "Second completely different answer that agrees with neither.";
    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "alice")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "bob")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, d1, "mallory")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, d2, "trent")
        .unwrap();

    let decision = decide_rlm_quorum(&state, project, level, subject)
        .await
        .unwrap();
    assert!(
        matches!(decision, QuorumDecision::Rejected { rejected_submission_ids } if rejected_submission_ids.len() == 4),
        "2 agreeing < 2f+1 (need 3) -> reject all 4"
    );

    // No node published, and every author took a strike.
    assert!(
        storage
            .get_rlm_node_by_subject(project, level, subject)
            .unwrap()
            .is_none(),
        "node must NOT be published when the 2f+1 bound is unmet"
    );
    assert_eq!(storage.volunteer_strikes("alice").unwrap(), 1);
    assert_eq!(storage.volunteer_strikes("bob").unwrap(), 1);
}

#[tokio::test]
async fn rlm_quorum_fusion_prefers_higher_trust() {
    // n = 3 -> f = 0 -> need >= 1 agreeing. Two volunteers submit the SAME
    // agreeing text but with different trust: alice (trust 1.0) vs bob (trust
    // lowered to 0.8 via a strike). mallory diverges. The published node must be
    // attributed to the higher-trust volunteer (alice).
    let (_dir, storage, state) = fixture(3).await;
    let project = "proj";
    let level = 1i64;
    let subject = "src/a.rs";
    let key = subject_key(project, level, subject);

    // Lower bob's trust so alice clearly outranks him.
    storage.record_strike("bob").unwrap();
    assert!(
        storage.read_trust("alice").unwrap().1 > storage.read_trust("bob").unwrap().1,
        "alice must outrank bob on trust"
    );

    let agreeing = "The module parses JSON configuration and exposes a typed API.";
    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "alice")
        .unwrap();
    storage
        .insert_submission(project, "rlm_node", &key, agreeing, "bob")
        .unwrap();
    storage
        .insert_submission(
            project,
            "rlm_node",
            &key,
            "Entirely unrelated content about rendering shaders.",
            "mallory",
        )
        .unwrap();

    let decision = decide_rlm_quorum(&state, project, level, subject)
        .await
        .unwrap();
    match decision {
        QuorumDecision::Accepted {
            fused_text,
            accepted_submission_ids,
            ..
        } => {
            assert_eq!(fused_text, agreeing, "consensus text is the agreeing text");
            assert_eq!(accepted_submission_ids.len(), 2, "two winners");
        }
        other => panic!("expected Accepted, got {other:?}"),
    }

    let node = storage
        .get_rlm_node_by_subject(project, level, subject)
        .unwrap()
        .unwrap();
    assert_eq!(node.summary_text, agreeing);
    assert_eq!(
        node.volunteer_username.as_deref(),
        Some("alice"),
        "published node attributed to the higher-trust volunteer"
    );
}

#[test]
fn choose_published_prefers_higher_trust() {
    // Direct unit check of the trust-weighted selection with synthetic vectors:
    // two agreeing candidates with near-identical similarity, but different trust
    // scores; the higher-trust member's text must be published.
    use std::collections::HashMap;

    use arags_storage::sqlite::submissions::Submission;

    let mk = |id: i64, by: &str, text: &str| Submission {
        id,
        project: "p".into(),
        subject_type: "rlm_node".into(),
        subject_key: "k".into(),
        candidate_text: text.into(),
        candidate_by: by.into(),
        similarity: None,
        status: "candidate".into(),
        created_at: 0,
        decided_at: None,
        decided_by: None,
    };
    let pending = vec![
        mk(1, "alice", "higher-trust answer"),
        mk(2, "bob", "lower-trust answer"),
    ];

    // Two near-identical (high cosine) vectors.
    let v0 = vec![1.0_f32, 0.0, 0.0];
    let v1 = vec![0.99_f32, 0.01, 0.0];
    let vectors = vec![v0, v1];

    let mut trust = HashMap::new();
    trust.insert("alice".to_string(), 1.0_f64);
    trust.insert("bob".to_string(), 0.5_f64);

    let cfg = crate::config::QuorumConfig {
        n: 3,
        quorum_sim_threshold: 0.85,
        fusion_strategy: crate::config::FusionStrategy::Consensus,
        strikes_limit: 3,
    };
    let agreeing = vec![0usize, 1usize];
    let idx = super::choose_published(&cfg, &agreeing, &pending, &vectors, &trust);
    assert_eq!(
        pending[idx].candidate_text, "higher-trust answer",
        "higher-trust volunteer's text is published"
    );

    // Reversed trust: bob now outranks alice -> bob's text is published.
    let mut trust2 = HashMap::new();
    trust2.insert("alice".to_string(), 0.2_f64);
    trust2.insert("bob".to_string(), 1.0_f64);
    let idx2 = super::choose_published(&cfg, &agreeing, &pending, &vectors, &trust2);
    assert_eq!(
        pending[idx2].candidate_text, "lower-trust answer",
        "when bob outranks alice, bob's text is published"
    );
}

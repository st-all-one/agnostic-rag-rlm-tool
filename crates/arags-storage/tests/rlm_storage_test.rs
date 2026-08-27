//! Behavioral tests for the RLM storage layer (nodes, jobs, graph).
//!
//! Exercises only the public `arags_storage` API against a throwaway
//! SQLite-style database (see `Storage::open`): upsert and review gate semantics,
//! lease claiming with generation checks, atomic completion, staleness
//! propagation and the parent chain CTE.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]

use anyhow::Context;
use arags_storage::Storage;
use arags_storage::sqlite::rlm::{
    DEFAULT_RLM_LEASE_MS, NewRlmJob, NewRlmNode, REVIEW_APPROVED, REVIEW_PENDING, RlmJobPayload,
    rlm_job_key,
};
use arags_storage::sqlite::tokens::now_ms;
use rusqlite::params;

fn temp_storage() -> Storage {
    let dir = tempfile::tempdir().unwrap();
    Storage::open(dir.path()).unwrap()
}

fn node(project: &str, level: i64, subject: &str, hashes: &[&str]) -> NewRlmNode {
    NewRlmNode {
        buffer_id: Some(1),
        project: project.into(),
        level,
        subject: subject.into(),
        summary_text: format!("summary of {subject}"),
        source_hashes: hashes.iter().map(|h| (*h).to_string()).collect(),
        model: Some("llama3.2".into()),
        volunteer_username: Some("alice".into()),
        created_by: None,
        template_version: Some("v1".into()),
        token_count: 42,
    }
}

fn job(project: &str, level: i64, subject: &str) -> NewRlmJob {
    NewRlmJob {
        buffer_id: Some(1),
        project: project.into(),
        level,
        subject: subject.into(),
        payload: "{}".into(),
        priority: 5,
        quorum_slots: 1,
    }
}

#[test]
fn store_node_supersede_resets_review_gate() {
    let storage = temp_storage();
    let (id1, nid1) = storage
        .store_rlm_node(&node("p", 1, "src/main.rs", &["h1"]))
        .unwrap();
    assert!(!nid1.is_empty());

    let n = storage.get_rlm_node(&nid1).unwrap().unwrap();
    assert_eq!(n.review_status, REVIEW_PENDING);
    assert_eq!(n.level, 1);

    // Approve, then resubmit: a NEW row supersedes the old one; its review gate
    // resets to pending while the previous revision keeps its verdict.
    assert!(storage.review_rlm_node(&nid1, true, "admin", None).unwrap());
    let (id2, nid2) = storage
        .store_rlm_node(&node("p", 1, "src/main.rs", &["h2"]))
        .unwrap();
    assert_ne!(id1, id2);
    assert_ne!(nid1, nid2);

    // The active revision for the subject is the new row.
    let active = storage
        .get_rlm_node_by_subject("p", 1, "src/main.rs")
        .unwrap()
        .unwrap();
    assert_eq!(active.id, id2);
    assert_eq!(active.review_status, REVIEW_PENDING);
    assert_eq!(active.source_hashes, vec!["h2".to_string()]);
    assert!(!active.stale);

    // The previous revision is retired and links forward to the new row, but
    // retains its own (approved) review verdict.
    let old = storage.get_rlm_node(&nid1).unwrap().unwrap();
    assert_eq!(old.is_active, false);
    assert_eq!(old.superseded_by, Some(id2));
    assert_eq!(old.review_status, REVIEW_APPROVED);
    assert_eq!(old.source_hashes, vec!["h1".to_string()]);
}

#[test]
fn list_nodes_filters_by_review_and_level() {
    let storage = temp_storage();
    let (_, l3) = storage.store_rlm_node(&node("p", 3, "p", &[])).unwrap();
    storage.review_rlm_node(&l3, true, "admin", None).unwrap();
    let _ = storage.store_rlm_node(&node("p", 1, "a.rs", &[])).unwrap();

    let approved = storage.list_rlm_nodes("p", None, false).unwrap();
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].level, 3);

    let all = storage.list_rlm_nodes("p", Some(1), true).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].subject, "a.rs");

    assert!(
        storage
            .list_rlm_nodes("other", None, true)
            .unwrap()
            .is_empty()
    );
}

/// Regression (agnostic-rlm-rs-0764): hydration of vector-search hits must be
/// scoped by buffer so other projects' summaries never surface in results.
#[test]
fn approved_node_hydration_is_scoped_by_buffer() {
    let storage = temp_storage();

    let mut other = node("other", 1, "src/auth.rs", &[]);
    other.buffer_id = Some(2);
    let (rowid_other, nid_other) = storage.store_rlm_node(&other).unwrap();
    assert!(
        storage
            .review_rlm_node(&nid_other, true, "admin", None)
            .unwrap()
    );

    let (rowid_own, nid_own) = storage
        .store_rlm_node(&node("p", 1, "src/auth.rs", &[]))
        .unwrap();
    assert!(
        storage
            .review_rlm_node(&nid_own, true, "admin", None)
            .unwrap()
    );

    // Vector search is global: both rowids come back as candidates, but only
    // the caller's buffer may hydrate.
    let ids = [
        u64::try_from(rowid_own).unwrap(),
        u64::try_from(rowid_other).unwrap(),
    ];
    let scoped = storage.get_approved_rlm_nodes(&ids, 1).unwrap();
    assert_eq!(scoped.len(), 1, "only same-buffer nodes may hydrate");
    assert_eq!(scoped[0].project, "p");

    let other_side = storage.get_approved_rlm_nodes(&ids, 2).unwrap();
    assert_eq!(other_side.len(), 1, "the other buffer sees its own node");
    assert_eq!(other_side[0].project, "other");
}

#[test]
fn edges_and_parent_chain_walk_up() {
    let storage = temp_storage();
    let (l1_id, _) = storage.store_rlm_node(&node("p", 1, "a.rs", &[])).unwrap();
    let (l2_id, _) = storage.store_rlm_node(&node("p", 2, "core", &[])).unwrap();
    let (l3_id, _) = storage.store_rlm_node(&node("p", 3, "p", &[])).unwrap();
    storage.add_rlm_edge(l2_id, Some(l1_id), None).unwrap();
    storage.add_rlm_edge(l3_id, Some(l2_id), None).unwrap();

    // Exactly-one-reference guard.
    assert!(storage.add_rlm_edge(l1_id, None, None).is_err());

    let chain = storage.rlm_parent_chain(&[l1_id]).unwrap();
    assert_eq!(chain, vec![l2_id, l3_id]);
}

#[test]
fn staleness_marks_affected_nodes_with_hashes() {
    let storage = temp_storage();
    let (_, nid) = storage
        .store_rlm_node(&node("p", 1, "a.rs", &["h1", "h2"]))
        .unwrap();
    let affected = storage
        .mark_rlm_stale_by_hashes(1, &["zzz".to_string()])
        .unwrap();
    assert!(affected.is_empty());
    let affected = storage
        .mark_rlm_stale_by_hashes(1, &["h2".to_string()])
        .unwrap();
    assert_eq!(affected.len(), 1);
    let n = storage.get_rlm_node(&nid).unwrap().unwrap();
    assert!(n.stale);
    assert_eq!(n.confidence, 0.0);
}

#[test]
fn enqueue_is_idempotent_for_pending_and_resets_finished() {
    let storage = temp_storage();
    let (id1, gen1) = storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    assert_eq!(gen1, 0);
    let (id2, _) = storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    assert_eq!(id1, id2);

    // Claim then finish; a new enqueue bumps generation and re-opens it.
    let claimed = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.subject, "a.rs");
    assert!(
        storage
            .complete_rlm_job(claimed.id, "bob", claimed.generation)
            .unwrap()
    );
    assert_eq!(storage.count_rlm_jobs("p", "done").unwrap(), 1);

    let (_, gen3) = storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    assert_eq!(gen3, 1);
    assert_eq!(storage.count_rlm_jobs("p", "pending").unwrap(), 1);
}

#[test]
fn claim_locks_work_unit_until_completion() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();

    let first = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    // While the lease is live no other volunteer can claim the same unit.
    assert!(
        storage
            .claim_rlm_job("carol", DEFAULT_RLM_LEASE_MS, None, 3)
            .unwrap()
            .is_none()
    );

    // Wrong worker or generation is rejected.
    assert!(
        !storage
            .complete_rlm_job(first.id, "carol", first.generation)
            .unwrap()
    );
    assert!(
        !storage
            .complete_rlm_job(first.id, "bob", first.generation + 7)
            .unwrap()
    );
    assert!(
        storage
            .complete_rlm_job(first.id, "bob", first.generation)
            .unwrap()
    );
}

#[test]
fn expired_lease_requeues() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    let _ = storage
        .claim_rlm_job("bob", 1_000, None, 3)
        .unwrap()
        .unwrap();
    // Simulate lease expiry by backdating it to epoch.
    let conn = storage.connection().unwrap();
    conn.execute(|c| Ok(c.execute("UPDATE rlm_jobs SET lease_expires_at = 1", [])?))
        .unwrap();
    drop(conn);

    assert_eq!(storage.requeue_expired_rlm_leases().unwrap(), 1);
    assert_eq!(storage.count_rlm_jobs("p", "pending").unwrap(), 1);
}

#[test]
fn cancel_bumps_generation_and_elevates_priority() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    let claimed = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();

    let n = storage
        .cancel_rlm_jobs_for_subjects("p", &[(1, "a.rs".into())])
        .unwrap();
    assert_eq!(n, 1);

    // Old lease completion is rejected (generation mismatch).
    assert!(
        !storage
            .complete_rlm_job(claimed.id, "bob", claimed.generation)
            .unwrap()
    );
    // Job is back at the front of the queue for reprocessing.
    let next = storage
        .claim_rlm_job("carol", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    assert_eq!(next.id, claimed.id);
    assert_eq!(next.generation, claimed.generation + 1);
}

#[test]
fn fail_returns_to_pending_then_parks_after_max_attempts() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    let j1 = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    storage
        .fail_rlm_job(j1.id, "bob", "llm timeout", 3)
        .unwrap();
    assert_eq!(storage.count_rlm_jobs("p", "pending").unwrap(), 1);

    for worker in ["c1", "c2"] {
        let j = storage
            .claim_rlm_job(worker, DEFAULT_RLM_LEASE_MS, None, 3)
            .unwrap()
            .unwrap();
        storage
            .fail_rlm_job(j.id, worker, "llm timeout", 3)
            .unwrap();
    }
    assert_eq!(storage.count_rlm_jobs("p", "failed").unwrap(), 1);
}

#[test]
fn max_level_filter_limits_claims() {
    let storage = temp_storage();
    storage
        .enqueue_rlm_job(&job("p", 3, "p-overview"), &[])
        .unwrap();
    // Volunteer that only accepts L1/L2 gets nothing.
    assert!(
        storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, Some(2), 3)
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, Some(3), 3)
            .unwrap()
            .is_some()
    );
}

#[test]
fn job_key_is_deterministic_per_level_subject() {
    assert_eq!(
        rlm_job_key("proj", 2, "core"),
        rlm_job_key("proj", 2, "core")
    );
    assert_ne!(
        rlm_job_key("proj", 1, "core"),
        rlm_job_key("proj", 2, "core")
    );
}

fn completion_input(subject: &str) -> NewRlmNode {
    NewRlmNode {
        buffer_id: Some(1),
        project: "p".into(),
        level: 1,
        subject: subject.into(),
        summary_text: format!("summary of {subject}"),
        source_hashes: vec!["h1".into()],
        model: Some("llama3.2".into()),
        volunteer_username: Some("bob".into()),
        created_by: None,
        template_version: None,
        token_count: 7,
    }
}

#[test]
fn complete_with_node_persists_both_atomically() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    let claimed = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();

    let (rowid, node_id) = storage
        .complete_rlm_job_with_node(
            claimed.id,
            "bob",
            claimed.generation,
            &completion_input("a.rs"),
        )
        .unwrap()
        .unwrap();

    assert!(rowid > 0);
    assert_eq!(storage.count_rlm_jobs("p", "done").unwrap(), 1);
    let n = storage.get_rlm_node(&node_id).unwrap().unwrap();
    assert_eq!(n.subject, "a.rs");
    assert_eq!(n.source_hashes, vec!["h1".to_string()]);
    // Review gate starts pending even on the atomic path.
    assert_eq!(n.review_status, REVIEW_PENDING);
}

#[test]
fn complete_with_node_persists_created_by_and_model() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    let claimed = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();

    let mut node = completion_input("a.rs");
    node.created_by = Some("bob".into());
    node.model = Some("llama3.2".into());

    let (rowid, _node_id) = storage
        .complete_rlm_job_with_node(claimed.id, "bob", claimed.generation, &node)
        .unwrap()
        .unwrap();
    assert!(rowid > 0);

    let (cb, m): (Option<String>, Option<String>) = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT created_by, model FROM rlm_nodes WHERE id = ?1",
                rusqlite::params![rowid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .context("read rlm_nodes authorship")
        })
        .unwrap();
    assert_eq!(cb.as_deref(), Some("bob"), "created_by populated");
    assert_eq!(m.as_deref(), Some("llama3.2"), "model populated");
}

#[test]
fn complete_with_node_rejects_stale_generation_without_side_effects() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();
    let claimed = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();

    let outcome = storage
        .complete_rlm_job_with_node(
            claimed.id,
            "bob",
            claimed.generation + 1,
            &completion_input("a.rs"),
        )
        .unwrap();
    assert!(outcome.is_none());

    // Nothing persisted: no phantom node, job still claimable.
    assert!(
        storage
            .get_rlm_node_by_subject("p", 1, "a.rs")
            .unwrap()
            .is_none()
    );
    assert_eq!(storage.count_rlm_jobs("p", "done").unwrap(), 0);

    // Correct generation still completes after a rejected attempt.
    let retry = storage
        .complete_rlm_job_with_node(
            claimed.id,
            "bob",
            claimed.generation,
            &completion_input("a.rs"),
        )
        .unwrap();
    assert!(retry.is_some());
}

#[test]
fn complete_with_node_rejects_wrong_worker_without_side_effects() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "b.rs"), &[]).unwrap();
    let claimed = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();

    let outcome = storage
        .complete_rlm_job_with_node(
            claimed.id,
            "mallory",
            claimed.generation,
            &completion_input("b.rs"),
        )
        .unwrap();
    assert!(outcome.is_none());
    assert_eq!(storage.count_rlm_jobs("p", "claimed").unwrap(), 1);
}

#[test]
fn shared_payload_type_round_trips_through_job_queue() {
    // The payload column is opaque JSON for storage; ensure a full payload
    // written via the queue survives a deserialize on the volunteer side.
    let storage = temp_storage();
    let payload = RlmJobPayload {
        chunk_ids: vec![10],
        hashes: vec!["h".into()],
        texts: vec!["t".into()],
        template_version: "v1".into(),
        subject_kind: "file".into(),
        ..RlmJobPayload::default()
    };
    let json = serde_json::to_string(&payload).unwrap();
    storage
        .enqueue_rlm_job(
            &NewRlmJob {
                buffer_id: Some(1),
                project: "p".into(),
                level: 1,
                subject: "a.rs".into(),
                payload: json,
                priority: 5,
                quorum_slots: 1,
            },
            &[],
        )
        .unwrap();
    let claimed = storage
        .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    let back: RlmJobPayload = serde_json::from_str(&claimed.payload).unwrap();
    assert_eq!(back.chunk_ids, vec![10]);
    assert_eq!(back.template_version, "v1");
    assert_eq!(back.subject_kind, "file");
    let _ = now_ms(); // keep import honest for future tests
}

#[test]
fn supersede_rlm_node_creates_new_active_row_and_history() {
    let storage = temp_storage();
    let mut v1 = node("p", 1, "src/main.rs", &["h1"]);
    v1.summary_text = "first draft".into();
    let (id1, nid1) = storage.store_rlm_node(&v1).unwrap();

    let mut v2 = node("p", 1, "src/main.rs", &["h2"]);
    v2.summary_text = "second draft".into();
    let (id2, nid2) = storage.store_rlm_node(&v2).unwrap();

    let mut v3 = node("p", 1, "src/main.rs", &["h3"]);
    v3.summary_text = "third draft".into();
    let (id3, _nid3) = storage.store_rlm_node(&v3).unwrap();

    // (a) exactly one ACTIVE node for the subject; the two earlier ones retired.
    let active_count: i64 = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM rlm_nodes WHERE project = 'p' AND level = 1 \
                 AND subject = 'src/main.rs' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .context("count active rlm_nodes")
        })
        .unwrap();
    assert_eq!(active_count, 1);

    let old = storage.get_rlm_node(&nid1).unwrap().unwrap();
    assert_eq!(old.is_active, false);
    assert_eq!(old.superseded_by, Some(id2));
    let mid = storage.get_rlm_node(&nid2).unwrap().unwrap();
    assert_eq!(mid.is_active, false);
    assert_eq!(mid.superseded_by, Some(id3));

    // (b) the subject read returns only the latest active node.
    let active = storage
        .get_rlm_node_by_subject("p", 1, "src/main.rs")
        .unwrap()
        .unwrap();
    assert_eq!(active.id, id3);
    assert_eq!(active.summary_text, "third draft");

    // (c) the history getter walks the full chain oldest -> newest.
    let history = storage.get_node_history(id1).unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].id, id1);
    assert_eq!(history[0].summary_text, "first draft");
    assert_eq!(history[1].id, id2);
    assert_eq!(history[2].id, id3);
    assert_eq!(history[2].summary_text, "third draft");
}

#[test]
fn banned_volunteer_claim_is_rejected() {
    let storage = temp_storage();
    storage.enqueue_rlm_job(&job("p", 1, "a.rs"), &[]).unwrap();

    // Push alice to the ban threshold (3 strikes).
    storage.record_strike("alice").unwrap();
    storage.record_strike("alice").unwrap();
    storage.record_strike("alice").unwrap();
    assert!(storage.is_banned("alice", 3).unwrap());

    // A banned volunteer cannot claim the pending slot...
    assert!(
        storage
            .claim_rlm_job("alice", DEFAULT_RLM_LEASE_MS, None, 3)
            .unwrap()
            .is_none(),
        "banned volunteer must not claim"
    );
    // ...but an un-banned one can.
    assert!(
        storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
            .unwrap()
            .is_some(),
        "non-banned volunteer may claim"
    );
}

#[test]
fn diverger_is_excluded_from_reassigned_generation_group() {
    let storage = temp_storage();
    let spec = NewRlmJob {
        buffer_id: Some(1),
        project: "p".into(),
        level: 1,
        subject: "a.rs".into(),
        payload: "{}".into(),
        priority: 5,
        quorum_slots: 2,
    };

    // Original fan-out (no exclusions).
    let (_, gen0) = storage.enqueue_rlm_job(&spec, &[]).unwrap();

    // Volunteers claim and finish both slots so the group is fully done.
    let j1 = storage
        .claim_rlm_job("w1", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    let j2 = storage
        .claim_rlm_job("w2", DEFAULT_RLM_LEASE_MS, None, 3)
        .unwrap()
        .unwrap();
    assert!(
        storage
            .complete_rlm_job(j1.id, "w1", j1.generation)
            .unwrap()
    );
    assert!(
        storage
            .complete_rlm_job(j2.id, "w2", j2.generation)
            .unwrap()
    );

    // Re-fan-out after divergence, excluding two volunteers.
    let (_, gen1) = storage
        .enqueue_rlm_job(&spec, &["alice".into(), "bob".into()])
        .unwrap();
    assert!(gen1 > gen0, "re-fan-out advances the generation");

    // The new slots belong to a fresh generation group that records the
    // divergers as excluded.
    let new_group: i64 = storage
        .connection()
        .unwrap()
        .execute(|c| {
            c.query_row(
                "SELECT generation_group_id FROM rlm_jobs \
                 WHERE project = 'p' AND level = 1 AND subject = 'a.rs' \
                 ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .context("read new group id")
        })
        .unwrap();
    let excluded: Vec<String> = storage
        .connection()
        .unwrap()
        .execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT volunteer FROM rlm_job_exclusions \
                     WHERE generation_group_id = ?1",
                )
                .context("prepare exclusions")?;
            let rows = stmt
                .query_map(params![new_group], |r| r.get(0))
                .context("query exclusions")?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.context("map exclusion")?);
            }
            Ok(out)
        })
        .unwrap();
    assert!(excluded.contains(&"alice".to_string()));
    assert!(excluded.contains(&"bob".to_string()));

    // An excluded diverger cannot claim the new slots.
    assert!(
        storage
            .claim_rlm_job("alice", DEFAULT_RLM_LEASE_MS, None, 3)
            .unwrap()
            .is_none(),
        "excluded diverger must not claim the new group"
    );
}

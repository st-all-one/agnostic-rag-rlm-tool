//! Behavioral tests for the explorations dataset (plan 022), backed by
//! `SQLite` via `Storage`: persistence with anchors in one transaction, FTS
//! search, epoch drift, anchor-based staleness and the feedback loop.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use anyhow::Context;
use arags_storage::Storage;
use arags_storage::explorations::{
    ExplorationAnchor, FeedbackKind, PersistExplorationInput, ROLE_CITED, ROLE_CONTEXT,
    STATUS_FRESH, STATUS_STALE,
};

fn temp_storage() -> Storage {
    let dir = tempfile::tempdir().unwrap();
    Storage::open(dir.path()).unwrap()
}

fn anchor(path: &str, hash: &str) -> ExplorationAnchor {
    ExplorationAnchor {
        buffer_id: 1,
        path: path.into(),
        // `chunks.hash` is stored as raw bytes; the read-time recheck compares
        // `lower(hex(ch.hash)) = lower(content_hash)`, so the anchor hash must be
        // the hex encoding of the same bytes that `seed_chunks` stores as `hash.as_bytes()`.
        content_hash: hex::encode(hash.as_bytes()),
        role: ROLE_CITED.into(),
    }
}

fn input(project: &str, goal: &str, anchors: Vec<ExplorationAnchor>) -> PersistExplorationInput {
    PersistExplorationInput {
        project: project.into(),
        buffer_id: Some(1),
        goal: goal.into(),
        body_markdown: format!("# Mapa\n\n## Conexões\n{goal}"),
        summary: format!("resumo de {goal}"),
        anchors,
        created_by: "agent-1".into(),
        model: Some("qwen2.5-coder:7b".into()),
        template_version: String::new(),
        token_count: 42,
    }
}

fn seed_chunks(storage: &Storage, buffer_id: i64, rows: &[(&str, &str)]) {
    // Simulates a reindex run: existing chunks for each path are replaced.
    storage
        .connection()
        .unwrap()
        .execute(|c| {
            c.execute(
                "INSERT INTO buffers (id, name, path) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(id) DO NOTHING",
                rusqlite::params![buffer_id, format!("proj-{buffer_id}"), "/tmp/proj"],
            )?;
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

#[test]
fn test_persist_and_get_roundtrip_decompresses_body() {
    let storage = temp_storage();
    let stored = storage
        .persist_exploration(&input("p1", "anexos", vec![anchor("src/a.rs", "h1")]))
        .unwrap();

    let row = storage
        .get_exploration_by_uuid(&stored.exploration_id)
        .unwrap()
        .expect("row exists");
    assert_eq!(row.id, stored.id);
    assert_eq!(row.project, "p1");
    assert_eq!(row.status, STATUS_FRESH);
    assert_eq!(row.created_by, "agent-1");
    assert_eq!(
        row.template_version,
        arags_storage::explorations::TEMPLATE_VERSION_V1
    );
    assert!(
        row.body.contains("## Conexões"),
        "body decompressed: {}",
        row.body
    );
    assert_eq!(row.token_count, 42);
    assert!(row.stale_reason.is_empty());

    let by_rowid = storage.get_exploration_by_rowid(row.id).unwrap().unwrap();
    assert_eq!(by_rowid.exploration_id, row.exploration_id);
}

#[test]
fn test_persist_is_atomic_row_and_anchors_together() {
    let storage = temp_storage();
    let stored = storage
        .persist_exploration(&input(
            "p1",
            "conexão x->y",
            vec![anchor("src/a.rs", "h1"), anchor("src/b.rs", "h2")],
        ))
        .unwrap();

    let anchors = storage.list_exploration_anchors(stored.id).unwrap();
    assert_eq!(anchors.len(), 2);
    assert!(anchors.iter().all(|(buf, _, _)| *buf == 1));

    // Deleting the map removes its anchors (cascade).
    storage.delete_exploration(stored.id).unwrap();
    assert!(
        storage
            .get_exploration_by_rowid(stored.id)
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .list_exploration_anchors(stored.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_fts_search_scoped_and_ranked() {
    let storage = temp_storage();
    storage
        .persist_exploration(&input(
            "p1",
            "anexos compartilhados entre licitações e publicações",
            vec![],
        ))
        .unwrap();
    storage
        .persist_exploration(&input("p1", "fluxo de autenticação middleware", vec![]))
        .unwrap();
    storage
        .persist_exploration(&input("outro", "anexos em outro projeto", vec![]))
        .unwrap();

    let hits = storage
        .search_explorations_fts("p1", "anexos publicações", 10)
        .unwrap();
    assert_eq!(hits.len(), 1, "scoped to project p1");
    assert!(hits[0].goal.contains("anexos"));

    assert!(
        storage
            .search_explorations_fts("p1", "", 10)
            .unwrap()
            .is_empty(),
        "empty query returns no hits"
    );
}

#[test]
fn test_epoch_bump_is_monotone_per_project() {
    let storage = temp_storage();
    assert_eq!(storage.current_project_epoch("p1").unwrap(), 0);
    assert_eq!(storage.bump_project_epoch("p1").unwrap(), 1);
    assert_eq!(storage.bump_project_epoch("p1").unwrap(), 2);
    assert_eq!(storage.current_project_epoch("other").unwrap(), 0);

    // Persisted maps stamp the current epoch.
    let stored = storage
        .persist_exploration(&input("p1", "após bumps", vec![]))
        .unwrap();
    let row = storage
        .get_exploration_by_rowid(stored.id)
        .unwrap()
        .unwrap();
    assert_eq!(row.epoch_created, 2);
}

#[test]
fn test_staleness_marks_map_when_cited_anchor_changes() {
    let storage = temp_storage();
    seed_chunks(&storage, 1, &[("src/a.rs", "h-old"), ("src/b.rs", "h-ok")]);

    let stored = storage
        .persist_exploration(&input(
            "p1",
            "mapa com âncoras",
            vec![anchor("src/a.rs", "h-old"), anchor("src/b.rs", "h-ok")],
        ))
        .unwrap();
    assert!(
        storage
            .recheck_anchors_for_rowid(stored.id)
            .unwrap()
            .is_empty(),
        "anchors hold right after persist"
    );

    // Index rewrites src/a.rs with a new hash.
    storage.bump_project_epoch("p1").unwrap();
    seed_chunks(&storage, 1, &[("src/a.rs", "h-new")]);

    assert_eq!(storage.mark_stale_if_anchors_changed("p1").unwrap(), 1);
    let row = storage
        .get_exploration_by_rowid(stored.id)
        .unwrap()
        .unwrap();
    assert_eq!(row.status, STATUS_STALE);
    assert_eq!(row.stale_reason, vec!["src/a.rs".to_string()]);
    assert_eq!(
        storage.recheck_anchors_for_rowid(stored.id).unwrap(),
        vec!["src/a.rs".to_string()]
    );

    // Idempotent: second run invalidates nothing new.
    assert_eq!(storage.mark_stale_if_anchors_changed("p1").unwrap(), 0);
}

#[test]
fn test_deleted_file_counts_as_broken_anchor() {
    let storage = temp_storage();
    seed_chunks(&storage, 1, &[("src/gone.rs", "h1")]);
    let stored = storage
        .persist_exploration(&input(
            "p1",
            "arquivo removido depois",
            vec![anchor("src/gone.rs", "h1")],
        ))
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(|c| {
            c.execute("DELETE FROM chunks WHERE file_path = 'src/gone.rs'", [])?;
            Ok(())
        })
        .unwrap();

    assert_eq!(storage.mark_stale_if_anchors_changed("p1").unwrap(), 1);
    assert_eq!(
        storage
            .get_exploration_by_rowid(stored.id)
            .unwrap()
            .unwrap()
            .status,
        STATUS_STALE
    );
}

#[test]
fn test_context_role_does_not_invalidate_but_cited_does() {
    let storage = temp_storage();
    seed_chunks(&storage, 1, &[("src/cited.rs", "h1"), ("src/ctx.rs", "h1")]);
    let stored = storage
        .persist_exploration(&input(
            "p1",
            "papéis distintos",
            vec![
                ExplorationAnchor {
                    buffer_id: 1,
                    path: "src/cited.rs".into(),
                    content_hash: hex::encode("h1".as_bytes()),
                    role: ROLE_CITED.into(),
                },
                ExplorationAnchor {
                    buffer_id: 1,
                    path: "src/ctx.rs".into(),
                    content_hash: hex::encode("h1".as_bytes()),
                    role: ROLE_CONTEXT.into(),
                },
            ],
        ))
        .unwrap();

    // Only the context anchor breaks: map stays fresh...
    seed_chunks(&storage, 1, &[("src/ctx.rs", "h2")]);
    assert_eq!(storage.mark_stale_if_anchors_changed("p1").unwrap(), 0);
    assert_eq!(
        storage
            .get_exploration_by_rowid(stored.id)
            .unwrap()
            .unwrap()
            .status,
        STATUS_FRESH
    );
    // ...but read-time recheck reports only cited breakage; now break the cited one.
    seed_chunks(&storage, 1, &[("src/cited.rs", "h2")]);
    assert_eq!(storage.mark_stale_if_anchors_changed("p1").unwrap(), 1);
}

#[test]
fn test_feedback_confirm_contradict_and_auto_retire() {
    let storage = temp_storage();
    let stored = storage
        .persist_exploration(&input("p1", "feedback", vec![]))
        .unwrap();

    let out = storage
        .record_feedback(&stored.exploration_id, FeedbackKind::Confirm, 2)
        .unwrap()
        .expect("map exists");
    assert_eq!(
        out,
        arags_storage::explorations::FeedbackOutcome::Confirmed { confirmed: 1 }
    );

    let out = storage
        .record_feedback(&stored.exploration_id, FeedbackKind::Contradict, 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        out,
        arags_storage::explorations::FeedbackOutcome::Contradicted {
            contradicted: 1,
            auto_retired: false
        }
    );
    assert_eq!(
        storage
            .get_exploration_by_uuid(&stored.exploration_id)
            .unwrap()
            .unwrap()
            .status,
        STATUS_FRESH
    );

    // Second contradiction crosses the limit of 2 → retired.
    let out = storage
        .record_feedback(&stored.exploration_id, FeedbackKind::Contradict, 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        out,
        arags_storage::explorations::FeedbackOutcome::Contradicted {
            contradicted: 2,
            auto_retired: true
        }
    );
    let row = storage
        .get_exploration_by_uuid(&stored.exploration_id)
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "retired");

    // Retired maps no longer accept contradictions (counter frozen).
    let out = storage
        .record_feedback(&stored.exploration_id, FeedbackKind::Contradict, 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        out,
        arags_storage::explorations::FeedbackOutcome::Contradicted {
            contradicted: 2,
            auto_retired: false
        }
    );

    assert!(
        storage
            .record_feedback("no-such-id", FeedbackKind::Confirm, 2)
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_admin_soft_invalidation_records_audit() {
    let storage = temp_storage();
    let stored = storage
        .persist_exploration(&input("p1", "invalidação admin", vec![]))
        .unwrap();

    assert!(
        storage
            .invalidate_exploration_stale(&stored.exploration_id, "admin-1", "revisão manual")
            .unwrap()
    );
    let row = storage
        .get_exploration_by_uuid(&stored.exploration_id)
        .unwrap()
        .unwrap();
    assert_eq!(row.status, STATUS_STALE);
    assert!(row.stale_reason.contains(&"revisão manual".to_string()));

    // Second soft invalidation on a non-fresh map is a no-op returning false.
    assert!(
        !storage
            .invalidate_exploration_stale(&stored.exploration_id, "admin-1", "de novo")
            .unwrap()
    );
}

#[test]
fn test_touch_and_count() {
    let storage = temp_storage();
    let stored = storage
        .persist_exploration(&input("p1", "contadores", vec![]))
        .unwrap();
    assert_eq!(storage.count_explorations("p1", None).unwrap(), 1);
    assert_eq!(
        storage
            .count_explorations("p1", Some(STATUS_FRESH))
            .unwrap(),
        1
    );
    assert_eq!(
        storage
            .count_explorations("p1", Some(STATUS_STALE))
            .unwrap(),
        0
    );
    assert_eq!(storage.count_explorations("missing", None).unwrap(), 0);

    storage.touch_exploration(stored.id).unwrap();
    storage.touch_exploration(stored.id).unwrap();
    assert_eq!(
        storage
            .get_exploration_by_rowid(stored.id)
            .unwrap()
            .unwrap()
            .access_count,
        2
    );
}

#[test]
fn supersede_exploration_creates_new_active_row_and_history() {
    let storage = temp_storage();
    let v1 = storage
        .persist_exploration(&input("p1", "mesmo objetivo", vec![]))
        .unwrap();
    let v2 = storage
        .persist_exploration(&input("p1", "mesmo objetivo", vec![]))
        .unwrap();
    let v3 = storage
        .persist_exploration(&input("p1", "mesmo objetivo", vec![]))
        .unwrap();

    // (a) exactly one ACTIVE map for the goal; the two earlier ones retired.
    let active_count: i64 = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM explorations WHERE project = 'p1' \
                 AND goal = 'mesmo objetivo' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .context("count active explorations")
        })
        .unwrap();
    assert_eq!(active_count, 1);

    let old = storage.get_exploration_by_rowid(v1.id).unwrap().unwrap();
    assert_eq!(old.is_active, false);
    assert_eq!(old.superseded_by, Some(v2.id));
    let mid = storage.get_exploration_by_rowid(v2.id).unwrap().unwrap();
    assert_eq!(mid.is_active, false);
    assert_eq!(mid.superseded_by, Some(v3.id));

    // (b) the FTS read returns only the latest active map.
    let hits = storage
        .search_explorations_fts("p1", "mesmo objetivo", 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, v3.id);

    // (c) the history getter walks the full chain oldest -> newest.
    let history = storage.get_exploration_history(v1.id).unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].id, v1.id);
    assert_eq!(history[1].id, v2.id);
    assert_eq!(history[2].id, v3.id);
    assert_eq!(history[2].is_active, true);
}

// Regression for agnostic-rag-rlm-tool-8007: a freshly persisted map whose cited
// anchor stores a REAL 32-byte (binary, non-UTF8) content hash must NOT be
// classified stale. `chunks.hash` holds the raw digest bytes; the anchor's
// `content_hash` is the hex of those bytes. The read-time recheck compares
// `lower(hex(ch.hash)) = lower(content_hash)`, which holds — so `recheck`
// returns empty and the map surfaces in default search.
#[test]
fn test_staleness_recheck_holds_with_binary_digest() {
    let storage = temp_storage();

    let digest: Vec<u8> = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"fn main() { let x = 1; }");
        h.finalize().to_vec()
    };
    let digest_hex = hex::encode(&digest);

    storage
        .connection()
        .unwrap()
        .execute(|c| {
            c.execute(
                "INSERT INTO buffers (id, name, path) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(id) DO NOTHING",
                rusqlite::params![1, "proj-1", "/tmp/proj"],
            )?;
            c.execute(
                "DELETE FROM chunks WHERE buffer_id = ?1 AND file_path = ?2",
                rusqlite::params![1, "src/a.rs"],
            )?;
            c.execute(
                "INSERT INTO chunks \
                  (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash) \
                  VALUES (?1, ?2, 0, 1, 1, 1, ?3)",
                rusqlite::params![1, "src/a.rs", digest.as_slice()],
            )?;
            Ok(())
        })
        .unwrap();

    let stored = storage
        .persist_exploration(&input(
            "p1",
            "mapa com âncora binária",
            vec![ExplorationAnchor {
                buffer_id: 1,
                path: "src/a.rs".into(),
                content_hash: digest_hex,
                role: ROLE_CITED.into(),
            }],
        ))
        .unwrap();

    assert!(
        storage
            .recheck_anchors_for_rowid(stored.id)
            .unwrap()
            .is_empty(),
        "binary-digest anchor must hold immediately after persist"
    );
}

//! Audit-log storage API tests (issue `agnostic-rlm-rs-7222`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arags_storage::Storage;
use arags_storage::sqlite::audit::AuditEntry;

fn open() -> Storage {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open(dir.path()).expect("open storage");
    std::mem::forget(dir);
    storage
}

#[test]
fn audit_log_writes_and_lists_entries() {
    let storage = open();

    storage
        .write_audit_log("proj-a", "alice", "index", None, None)
        .expect("write 1");
    storage
        .write_audit_log(
            "proj-a",
            "alice",
            "persist_exploration",
            Some("exp-1"),
            Some("goal x"),
        )
        .expect("write 2");
    storage
        .write_audit_log("proj-b", "bob", "complete_rlm_job", Some("node-9"), None)
        .expect("write 3");

    // Filter by user.
    let alice: Vec<AuditEntry> = storage
        .list_audit_log("", "alice", 100)
        .expect("list alice");
    assert_eq!(alice.len(), 2, "alice has 2 entries");
    assert!(alice.iter().all(|e| e.username == "alice"));

    // Filter by project.
    let proj_a: Vec<AuditEntry> = storage
        .list_audit_log("proj-a", "", 100)
        .expect("list proj-a");
    assert_eq!(proj_a.len(), 2, "proj-a has 2 entries");
    assert!(proj_a.iter().all(|e| e.project == "proj-a"));

    // Filter by both.
    let alice_proj_a: Vec<AuditEntry> = storage
        .list_audit_log("proj-a", "alice", 100)
        .expect("list alice/proj-a");
    assert_eq!(alice_proj_a.len(), 2);

    // Limit caps results.
    let limited: Vec<AuditEntry> = storage
        .list_audit_log("", "alice", 1)
        .expect("list limited");
    assert_eq!(limited.len(), 1, "limit = 1");

    // Newest-first ordering.
    let newest: Vec<AuditEntry> = storage
        .list_audit_log("", "alice", 100)
        .expect("list newest");
    assert!(newest[0].created_at >= newest[1].created_at, "newest first");

    // Detail/target survive a round-trip.
    let exp = alice
        .iter()
        .find(|e| e.action == "persist_exploration")
        .expect("found exploration entry");
    assert_eq!(exp.target.as_deref(), Some("exp-1"));
    assert_eq!(exp.detail.as_deref(), Some("goal x"));
}

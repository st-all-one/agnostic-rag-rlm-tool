use super::*;
use arags_storage::sqlite::buffers::NewBuffer;
use arags_storage::sqlite::chunks::NewChunk;
use arags_storage::sqlite::rlm::{DEFAULT_RLM_LEASE_MS, NewRlmJob};

fn temp_storage() -> Storage {
    Storage::open(tempfile::tempdir().expect("tempdir").path()).expect("open")
}

/// Seed a chunk (and its content) for `file` in `buffer_id` so that the index
/// hook's chunk snapshot is non-empty and treats the file as freshly indexed.
fn seed_chunk(storage: &Storage, buffer_id: i64, file: &str, hash: &[u8]) {
    let id = storage
        .insert_chunk(&NewChunk {
            buffer_id,
            file_path: file.to_string(),
            offset_start: 0,
            offset_end: 10,
            line_start: 1,
            line_end: 5,
            hash: hash.to_vec(),
            language: Some("rust".into()),
            chunk_type: Some("code".into()),
            token_count: Some(10),
        })
        .expect("insert chunk");
    storage
        .insert_chunk_content(id, "fn main() {}")
        .expect("insert chunk content");
}

#[test]
fn theme_buckets_by_first_path_segment() {
    assert_eq!(theme_of("crates/foo/src/lib.rs"), "crates");
    assert_eq!(theme_of("README.md"), "(root)");
    assert_eq!(theme_of("(root)/x"), "(root)");
}

#[test]
fn enqueue_rlm_l1_work_does_not_duplicate_pending_across_commits() {
    let storage = temp_storage();
    let buffer_id = storage
        .insert_buffer(&NewBuffer {
            name: "p".into(),
            path: "/p".into(),
        })
        .expect("insert buffer");
    let project = "p";
    let file = "src/a.rs".to_string();

    // Mimic an indexed buffer commit that produced a chunk for the file.
    seed_chunk(&storage, buffer_id, &file, b"deadbeef");

    // First commit enqueues the L1 job: exactly one new pending job.
    let (new_jobs, reset_jobs) =
        enqueue_rlm_l1_work(&storage, buffer_id, project, &[file.clone()]).unwrap();
    assert_eq!(new_jobs, 1, "first commit creates one new job");
    assert_eq!(reset_jobs, 0);
    assert_eq!(
        storage.count_rlm_jobs(project, "pending").unwrap(),
        1,
        "one pending job after first commit"
    );

    // A second buffer commit touches the same file: the hook must NOT create a
    // duplicate pending job, and must not report it as new work (issue
    // `agnostic-rag-rlm-tool-51be`).
    let (new_jobs2, _) =
        enqueue_rlm_l1_work(&storage, buffer_id, project, &[file.clone()]).unwrap();
    assert_eq!(new_jobs2, 0, "re-enqueue of a live job is not new work");
    assert_eq!(
        storage.count_rlm_jobs(project, "pending").unwrap(),
        1,
        "at most one pending job per (project, level, unit)"
    );

    // And a third pass still leaves exactly one pending job.
    enqueue_rlm_l1_work(&storage, buffer_id, project, &[file.clone()]).unwrap();
    assert_eq!(
        storage.count_rlm_jobs(project, "pending").unwrap(),
        1,
        "repeated enqueues never duplicate"
    );
}

#[test]
fn l1_enqueue_then_cascade_to_l2_and_l3() {
    let storage = temp_storage();
    // Simulate two indexed files.
    let f = |name: &str| format!("src/{name}");
    for name in ["a.rs", "b.rs"] {
        let path = f(name);
        storage
            .enqueue_rlm_job(
                &NewRlmJob {
                    buffer_id: Some(1),
                    project: "p".into(),
                    level: 1,
                    subject: path.clone(),
                    payload: r#"{"hashes":["h"],"texts":["t"]}"#.into(),
                    priority: PRIORITY_FRESH,
                    quorum_slots: 1,
                },
                &[],
            )
            .expect("seed job");
        // Volunteer claims + completes; store the node like the handler.
        let job = storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None, 3)
            .expect("claim")
            .expect("job");
        storage
            .store_rlm_node(&arags_storage::sqlite::rlm::NewRlmNode {
                buffer_id: Some(1),
                project: "p".into(),
                level: 1,
                subject: path.clone(),
                summary_text: format!("summary {path}"),
                source_hashes: vec!["h".into()],
                model: Some("llama3.2".into()),
                volunteer_username: Some("bob".into()),
                created_by: None,
                template_version: Some("t".into()),
                token_count: 5,
            })
            .expect("node");
        storage
            .complete_rlm_job(job.id, "bob", job.generation)
            .expect("complete");
    }

    // Cascade from each L1 completion: first creates the theme job, the
    // second stays within tolerance until hashes diverge.
    assert!(
        cascade_rlm(&storage, 1, "p", 1, &f("a.rs"), 0.3, 1).expect("cascade a"),
        "first cascade should create L2"
    );
    assert!(
        !cascade_rlm(&storage, 1, "p", 1, &f("b.rs"), 0.3, 1).expect("cascade b"),
        "identical hashes are within tolerance"
    );

    // Diverge one child hash: now above tolerance.
    let node = storage
        .get_rlm_node_by_subject("p", 1, &f("b.rs"))
        .expect("get")
        .expect("some");
    storage
        .store_rlm_node(&arags_storage::sqlite::rlm::NewRlmNode {
            source_hashes: vec!["h2".into()],
            subject: f("b.rs"),
            ..node_snapshot(&node)
        })
        .expect("update");
    assert!(
        cascade_rlm(&storage, 1, "p", 1, &f("b.rs"), 0.3, 1).expect("cascade c"),
        "changed hashes exceed tolerance"
    );
}

fn node_snapshot(
    n: &arags_storage::sqlite::rlm::RlmNode,
) -> arags_storage::sqlite::rlm::NewRlmNode {
    arags_storage::sqlite::rlm::NewRlmNode {
        buffer_id: n.buffer_id,
        project: n.project.clone(),
        level: n.level,
        subject: n.subject.clone(),
        summary_text: n.summary_text.clone(),
        source_hashes: n.source_hashes.clone(),
        model: n.model.clone(),
        volunteer_username: n.volunteer_username.clone(),
        created_by: n.created_by.clone(),
        template_version: n.template_version.clone(),
        token_count: n.token_count,
    }
}

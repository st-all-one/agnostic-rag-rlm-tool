use super::*;
use arags_storage::sqlite::rlm::{DEFAULT_RLM_LEASE_MS, NewRlmJob};

fn temp_storage() -> Storage {
    Storage::open(tempfile::tempdir().expect("tempdir").path()).expect("open")
}

#[test]
fn theme_buckets_by_first_path_segment() {
    assert_eq!(theme_of("crates/foo/src/lib.rs"), "crates");
    assert_eq!(theme_of("README.md"), "(root)");
    assert_eq!(theme_of("(root)/x"), "(root)");
}

#[test]
fn l1_enqueue_then_cascade_to_l2_and_l3() {
    let storage = temp_storage();
    // Simulate two indexed files.
    let f = |name: &str| format!("src/{name}");
    for name in ["a.rs", "b.rs"] {
        let path = f(name);
        storage
            .enqueue_rlm_job(&NewRlmJob {
                buffer_id: Some(1),
                project: "p".into(),
                level: 1,
                subject: path.clone(),
                payload: r#"{"hashes":["h"],"texts":["t"]}"#.into(),
                priority: PRIORITY_FRESH,
            })
            .expect("seed job");
        // Volunteer claims + completes; store the node like the handler.
        let job = storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None)
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
        cascade_rlm(&storage, 1, "p", 1, &f("a.rs"), 0.3).expect("cascade a"),
        "first cascade should create L2"
    );
    assert!(
        !cascade_rlm(&storage, 1, "p", 1, &f("b.rs"), 0.3).expect("cascade b"),
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
        cascade_rlm(&storage, 1, "p", 1, &f("b.rs"), 0.3).expect("cascade c"),
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

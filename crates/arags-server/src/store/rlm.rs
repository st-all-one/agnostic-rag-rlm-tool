//! RLM motor: server-side orchestration of the recursive summary pipeline.
//!
//! Responsibilities:
//! - **Post-index enqueue (L1):** after an indexing stream commits, snapshot
//!   each affected file's chunks and enqueue (or reset) its L1 summary job.
//!   Files whose chunks vanished mark their node stale instead.
//! - **Cascade with progressive tolerance:** when a level-N node completes,
//!   evaluate its parent level: enqueue re-summarization only if the parent
//!   node is missing or the change fraction exceeds the level's tolerance
//!   (`l2_tolerance` < `l3_tolerance`), so trivial edits never rebuild the
//!   project overview.
//! - **Theme grouping (L2, safest approach):** deterministic path-prefix
//!   bucketing — the first path segment is the theme/module; files at the
//!   repository root share the `(root)` theme. Deeper clustering (vector
//!   similarity of L1 summaries, entity affinity) is future work.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

use anyhow::{Context, Result};
use arags_storage::Storage;
use arags_storage::sqlite::rlm::{NewRlmJob, PRIORITY_CASCADE, PRIORITY_FRESH, RlmJobPayload};

/// Version tag stamped on jobs/nodes built with these templates.
pub const TEMPLATE_VERSION: &str = "arlm-v1";

/// L2 grouping: first path segment, or `(root)` for top-level files.
#[must_use]
pub fn theme_of(file_path: &str) -> String {
    match file_path.split('/').next() {
        Some(first) if file_path.contains('/') => first.to_string(),
        _ => "(root)".to_string(),
    }
}

fn payload_json(payload: &RlmJobPayload) -> Result<String> {
    serde_json::to_string(payload).context("serialize rlm job payload")
}

/// Enqueue L1 work for the given affected files. Returns
/// `(new_jobs, reset_jobs)` where resets are jobs whose source changed while
/// pending/claimed (generation bump = cooperative cancellation signal).
///
/// Each file is enqueued as **exactly one** pending job (`quorum_slots = 1`):
/// the index hook fires once per buffer commit, so re-enqueuing the same file
/// across commits must not fan out into duplicate pending jobs (issue
/// `agnostic-rag-rlm-tool-51be`). The cosine quorum fan-out is still applied where it
/// matters — at volunteer reassignment (`agnostic-rag-rlm-tool-6d97`) and the
/// cascade path — not here.
///
/// # Errors
///
/// Returns an error if any storage operation fails.
pub fn enqueue_rlm_l1_work(
    storage: &Storage,
    buffer_id: i64,
    project: &str,
    affected_files: &[String],
) -> Result<(usize, usize)> {
    let mut new_jobs = 0;
    let mut reset_jobs = 0;
    for file in affected_files {
        let snapshot = storage.rlm_chunks_snapshot(buffer_id, file)?;
        if snapshot.is_empty() {
            // File fully deleted: stale the node, drop any queued work.
            storage.mark_rlm_stale_by_subject(buffer_id, 1, file)?;
            continue;
        }
        let mut hashes: Vec<String> = snapshot.iter().map(|(_, h, _)| h.clone()).collect();
        hashes.sort();
        hashes.dedup(); // a re-indexed file can carry duplicate chunk rows

        // Change detection against the existing node (if any).
        if let Some(node) = storage.get_rlm_node_by_subject(project, 1, file)? {
            if !node.stale && same_hashes(&node.source_hashes, &hashes) {
                continue; // unchanged since last summarization
            }
            storage.mark_rlm_stale_by_subject(buffer_id, 1, file)?;
        }

        let payload = payload_json(&RlmJobPayload {
            chunk_ids: snapshot.iter().map(|(id, _, _)| *id).collect(),
            hashes,
            texts: snapshot
                .into_iter()
                .map(|(_, _, t)| t.unwrap_or_default())
                .collect(),
            template_version: TEMPLATE_VERSION.to_string(),
            subject_kind: "file".into(),
            ..RlmJobPayload::default()
        })?;
        let job = NewRlmJob {
            buffer_id: Some(buffer_id),
            project: project.to_string(),
            level: 1,
            subject: file.clone(),
            payload,
            priority: PRIORITY_FRESH,
            quorum_slots: 1,
        };
        let (_, generation, created) = storage.enqueue_rlm_job(&job, &[])?;
        if created {
            new_jobs += 1;
        } else if generation > 0 {
            reset_jobs += 1;
        }
    }
    Ok((new_jobs, reset_jobs))
}

/// Evaluate the parent level after a node completed. Enqueues parent-level
/// work only under progressive tolerance:
/// - missing parent node → always enqueue (first build);
/// - otherwise enqueue only when the change fraction exceeds `tolerance`.
///
/// Returns whether parent work was enqueued.
///
/// `quorum_n` is the fan-out degree applied to the enqueued parent job (issue
/// `agnostic-rag-rlm-tool-6d97`); `1` keeps the single-volunteer behaviour.
///
/// # Errors
///
/// Returns an error if any storage operation fails.
pub fn cascade_rlm(
    storage: &Storage,
    buffer_id: i64,
    project: &str,
    child_level: i64,
    child_subject: &str,
    tolerance: f64,
    quorum_n: usize,
) -> Result<bool> {
    let (parent_level, parent_subject) = match child_level {
        1 => (2, theme_of(child_subject)),
        2 => (3, project.to_string()),
        _ => return Ok(false), // L3 is the root; nothing above it
    };

    // Gather current children (non-rejected nodes at the child level that
    // belong to this parent).
    let children: Vec<arags_storage::sqlite::rlm::RlmNode> = storage
        .list_rlm_nodes(project, Some(child_level), true)?
        .into_iter()
        .filter(|n| {
            !matches!(
                n.review_status.as_str(),
                arags_storage::sqlite::rlm::REVIEW_REJECTED
            ) && match child_level {
                1 => theme_of(&n.subject) == parent_subject,
                // L2 themes belong to the single L3 project overview.
                _ => true,
            }
        })
        .collect();
    if children.is_empty() {
        return Ok(false);
    }

    // Union of children hashes is the parent's expected source fingerprint.
    let mut union: Vec<String> = children
        .iter()
        .flat_map(|c| c.source_hashes.iter().cloned())
        .collect();
    union.sort();
    union.dedup();

    let existing = storage.get_rlm_node_by_subject(project, parent_level, &parent_subject)?;
    let need = match &existing {
        None => true, // first build
        Some(node) if node.stale => true,
        Some(node) => {
            let kept = node
                .source_hashes
                .iter()
                .filter(|h| union.contains(h))
                .count();
            #[allow(clippy::cast_precision_loss)] // counts are small
            let changed = 1.0 - (kept as f64 / union.len().max(1) as f64);
            if changed > tolerance {
                storage.mark_rlm_stale_by_subject(buffer_id, parent_level, &parent_subject)?;
                true
            } else {
                false // within tolerance: leave the parent alone
            }
        }
    };
    if !need {
        return Ok(false);
    }

    let payload = payload_json(&RlmJobPayload {
        node_ids: children.iter().map(|c| c.id).collect(),
        hashes: union.clone(),
        texts: children.iter().map(|c| c.summary_text.clone()).collect(),
        template_version: TEMPLATE_VERSION.to_string(),
        subject_kind: if parent_level == 2 {
            "theme"
        } else {
            "project"
        }
        .into(),
        ..RlmJobPayload::default()
    })?;

    // A live job may already exist (pending from an earlier cascade or claimed
    // by a volunteer). Skip when its payload already matches the current
    // inputs; otherwise refresh in place — `cancel` bumps the generation so a
    // claimed worker detects the change and discards stale work.
    if let Some(live) = storage.get_live_rlm_job_by_key(project, parent_level, &parent_subject)? {
        if payload_covers(&live.payload, &union) {
            return Ok(false);
        }
        storage.cancel_rlm_jobs_for_subjects(project, &[(parent_level, parent_subject.clone())])?;
        storage.update_rlm_job_payload(project, parent_level, &parent_subject, &payload)?;
        return Ok(true);
    }
    storage.enqueue_rlm_job(
        &NewRlmJob {
            buffer_id: Some(buffer_id),
            project: project.to_string(),
            level: parent_level,
            subject: parent_subject,
            payload,
            priority: PRIORITY_CASCADE, // cascades outrank fresh L1 work but not cancellations
            quorum_slots: quorum_n,
        },
        &[],
    )?;
    Ok(true)
}

/// Whether a stored job payload's hash set equals the expected union.
fn payload_covers(job_payload: &str, union: &[String]) -> bool {
    serde_json::from_str::<RlmJobPayload>(job_payload)
        .map(|p| same_hashes(&p.hashes, union))
        .unwrap_or(false)
}

fn same_hashes(a: &[String], b: &[String]) -> bool {
    let mut sa: Vec<&String> = a.iter().collect();
    let mut sb: Vec<&String> = b.iter().collect();
    sa.sort_unstable();
    sb.sort_unstable();
    sa == sb
}

#[cfg(test)]
mod tests;

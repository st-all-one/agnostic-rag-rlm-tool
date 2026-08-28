//! Quorum fan-out decision for `CompleteRlmJob` (issue `agnostic-rag-rlm-tool-64af`).
//!
//! When a subject is fanned out to `N > 1` volunteers, each submission only
//! stages a candidate; the cosine quorum decides the published node. The helper
//! returns `Ok(Some(resp))` to short-circuit the single-volunteer path with the
//! definitive response, or `Ok(None)` to fall through to it.

use std::time::Instant;
use tracing::{info, warn};

use subtle::ConstantTimeEq;

use arags_storage::sqlite::rlm::{RlmJob, rlm_job_key};
use tonic::Status;

use crate::grpc::error::internal;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::CompleteRlmJobResponse;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn decide_quorum_submission(
    state: &AppState,
    job: &RlmJob,
    ctx_username: &str,
    raw_token: &str,
    generation: i64,
    summary_text: &str,
    job_id: i64,
    _quorum_n: i64,
    provided_hmac: &str,
    start: Instant,
) -> Result<Option<CompleteRlmJobResponse>, Status> {
    // Attestation gate (issue `agnostic-rag-rlm-tool-64af`): every non-admin
    // volunteer submission is HMAC-signed (session-bound). Verify it before
    // counting the candidate toward the BFT quorum; a mismatch is rejected at
    // the edge and is NEVER staged.
    let verify_elapsed_ms = start.elapsed().as_millis() as u64;
    let expected_hmac = arags_core::rlm_attestation::sign_rlm_submission(
        raw_token,
        job_id,
        generation,
        summary_text,
    );
    let hmac_ok = bool::from(expected_hmac.as_bytes().ct_eq(provided_hmac.as_bytes()));
    if !hmac_ok {
        warn!(
            phase = "rlm_submission_verify",
            elapsed_ms = verify_elapsed_ms,
            job_id,
            volunteer = %ctx_username,
            "rlm submission HMAC mismatch: rejecting before staging"
        );
        return Err(Status::unauthenticated(
            "submission attestation failed: invalid or missing HMAC",
        ));
    }
    info!(
        phase = "rlm_submission_verify",
        elapsed_ms = verify_elapsed_ms,
        job_id,
        volunteer = %ctx_username,
        "rlm submission HMAC verified"
    );

    let subject_key = rlm_job_key(&job.project, job.level, &job.subject);
    let completed = {
        let storage = state.storage.clone();
        let username = ctx_username.to_string();
        store::blocking(move || storage.complete_rlm_job(job_id, &username, generation))
            .await
            .map_err(internal)?
    };
    if !completed {
        return Ok(Some(CompleteRlmJobResponse {
            accepted: false,
            reason: "stale lease, wrong worker or cancelled generation".into(),
            ..CompleteRlmJobResponse::default()
        }));
    }
    store::blocking({
        let storage = state.storage.clone();
        let (p, k, t, by) = (
            job.project.clone(),
            subject_key.clone(),
            summary_text.to_string(),
            ctx_username.to_string(),
        );
        move || storage.insert_submission(&p, "rlm_node", &k, &t, &by)
    })
    .await
    .map_err(internal)?;

    // Audit the candidate staging (issue `agnostic-rag-rlm-tool-7222`). Best-effort:
    // a logging failure must not fail the request.
    state.audit(
        &job.project,
        ctx_username,
        "submit_rlm_candidate",
        Some(&subject_key),
        None,
    );

    // Immediate, idempotent decision: returns Pending until N candidates
    // are staged, Accepted once a consensus is found, Rejected otherwise.
    match crate::quorum::decide_rlm_quorum(state, &job.project, job.level, &job.subject).await {
        Ok(crate::quorum::QuorumDecision::Accepted { .. }) => {
            let node_id = store::blocking({
                let storage = state.storage.clone();
                let (p, l, s) = (job.project.clone(), job.level, job.subject.clone());
                move || {
                    storage
                        .get_rlm_node_by_subject(&p, l, &s)
                        .map(|n| n.map(|x| x.node_id))
                }
            })
            .await
            .map_err(internal)?
            .unwrap_or_default();
            info!(
                job_id,
                %node_id,
                level = job.level,
                volunteer = %ctx_username,
                "rlm quorum job completed (consensus)"
            );
            Ok(Some(CompleteRlmJobResponse {
                accepted: true,
                reason: String::new(),
                node_id,
                auto_approved: false,
            }))
        }
        Ok(crate::quorum::QuorumDecision::Rejected { .. }) => {
            info!(job_id, level = job.level, volunteer = %ctx_username, "rlm quorum rejected (no consensus)");
            Ok(Some(CompleteRlmJobResponse {
                accepted: false,
                reason: "quorum rejected: no consensus among volunteers".into(),
                ..CompleteRlmJobResponse::default()
            }))
        }
        Ok(crate::quorum::QuorumDecision::Pending) => Ok(Some(CompleteRlmJobResponse {
            accepted: false,
            reason: "quorum pending: awaiting more volunteers".into(),
            ..CompleteRlmJobResponse::default()
        })),
        Err(e) => {
            warn!(error = %e, job_id, "rlm quorum decision failed");
            Ok(Some(CompleteRlmJobResponse {
                accepted: false,
                reason: "quorum decision error".into(),
                ..CompleteRlmJobResponse::default()
            }))
        }
    }
}

//! `CompleteRlmJob` handler: persist a volunteer-submitted summary and gate it
//! through the admin review queue or the cosine quorum.

use std::time::Instant;
use tracing::{info, warn};

use arags_storage::sqlite::rlm::NewRlmNode;
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{CompleteRlmJobRequest, CompleteRlmJobResponse};

/// Submit a finished summary. Acceptance requires the caller to still own a
/// valid lease with matching generation; accepted nodes are attributed to the
/// caller and pass into review — unless the submitter is an admin, in which
/// case the node is auto-approved per project decision.
pub(crate) async fn handle_complete_rlm_job(
    state: &AppState,
    request: Request<CompleteRlmJobRequest>,
) -> Result<Response<CompleteRlmJobResponse>, Status> {
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    // Per-user rate limit on this mutating RPC (issue `agnostic-rlm-rs-7222`).
    // A denial must NOT be audited.
    let now = crate::state::AppState::now_secs();
    if !state.check_rate_limit(&ctx.username, now) {
        return Err(Status::resource_exhausted("rate limit exceeded"));
    }
    let start = Instant::now();
    // Re-extract the RAW session token the interceptor placed in the
    // `Authorization` header so we can verify the submission attestation.
    let raw_token = crate::auth::bearer(request.metadata())?;
    let req = request.into_inner();

    if req.summary_text.trim().is_empty() {
        return Err(invalid_arg("summary_text is required"));
    }
    let job_id = req.job_id;
    let generation = req.generation;
    let model = (!req.model.is_empty()).then_some(req.model.clone());
    let template_version =
        (!req.template_version.is_empty()).then_some(req.template_version.clone());
    let token_count = req.token_count.max(0);
    let summary_text = req.summary_text.trim().to_string();

    // Load the job first so provenance (project/level/subject/payload hashes)
    // can be embedded in the node before the atomic complete+persist step.
    let job = {
        let storage = state.storage.clone();
        store::blocking(move || storage.get_rlm_job(job_id))
            .await
            .map_err(internal)?
            .ok_or_else(|| Status::not_found("unknown job_id"))?
    };

    // Persist the node (upsert keyed by project/level/subject) with attribution.
    let hashes = payload_hashes(&job.payload);
    let summary_for_embed = summary_text.clone();
    let node = NewRlmNode {
        buffer_id: job.buffer_id,
        project: job.project.clone(),
        level: job.level,
        subject: job.subject.clone(),
        summary_text: summary_text.clone(),
        source_hashes: hashes,
        model,
        volunteer_username: Some(ctx.username.clone()),
        created_by: Some(ctx.username.clone()),
        template_version,
        token_count,
    };

    // Quorum fan-out path: when a subject is fanned out to N volunteers, each
    // completion only stages a candidate submission; the cosine quorum decides
    // the published node once N candidates are in. We never publish a fresh node
    // here for N>1 (the quorum is the authority).
    let quorum_n = state.config.quorum.n.max(1);
    let is_admin = ctx.is_admin();
    // Admin submitters bypass the cosine quorum and are auto-approved
    // immediately (project decision). The quorum remains the authority for
    // non-admin volunteers, so `quorum.n = 3` can stay the default while a
    // trusted admin token still force-approves (issue `agnostic-rlm-rs-3a68`).
    if quorum_n > 1 && !is_admin {
        if let Some(resp) = super::quorum::decide_quorum_submission(
            state,
            &job,
            &ctx.username,
            &raw_token,
            generation,
            &summary_text,
            job_id,
            quorum_n as i64,
            &req.submission_hmac,
            start,
        )
        .await?
        {
            return Ok(Response::new(resp));
        }
    }

    // Single-volunteer / admin-bypass path: persist the node atomically and
    // gate it through the admin review queue.
    // Atomic completion: lease/generation validation, node upsert and job
    // flip to `done` share one transaction — a failure cannot strand a done
    // job without its node (the claim stays retryable instead).
    let auto_approved = is_admin;
    let accepted = {
        let storage = state.storage.clone();
        let username = ctx.username.clone();
        store::blocking(move || {
            storage.complete_rlm_job_with_node(job_id, &username, generation, &node)
        })
        .await
        .map_err(internal)?
    };
    let Some((rowid, node_id)) = accepted else {
        return Ok(Response::new(CompleteRlmJobResponse {
            accepted: false,
            reason: "stale lease, wrong worker or cancelled generation".into(),
            ..CompleteRlmJobResponse::default()
        }));
    };
    if auto_approved {
        let storage = state.storage.clone();
        let reviewer = ctx.username.clone();
        let node_for_review = node_id.clone();
        store::blocking(move || storage.review_rlm_node(&node_for_review, true, &reviewer, None))
            .await
            .map_err(internal)?;
    }

    // Provenance edges from the payload refs (best effort; edges are additive).
    if let Some(payload) = parse_payload(&job.payload) {
        let storage = state.storage.clone();
        let chunk_ids = payload.chunk_ids;
        let node_ids = payload.node_ids;
        store::blocking(move || -> anyhow::Result<()> {
            for chunk_id in chunk_ids {
                storage.add_rlm_edge(rowid, None, Some(chunk_id))?;
            }
            for child_node_id in node_ids {
                storage.add_rlm_edge(rowid, Some(child_node_id), None)?;
            }
            Ok(())
        })
        .await
        .map_err(internal)?;
    }

    // Embed the summary into the dedicated RLM vector space so semantic
    // search finds it. Embedding is sync CPU work -> blocking task.
    if let Some(vectors) = state.rlm_vector_store.as_ref() {
        let embedder = state.embedder.clone();
        let text = format!("{}\n{}", job.subject, summary_for_embed);
        match tokio::task::spawn_blocking(move || embedder.embed(&text)).await {
            Ok(Ok(vec)) => {
                #[allow(clippy::cast_possible_truncation)] // rowids fit u64 here
                let key = u64::try_from(rowid).unwrap_or(u64::MAX);
                if let Err(e) = vectors.insert(key, &vec) {
                    warn!(error = %e, node_id = %node_id, "rlm vector insert failed; marking node pending_vector");
                    if let Err(m) = state
                        .storage
                        .mark_rlm_nodes_pending_vector(job.buffer_id.unwrap_or(0), &[rowid])
                    {
                        warn!(error = %m, "failed to mark rlm node pending_vector");
                    }
                }
            }
            Ok(Err(e)) => {
                warn!(error = %e, node_id = %node_id, "rlm embedding failed; marking node pending_vector");
                if let Err(m) = state
                    .storage
                    .mark_rlm_nodes_pending_vector(job.buffer_id.unwrap_or(0), &[rowid])
                {
                    warn!(error = %m, "failed to mark rlm node pending_vector");
                }
            }
            Err(e) => {
                warn!(error = %e, node_id = %node_id, "rlm embedding task panicked; marking node pending_vector");
                if let Err(m) = state
                    .storage
                    .mark_rlm_nodes_pending_vector(job.buffer_id.unwrap_or(0), &[rowid])
                {
                    warn!(error = %m, "failed to mark rlm node pending_vector");
                }
            }
        }
    }

    // Cascade: evaluate the parent level under progressive tolerance so a
    // trivial edit never rebuilds the theme/project summary.
    if state.config.rlm.enabled && job.level < 3 {
        let tolerance = if job.level == 1 {
            state.config.rlm.l2_tolerance
        } else {
            state.config.rlm.l3_tolerance
        };
        let storage = state.storage.clone();
        let project = job.project.clone();
        let subject = job.subject.clone();
        let level = job.level;
        match store::blocking(move || {
            store::rlm::cascade_rlm(
                &storage,
                job.buffer_id.unwrap_or(0),
                &project,
                level,
                &subject,
                tolerance,
                quorum_n,
            )
        })
        .await
        {
            Ok(true) => info!(level = job.level, "rlm cascade enqueued parent work"),
            Ok(false) => {}
            Err(e) => warn!(error = %e, "rlm cascade failed"),
        }
    }

    info!(
        job_id,
        %node_id,
        level = job.level,
        volunteer = %ctx.username,
        auto_approved,
        "rlm job completed"
    );

    // Audit the successful completion (issue `agnostic-rlm-rs-7222`).
    // Best-effort: a logging failure must not fail the request.
    state.audit(
        &job.project,
        &ctx.username,
        "complete_rlm_job",
        Some(&node_id),
        None,
    );

    Ok(Response::new(CompleteRlmJobResponse {
        accepted: true,
        reason: String::new(),
        node_id,
        auto_approved,
    }))
}

fn parse_payload(payload: &str) -> Option<arags_storage::sqlite::rlm::RlmJobPayload> {
    serde_json::from_str(payload).ok()
}

fn payload_hashes(payload: &str) -> Vec<String> {
    parse_payload(payload).map(|p| p.hashes).unwrap_or_default()
}

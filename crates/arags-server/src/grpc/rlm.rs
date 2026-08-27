//! RLM recursive-summary RPC handlers.
//!
//! Volunteers claim jobs with a lease (client-configurable, default 500s),
//! synthesize a summary with their local LLM, and submit it through
//! [`handle_complete_rlm_job`]. The server is LLM-free: it only validates,
//! persists, and gates. Attribution (username from the session + model string)
//! is recorded on every node; admin submitters are auto-approved (quality
//! gate), everyone else lands in the review queue.

use arags_storage::sqlite::rlm::{DEFAULT_RLM_LEASE_MS, RlmJobPayload};
use tonic::{Request, Response, Status};

use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{
    ClaimRlmJobRequest, ClaimRlmJobResponse, CompleteRlmJobRequest, CompleteRlmJobResponse,
    GetRlmJobStatusRequest, ListRlmNodesRequest, ListRlmNodesResponse, ReviewRlmNodeRequest,
    ReviewRlmNodeResponse, RlmJobStatus, RlmNodeInfo,
};

/// Claim the next pending job for this authenticated volunteer. The job is
/// locked for the requested lease so no other volunteer receives it meanwhile.
pub(crate) async fn handle_claim_rlm_job(
    state: &AppState,
    request: Request<ClaimRlmJobRequest>,
) -> Result<Response<ClaimRlmJobResponse>, Status> {
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();

    let lease_ms = if req.lease_ms > 0 {
        req.lease_ms
    } else {
        DEFAULT_RLM_LEASE_MS
    };
    if !(1_000..=3_600_000).contains(&lease_ms) {
        return Err(invalid_arg("lease_ms must be between 1000 and 3600000"));
    }
    let max_level = match req.max_level {
        0 => None,
        n @ (1..=3) => Some(i64::from(n)),
        _ => return Err(invalid_arg("max_level must be 0..=3")),
    };

    let storage = state.storage.clone();
    let username = ctx.username.clone();
    let claimed = store::blocking(move || storage.claim_rlm_job(&username, lease_ms, max_level))
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "claim_rlm_job failed");
            internal(e)
        })?;

    Ok(Response::new(match claimed {
        Some(job) => ClaimRlmJobResponse {
            available: true,
            job_id: job.id,
            job_key: job.job_key,
            project: job.project,
            level: i32::try_from(job.level).unwrap_or(1),
            subject: job.subject,
            payload: job.payload,
            generation: job.generation,
            lease_ms: job.lease_ms,
        },
        None => ClaimRlmJobResponse {
            available: false,
            ..ClaimRlmJobResponse::default()
        },
    }))
}

/// Submit a finished summary. Acceptance requires the caller to still own a
/// valid lease with matching generation; accepted nodes are attributed to the
/// caller and pass into review — unless the submitter is an admin, in which
/// case the node is auto-approved per project decision.
pub(crate) async fn handle_complete_rlm_job(
    state: &AppState,
    request: Request<CompleteRlmJobRequest>,
) -> Result<Response<CompleteRlmJobResponse>, Status> {
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
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
    let node = arags_storage::sqlite::rlm::NewRlmNode {
        buffer_id: job.buffer_id,
        project: job.project.clone(),
        level: job.level,
        subject: job.subject.clone(),
        summary_text,
        source_hashes: hashes,
        model,
        volunteer_username: Some(ctx.username.clone()),
        template_version,
        token_count,
    };

    // Atomic completion: lease/generation validation, node upsert and job
    // flip to `done` share one transaction — a failure cannot strand a done
    // job without its node (the claim stays retryable instead).
    let auto_approved = ctx.is_admin();
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
                    tracing::warn!(error = %e, node_id = %node_id, "rlm vector insert failed; marking node pending_vector");
                    if let Err(m) = state
                        .storage
                        .mark_rlm_nodes_pending_vector(job.buffer_id.unwrap_or(0), &[rowid])
                    {
                        tracing::warn!(error = %m, "failed to mark rlm node pending_vector");
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, node_id = %node_id, "rlm embedding failed; marking node pending_vector");
                if let Err(m) = state
                    .storage
                    .mark_rlm_nodes_pending_vector(job.buffer_id.unwrap_or(0), &[rowid])
                {
                    tracing::warn!(error = %m, "failed to mark rlm node pending_vector");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, node_id = %node_id, "rlm embedding task panicked; marking node pending_vector");
                if let Err(m) = state
                    .storage
                    .mark_rlm_nodes_pending_vector(job.buffer_id.unwrap_or(0), &[rowid])
                {
                    tracing::warn!(error = %m, "failed to mark rlm node pending_vector");
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
            )
        })
        .await
        {
            Ok(true) => tracing::info!(level = job.level, "rlm cascade enqueued parent work"),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "rlm cascade failed"),
        }
    }

    tracing::info!(
        job_id,
        %node_id,
        level = job.level,
        volunteer = %ctx.username,
        auto_approved,
        "rlm job completed"
    );

    Ok(Response::new(CompleteRlmJobResponse {
        accepted: true,
        reason: String::new(),
        node_id,
        auto_approved,
    }))
}

/// Worker poll: detect cancellation while processing (source data changed).
pub(crate) async fn handle_get_rlm_job_status(
    state: &AppState,
    request: Request<GetRlmJobStatusRequest>,
) -> Result<Response<RlmJobStatus>, Status> {
    crate::auth::authenticate(request.metadata(), &state.storage)?;
    let job_id = request.into_inner().job_id;
    let job = {
        let storage = state.storage.clone();
        store::blocking(move || storage.get_rlm_job(job_id))
            .await
            .map_err(internal)?
    };
    Ok(Response::new(match job {
        Some(j) => RlmJobStatus {
            job_id,
            status: j.status,
            generation: j.generation,
        },
        None => RlmJobStatus {
            job_id,
            status: "unknown".into(),
            generation: -1,
        },
    }))
}

/// Quality gate (admin only). Rejection re-enqueues reprocessing with
/// elevated priority.
pub(crate) async fn handle_review_rlm_node(
    state: &AppState,
    request: Request<ReviewRlmNodeRequest>,
) -> Result<Response<ReviewRlmNodeResponse>, Status> {
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    if !ctx.is_admin() {
        return Err(Status::permission_denied("admin role required"));
    }
    let req = request.into_inner();

    let applied = {
        let storage = state.storage.clone();
        let username = ctx.username.clone();
        let node_id = req.node_id.clone();
        let reason = (!req.reason.is_empty()).then_some(req.reason.clone());
        store::blocking(move || {
            storage.review_rlm_node(&node_id, req.approved, &username, reason.as_deref())
        })
        .await
        .map_err(internal)?
    };
    if applied && !req.approved {
        // Rejection: requeue reprocessing at elevated priority.
        let storage = state.storage.clone();
        let node_id = req.node_id.clone();
        let subject = store::blocking(move || storage.rlm_subject_of(&node_id))
            .await
            .map_err(internal)?;
        if let Some((project, level, subj)) = subject {
            let storage2 = state.storage.clone();
            store::blocking(move || {
                storage2.cancel_rlm_jobs_for_subjects(&project, &[(level, subj)])
            })
            .await
            .map_err(internal)?;
        }
    }
    Ok(Response::new(ReviewRlmNodeResponse { applied }))
}

/// List summaries of a project. Non-admins only see approved nodes.
pub(crate) async fn handle_list_rlm_nodes(
    state: &AppState,
    request: Request<ListRlmNodesRequest>,
) -> Result<Response<ListRlmNodesResponse>, Status> {
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();
    if req.project.trim().is_empty() {
        return Err(invalid_arg("project is required"));
    }
    let include_pending = req.include_pending && ctx.is_admin();
    let level = i64::from(req.level);
    let level = (level > 0).then_some(level);

    let nodes = {
        let storage = state.storage.clone();
        let project = req.project.clone();
        store::blocking(move || storage.list_rlm_nodes(&project, level, include_pending))
            .await
            .map_err(internal)?
    };

    Ok(Response::new(ListRlmNodesResponse {
        nodes: nodes
            .into_iter()
            .map(|n| RlmNodeInfo {
                node_id: n.node_id,
                level: i32::try_from(n.level).unwrap_or(0),
                subject: n.subject,
                summary_text: n.summary_text,
                review_status: n.review_status,
                model: n.model.unwrap_or_default(),
                volunteer_username: n.volunteer_username.unwrap_or_default(),
                confidence: n.confidence,
                stale: n.stale,
                created_at: epoch_to_string(n.created_at),
                updated_at: epoch_to_string(n.updated_at),
            })
            .collect(),
    }))
}

// ── helpers ─────────────────────────────────────────────────────────────

fn parse_payload(payload: &str) -> Option<RlmJobPayload> {
    serde_json::from_str(payload).ok()
}

fn payload_hashes(payload: &str) -> Vec<String> {
    parse_payload(payload).map(|p| p.hashes).unwrap_or_default()
}

fn epoch_to_string(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

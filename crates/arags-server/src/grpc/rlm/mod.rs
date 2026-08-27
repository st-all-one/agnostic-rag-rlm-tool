//! RLM recursive-summary RPC handlers.
//!
//! Volunteers claim jobs with a lease (client-configurable, default 500s),
//! synthesize a summary with their local LLM, and submit it through
//! [`handle_complete_rlm_job`]. The server is LLM-free: it only validates,
//! persists, and gates. Attribution (username from the session + model string)
//! is recorded on every node; admin submitters are auto-approved (quality
//! gate), everyone else lands in the review queue.

pub(crate) mod complete;
pub(crate) mod quorum;

pub(crate) use complete::handle_complete_rlm_job;

use tonic::{Request, Response, Status};

use arags_proto::proto::{
    ClaimRlmJobRequest, ClaimRlmJobResponse, GetRlmJobStatusRequest, ListRlmNodesRequest,
    ListRlmNodesResponse, ReviewRlmNodeRequest, ReviewRlmNodeResponse, RlmJobStatus, RlmNodeInfo,
};
use arags_storage::sqlite::rlm::DEFAULT_RLM_LEASE_MS;

use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

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
    let strikes_limit = state.config.quorum.strikes_limit;
    let claimed = store::blocking(move || {
        storage.claim_rlm_job(&username, lease_ms, max_level, strikes_limit)
    })
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

fn epoch_to_string(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

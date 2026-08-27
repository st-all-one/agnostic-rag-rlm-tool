//! Explorations RPC handlers (plan 022).
//!
//! Explorer agents persist dense, goal-driven maps of code relationships
//! (fire-and-forget, like `StoreAnswer`) and consumers search them with a
//! server-side confidence score so every caller sees consistent rankings.
//! The server is LLM-free: it validates the contract, anchors cited files
//! with content hashes, embeds summaries into the dedicated vector space,
//! rechecks anchors at read time and serves the maps.
//!
//! Split by concern:
//! - `search`: semantic search + single-map fetch (read-time trust pipeline)
//! - `feedback`: admin invalidation + admin review gate (the public consumer
//!   feedback RPC was HARD-REMOVED in issue `agnostic-rlm-rs-f5f3` — sybil
//!   risk; internal `record_feedback` storage may still be exercised directly)

pub mod feedback;
pub(crate) mod grounding;
pub mod search;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_grounding;
#[cfg(test)]
mod tests_moderation;

use arags_core::exploration::ConfidenceConfig;
use arags_proto::proto::PersistExplorationRequest;
use tonic::{Request, Response, Status};

use crate::config::ValidationMode;

use crate::grpc::error::{internal, invalid_arg, not_found};
use crate::state::AppState;
use crate::store;

/// Hard caps protecting the data plane from oversized payloads.
const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_GOAL_CHARS: usize = 2_000;
const MAX_SUMMARY_CHARS: usize = 4_000;
const MAX_FILES: usize = 128;

/// Search defaults applied when a request omits them.
pub(crate) const DEFAULT_LIMIT: i32 = 5;
pub(crate) const MAX_LIMIT: i32 = 25;

/// Confidence config derived from `[exploration]` server knobs.
pub(crate) fn exploration_confidence(state: &AppState) -> ConfidenceConfig {
    let cfg = &state.config.exploration;
    ConfidenceConfig {
        hit_high: cfg.hit_high,
        hit_low: cfg.hit_low,
        ..ConfidenceConfig::default()
    }
}

/// Age in whole days since `epoch_ms`, clamped at zero for future stamps.
#[allow(clippy::cast_precision_loss)] // ms→days fits f32 exactly here
pub(crate) fn age_days_since(epoch_ms: i64) -> u32 {
    let now = arags_storage::sqlite::tokens::now_ms();
    let days = (now - epoch_ms).max(0) / 86_400_000;
    u32::try_from(days).unwrap_or(u32::MAX)
}

/// Embed text with the shared embedder inside a blocking task; failures are
/// logged and returned as empty so persistence never depends on embedding.
pub(crate) async fn embed_lenient(state: &AppState, text: String) -> Option<Vec<f32>> {
    let embedder = state.embedder.clone();
    match tokio::task::spawn_blocking(move || embedder.embed(&text)).await {
        Ok(Ok(vec)) if !vec.is_empty() => Some(vec),
        Ok(Ok(_)) => None,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "embedding failed");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "embedding task panicked");
            None
        }
    }
}

/// Persist an exploration map fire-and-forget. Anchors that resolve keep the
/// map honest over time; unresolved paths are reported but never fail the call.
pub(crate) async fn handle_persist_exploration(
    state: &AppState,
    request: Request<PersistExplorationRequest>,
) -> Result<Response<arags_proto::proto::PersistExplorationResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.persist_exploration");
    let ctx = crate::auth::authenticate(request.metadata(), &state.storage)?;
    // Per-user rate limit on this mutating RPC (issue `agnostic-rlm-rs-7222`).
    // A denial must NOT be audited.
    let now = crate::state::AppState::now_secs();
    if !state.check_rate_limit(&ctx.username, now) {
        return Err(Status::resource_exhausted("rate limit exceeded"));
    }
    let mut req = request.into_inner();

    if !state.config.exploration.enabled {
        return Err(Status::unavailable(
            "explorations disabled by configuration",
        ));
    }
    validate_persist(&req)?;

    let buffer_id = store::blocking({
        let storage = state.storage.clone();
        let project = req.project.clone();
        move || store::buffer_id_for_project(&storage, &project)
    })
    .await
    .map_err(internal)?
    .ok_or_else(|| not_found(&format!("unknown project {}", req.project)))?;

    let (anchors, unresolved_paths) =
        resolve_anchors(state, buffer_id, std::mem::take(&mut req.files)).await?;

    let input = arags_storage::explorations::PersistExplorationInput {
        project: req.project.clone(),
        buffer_id: Some(buffer_id),
        goal: req.goal.trim().to_string(),
        body_markdown: std::mem::take(&mut req.body_markdown),
        summary: req.summary.trim().to_string(),
        anchors,
        created_by: ctx.username.clone(),
        model: (!req.model.is_empty()).then_some(req.model.clone()),
        template_version: arags_core::exploration::TEMPLATE_VERSION_V1.into(),
        token_count: 0,
    };

    let storage = state.storage.clone();
    let stored = store::blocking(move || storage.persist_exploration(&input))
        .await
        .map_err(internal)?;

    // Validation gate (issue `agnostic-rlm-rs-e89e`): route non-admin persists
    // per `[exploration] validation_mode`. Admins auto-approve (maps stay
    // `fresh`) in both modes. Non-admins in `Review` mode keep the original
    // admin-approval gate when `require_review` is set; non-admins in `Quorum`
    // mode (the default) land non-surfaced as a `candidate` submission for the
    // future cosine quorum worker (`6d97`/`64af`) to decide.
    let mut review_note = String::new();
    let mode = state.config.exploration.validation_mode;
    if ctx.is_admin() {
        tracing::debug!(
            phase = "exploration_persist",
            path = "admin_auto_approve",
            "admin persist auto-approved"
        );
    } else if mode == ValidationMode::Review && state.config.exploration.require_review {
        let storage = state.storage.clone();
        let rowid = stored.id;
        match store::blocking(move || storage.mark_exploration_pending(rowid)).await {
            Ok(true) => review_note = "pending admin review".into(),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "failed to mark exploration pending"),
        }
    } else if mode == ValidationMode::Quorum && !ctx.is_admin() {
        // Hold the map non-surfaced (reuse the existing `pending` gating so no
        // search-logic change is needed) and record a candidate submission for
        // the quorum worker to later accept/reject.
        let storage = state.storage.clone();
        let rowid = stored.id;
        if let Err(e) = store::blocking(move || storage.mark_exploration_pending(rowid)).await {
            tracing::warn!(error = %e, "failed to mark quorum exploration pending");
        }
        let storage = state.storage.clone();
        let project = req.project.clone();
        let subject_key = stored.exploration_id.clone();
        let subject_key_log = subject_key.clone();
        let candidate_text = req.summary.clone();
        let candidate_by = ctx.username.clone();
        match store::blocking(move || {
            storage.insert_submission(
                &project,
                "exploration",
                &subject_key,
                &candidate_text,
                &candidate_by,
            )
        })
        .await
        {
            Ok(sub_id) => {
                review_note = "pending quorum validation".into();
                tracing::info!(
                    phase = "exploration_persist",
                    path = "quorum_candidate",
                    submission_id = sub_id,
                    exploration_id = %subject_key_log,
                    "exploration candidate submitted for quorum"
                );
            }
            Err(e) => tracing::warn!(error = %e, "failed to record quorum submission"),
        }
    }

    // Embed goal+summary into the dedicated space (best effort). Pending maps
    // are embedded too: search gates them by status, and an admin approval
    // must not require a re-embed.
    if let Some(vectors) = state.exploration_vector_store.as_ref() {
        let text = format!("{}\n{}", req.goal, req.summary);
        if let Some(vec) = embed_lenient(state, text).await {
            #[allow(clippy::cast_possible_truncation)] // rowids fit u64 here
            let key = u64::try_from(stored.id).unwrap_or(u64::MAX);
            if let Err(e) = vectors.insert(key, &vec) {
                tracing::warn!(error = %e, exploration_id = %stored.exploration_id, "exploration vector insert failed; marking exploration pending_vector");
                if let Err(m) = state
                    .storage
                    .mark_explorations_pending_vector(buffer_id, &[stored.id])
                {
                    tracing::warn!(error = %m, "failed to mark exploration pending_vector");
                }
            }
        }
    }

    tracing::info!(
        exploration_id = %stored.exploration_id,
        project = %req.project,
        created_by = %ctx.username,
        unresolved = unresolved_paths.len(),
        review = %review_note,
        "exploration persisted"
    );

    // Audit the successful persist (issue `agnostic-rlm-rs-7222`). Best-effort:
    // a logging failure must not fail the request.
    state.audit(
        &req.project,
        &ctx.username,
        "persist_exploration",
        Some(&stored.exploration_id),
        None,
    );

    Ok(Response::new(
        arags_proto::proto::PersistExplorationResponse {
            exploration_id: stored.exploration_id,
            accepted: true,
            reason: review_note,
            unresolved_paths,
        },
    ))
}

fn validate_persist(req: &PersistExplorationRequest) -> Result<(), Status> {
    if req.project.trim().is_empty() {
        return Err(invalid_arg("project is required"));
    }
    if req.goal.trim().is_empty() {
        return Err(invalid_arg("goal is required"));
    }
    if req.summary.trim().is_empty() {
        return Err(invalid_arg("summary is required"));
    }
    if req.body_markdown.trim().is_empty() {
        return Err(invalid_arg("body_markdown is required"));
    }
    if req.goal.chars().count() > MAX_GOAL_CHARS {
        return Err(invalid_arg("goal too long"));
    }
    if req.summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(invalid_arg("summary too long"));
    }
    if req.body_markdown.len() > MAX_BODY_BYTES {
        return Err(invalid_arg("body_markdown too large"));
    }
    if req.files.len() > MAX_FILES {
        return Err(invalid_arg("too many files"));
    }
    Ok(())
}

/// Resolve request files into anchors + unresolved paths against the index.
async fn resolve_anchors(
    state: &AppState,
    buffer_id: i64,
    files: Vec<String>,
) -> Result<
    (
        Vec<arags_storage::explorations::ExplorationAnchor>,
        Vec<String>,
    ),
    Status,
> {
    use arags_storage::explorations::ExplorationAnchor;
    use arags_storage::explorations::ROLE_CITED;

    let storage = state.storage.clone();
    let resolved = store::blocking(move || storage.current_hashes_for_paths(buffer_id, &files))
        .await
        .map_err(internal)?;

    let mut anchors = Vec::new();
    let mut unresolved = Vec::new();
    for (path, hash) in resolved {
        match hash {
            Some(content_hash) => anchors.push(ExplorationAnchor {
                buffer_id,
                path,
                content_hash,
                role: ROLE_CITED.into(),
            }),
            None => unresolved.push(path),
        }
    }
    Ok((anchors, unresolved))
}

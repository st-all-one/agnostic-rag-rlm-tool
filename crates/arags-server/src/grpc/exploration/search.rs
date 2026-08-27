//! Exploration search + single-map fetch (plan 022).
//!
//! Both handlers share the read-time trust pipeline: vector candidates →
//! anchor recheck → confidence score → status gate. Anchors are rechecked on
//! every hit because hits are rare and the persisted flag alone can lag a
//! concurrent index run.

use arags_core::exploration::confidence_score;
use arags_proto::proto::{
    ExplorationHit, GetExplorationByIdRequest, GetExplorationByIdResponse,
    SearchExplorationsRequest, SearchExplorationsResponse,
};
use tonic::{Request, Response, Status};

pub(crate) use super::grounding::{Grounding, ground_candidate};
use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

use super::DEFAULT_LIMIT;
use super::MAX_LIMIT;
use super::age_days_since;
use super::embed_lenient;

/// Extra vector candidates fetched so staleness filtering still fills a page.
const SEARCH_MARGIN: usize = 3;

pub(crate) struct Candidate {
    pub(super) row: arags_storage::explorations::ExplorationRow,
    pub(super) broken: Vec<String>,
    pub(super) similarity: f32,
}

impl Candidate {
    /// A map is stale when any cited anchor broke now or its stored status
    /// says so; retired maps never surface in search results.
    fn is_stale(&self) -> bool {
        !self.broken.is_empty() || self.row.status != arags_storage::explorations::STATUS_FRESH
    }

    fn confidence(&self, cfg: &arags_core::exploration::ConfidenceConfig, epoch: i64) -> f32 {
        let drift = u32::try_from((epoch - self.row.epoch_created).max(0)).unwrap_or(u32::MAX);
        confidence_score(
            self.similarity,
            drift,
            age_days_since(self.row.created_at),
            saturate_u32(self.row.confirmed),
            saturate_u32(self.row.contradicted),
            cfg,
        )
    }
}

fn saturate_u32(value: i64) -> u32 {
    u32::try_from(value.clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
}

fn hit_from(cand: &Candidate, confidence: f32, epoch: i64) -> ExplorationHit {
    let drift = i32::try_from((epoch - cand.row.epoch_created).max(0)).unwrap_or(i32::MAX);
    ExplorationHit {
        exploration_id: cand.row.exploration_id.clone(),
        goal: cand.row.goal.clone(),
        summary: cand.row.summary.clone(),
        confidence,
        similarity: cand.similarity,
        status: if cand.broken.is_empty() {
            cand.row.status.clone()
        } else {
            arags_storage::explorations::STATUS_STALE.into()
        },
        stale_reason: if cand.broken.is_empty() {
            cand.row.stale_reason.clone()
        } else {
            cand.broken.clone()
        },
        epoch_drift: drift,
        confirmed: cand.row.confirmed,
        contradicted: cand.row.contradicted,
        created_by: cand.row.created_by.clone(),
        model: cand.row.model.clone().unwrap_or_default(),
        epoch: cand.row.epoch_created,
        version: cand.row.version,
    }
}

/// Semantic search over maps RPC: authenticate + validate, then run the
/// shared read-time trust pipeline via [`search_explorations_core`].
pub(crate) async fn handle_search_explorations(
    state: &AppState,
    request: Request<SearchExplorationsRequest>,
) -> Result<Response<SearchExplorationsResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.search_explorations");
    crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();

    if req.project.trim().is_empty() || req.query.trim().is_empty() {
        return Err(invalid_arg("project and query are required"));
    }
    let limit = usize::try_from(if req.limit <= 0 {
        DEFAULT_LIMIT
    } else {
        req.limit.min(MAX_LIMIT)
    })
    .unwrap_or(1);

    let response = search_explorations_core(
        state,
        req.project,
        req.query,
        limit,
        req.include_stale,
        req.as_of_epoch,
    )
    .await?;
    Ok(Response::new(response))
}

/// Core exploration search shared by the RPC handler and the unified query
/// pipeline (plan 023): embed query → top-k in the dedicated space → anchor
/// recheck at read time → composite confidence → threshold gate.
///
/// The caller is responsible for authentication and argument validation. When
/// `as_of_epoch > 0` (plan 021) each hit is time-traveled to the revision of
/// its `(project, goal)` map active at that epoch.
pub(crate) async fn search_explorations_core(
    state: &AppState,
    project: String,
    query: String,
    limit: usize,
    include_stale: bool,
    as_of_epoch: i64,
) -> Result<SearchExplorationsResponse, Status> {
    if limit == 0 {
        return Ok(SearchExplorationsResponse { hits: Vec::new() });
    }
    let start = std::time::Instant::now();
    let req_project = project.as_str();
    let req_query = query.as_str();

    let Some(vectors) = state.exploration_vector_store.as_ref() else {
        return Ok(SearchExplorationsResponse { hits: Vec::new() });
    };

    let Some(query_vec) = embed_lenient(state, req_query.to_string()).await else {
        return Err(internal("embedding unavailable"));
    };

    let candidates = vectors
        .search(&query_vec, limit + SEARCH_MARGIN)
        .map_err(internal)?;

    // Hydrate rows and recheck anchors per candidate (blocking tasks). Rows
    // that vanished between index write and hydration are skipped: the
    // vector index can briefly lag SQLite (debounced saves, concurrent
    // deletes) and one stale key must not fail the whole search.
    let mut cands: Vec<Candidate> = Vec::with_capacity(candidates.len());
    for cand in &candidates {
        #[allow(clippy::cast_possible_wrap)] // rowids fit i64 here
        let rowid = i64::try_from(cand.id).unwrap_or(i64::MAX);
        let storage = state.storage.clone();
        let as_of = as_of_epoch;
        let resolved = store::blocking(move || -> Result<Option<arags_storage::explorations::ExplorationRow>, anyhow::Error> {
            let Some(row) = storage.get_exploration_by_rowid(rowid)? else {
                return Ok(None);
            };
            if as_of > 0 {
                // Time-travel (plan 021): pick the revision active at T.
                return storage.get_exploration_as_of(&row.project, &row.goal, as_of);
            }
            Ok(Some(row))
        })
        .await
        .map_err(internal)?;
        let Some(row) = resolved else {
            tracing::debug!(rowid, "exploration vanished mid-search; skipping");
            continue;
        };
        if row.project != req_project {
            continue;
        }
        let storage = state.storage.clone();
        let broken = store::blocking(move || storage.recheck_anchors_for_rowid(rowid))
            .await
            .map_err(internal)?;
        cands.push(Candidate {
            row,
            broken,
            similarity: cand.similarity,
        });
    }

    let storage = state.storage.clone();
    let epoch_project = req_project.to_string();
    let epoch = store::blocking(move || storage.current_project_epoch(&epoch_project))
        .await
        .map_err(internal)?;

    let cfg = super::exploration_confidence(state);
    let verify = state.config.exploration.verify_on_hit && state.vector_store.is_some();
    let mut hits: Vec<(f32, ExplorationHit)> = Vec::new();
    for c in &cands {
        // Gate before scoring: fresh always; pending review never surfaces
        // (plan 023 review gate); stale only when asked; retired never;
        // below `hit_low` nothing surfaces (precision > recall).
        if c.row.status == arags_storage::explorations::STATUS_RETIRED
            || c.row.status == arags_storage::explorations::STATUS_PENDING
        {
            continue;
        }
        if !include_stale && c.is_stale() {
            continue;
        }
        if cfg.classify(c.similarity) == arags_core::exploration::HitClass::None {
            continue;
        }

        // Lazy verify-on-hit (plan 022.8): weak corpus support forces stale.
        let grounding = if verify {
            ground_candidate(state, c).await
        } else {
            None
        };
        let grounded_out = matches!(grounding, Some(Grounding::Unsupported));
        if grounded_out {
            tracing::info!(rowid = c.row.id, "grounding downgraded map to stale");
            if !include_stale {
                continue;
            }
            let mut forced = hit_from(c, 0.0, epoch);
            forced.status = arags_storage::explorations::STATUS_STALE.into();
            forced.confidence = 0.0;
            forced.stale_reason = vec!["grounding weak: no supporting chunks found".into()];
            hits.push((0.0, forced));
            continue;
        }

        let score = c.confidence(&cfg, epoch);
        hits.push((score, hit_from(c, score, epoch)));
    }
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);

    for (_, hit) in &hits {
        touch_logged(state, &hit.exploration_id).await;
    }

    tracing::debug!(
        elapsed_ms = start.elapsed().as_millis(),
        candidates = candidates.len(),
        hits = hits.len(),
        "search_explorations_core completed"
    );
    Ok(SearchExplorationsResponse {
        hits: hits.into_iter().map(|(_, h)| h).collect(),
    })
}

async fn touch_logged(state: &AppState, exploration_id: &str) {
    let storage = state.storage.clone();
    let id = exploration_id.to_string();
    match store::blocking(move || {
        storage
            .get_exploration_by_uuid(&id)?
            .map_or(Ok(()), |row| storage.touch_exploration(row.id))
    })
    .await
    {
        Ok(()) => {}
        Err(e) => tracing::warn!(error = %e, "exploration touch failed"),
    }
}

/// Fetch one map with full body, anchors and live trust metadata.
pub(crate) async fn handle_get_exploration_by_id(
    state: &AppState,
    request: Request<GetExplorationByIdRequest>,
) -> Result<Response<GetExplorationByIdResponse>, Status> {
    let _timer = crate::timing::Timer::new("handler.get_exploration_by_id");
    crate::auth::authenticate(request.metadata(), &state.storage)?;
    let req = request.into_inner();
    if req.exploration_id.trim().is_empty() {
        return Err(invalid_arg("exploration_id is required"));
    }

    let storage = state.storage.clone();
    let id = req.exploration_id.clone();
    let as_of = req.as_of_epoch;
    let Some(row) = store::blocking(
        move || -> Result<Option<arags_storage::explorations::ExplorationRow>, anyhow::Error> {
            let Some(row) = storage.get_exploration_by_uuid(&id)? else {
                return Ok(None);
            };
            if as_of > 0 {
                // Time-travel (plan 021): resolve the revision active at T.
                return storage.get_exploration_as_of_by_id(&id, as_of);
            }
            Ok(Some(row))
        },
    )
    .await
    .map_err(internal)?
    else {
        return Err(crate::grpc::error::not_found("unknown exploration_id"));
    };

    let storage = state.storage.clone();
    let anchors = store::blocking(move || storage.list_exploration_anchors(row.id))
        .await
        .map_err(internal)?;

    let storage = state.storage.clone();
    let broken = store::blocking(move || storage.recheck_anchors_for_rowid(row.id))
        .await
        .map_err(internal)?;

    let project = row.project.clone();
    let storage = state.storage.clone();
    let epoch = store::blocking(move || storage.current_project_epoch(&project))
        .await
        .map_err(internal)?;

    let cfg = super::exploration_confidence(state);
    let cand = Candidate {
        row: row.clone(),
        broken: Vec::new(),
        similarity: 0.0,
    };
    // Direct fetches report live trust even without a similarity context:
    // drift/age/feedback still apply, similarity is unknown (0 keeps it honest).
    let trust = cand.confidence(&cfg, epoch);

    Ok(Response::new(GetExplorationByIdResponse {
        found: true,
        exploration_id: row.exploration_id,
        project: row.project,
        goal: row.goal,
        body_markdown: row.body,
        summary: row.summary,
        confidence: trust,
        status: if broken.is_empty() {
            row.status
        } else {
            arags_storage::explorations::STATUS_STALE.into()
        },
        stale_reason: if broken.is_empty() {
            row.stale_reason
        } else {
            broken
        },
        epoch_drift: i32::try_from((epoch - row.epoch_created).max(0)).unwrap_or(i32::MAX),
        confirmed: row.confirmed,
        contradicted: row.contradicted,
        created_by: row.created_by,
        model: row.model.unwrap_or_default(),
        anchored_files: anchors
            .into_iter()
            .map(|(buf, path, hash)| format!("{path}@{hash} [buffer:{buf}]"))
            .collect(),
        epoch: row.epoch_created,
        version: row.version,
    }))
}

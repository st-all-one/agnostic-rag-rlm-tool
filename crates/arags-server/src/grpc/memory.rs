//! Memory / cache / maintenance RPCs (plan 019).
//!
//! - `ListMemory`: admin view of the semantic `qa_cache` for a project.
//! - `GetCache`: admin/debug fetch of a single cached answer by id.
//! - `TriggerMaintenance`: admin run (or dry-run) of consolidate + decay.
//! - `record_query_history`: shared helper that attributes a query to the
//!   authenticated user (plan 019, E).

use anyhow::Context;
use arags_proto::proto::*;
use arags_storage::Storage;
use tonic::{Request, Response, Status};

use crate::auth::{self, AuthContext};
use crate::grpc::error::{internal, invalid_arg};
use crate::state::AppState;
use crate::store;

/// A minimal projection of a `qa_cache` row for `ListMemory`.
struct QaMemoryRow {
    cache_id: String,
    project: String,
    question_text: String,
    created_at: i64,
    confidence: f64,
}

/// List cached query/answer memory for a project (admin-gated).
///
/// # Errors
///
/// Returns `PERMISSION_DENIED` for non-admins, or `internal` on storage failure.
pub async fn handle_list_memory(
    state: &AppState,
    request: Request<ListMemoryRequest>,
) -> Result<Response<ListMemoryResponse>, Status> {
    let ctx = auth::authenticate(request.metadata(), &state.storage)?;
    auth::require_admin(&ctx)?;

    let req = request.into_inner();
    let limit = if req.limit > 0 { req.limit } else { 100 };

    let storage = state.storage.clone();
    let project = req.project.clone();
    let rows = store::blocking(move || list_memory_rows(&storage, &project, limit))
        .await
        .map_err(internal)?;

    let entries = rows
        .iter()
        .map(|r| MemoryEntry {
            cache_id: r.cache_id.clone(),
            project: r.project.clone(),
            question: r.question_text.clone(),
            created_at: r.created_at.to_string(),
            score: r.confidence,
            entities: Vec::new(),
        })
        .collect();

    let stats = format!(
        "project={} entries={} include_entities={}",
        req.project,
        rows.len(),
        req.include_entities
    );

    Ok(Response::new(ListMemoryResponse { entries, stats }))
}

/// Fetch cached query/answer memory for a project (admin-gated).
fn list_memory_rows(
    storage: &Storage,
    project: &str,
    limit: i64,
) -> anyhow::Result<Vec<QaMemoryRow>> {
    let conn = storage.connection()?;
    conn.execute(|conn| {
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
            if project.is_empty() {
                (
                    "SELECT cache_id, project, question_text, created_at, confidence \
                 FROM qa_cache ORDER BY created_at DESC LIMIT ?1"
                        .to_string(),
                    vec![Box::new(limit)],
                )
            } else {
                (
                    "SELECT cache_id, project, question_text, created_at, confidence \
                 FROM qa_cache WHERE project = ?1 ORDER BY created_at DESC LIMIT ?2"
                        .to_string(),
                    vec![Box::new(project), Box::new(limit)],
                )
            };

        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare list_memory query")?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(AsRef::as_ref).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                Ok(QaMemoryRow {
                    cache_id: r.get(0)?,
                    project: r.get(1)?,
                    question_text: r.get(2)?,
                    created_at: r.get(3)?,
                    confidence: r.get(4)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    })
}

/// Fetch a single cached answer by id (admin/debug path).
///
/// # Errors
///
/// Returns `PERMISSION_DENIED` for non-admins, `invalid_argument` without a
/// `cache_id`, or `internal` on storage failure.
pub async fn handle_get_cache(
    state: &AppState,
    request: Request<GetCacheRequest>,
) -> Result<Response<GetCacheResponse>, Status> {
    let ctx = auth::authenticate(request.metadata(), &state.storage)?;
    auth::require_admin(&ctx)?;

    let req = request.into_inner();
    if req.cache_id.trim().is_empty() {
        return Err(invalid_arg("cache_id is required"));
    }

    let storage = state.storage.clone();
    let cache_id = req.cache_id.clone();
    let row = store::blocking(move || storage.get_qa_by_cache_id(&cache_id))
        .await
        .map_err(internal)?;

    match row {
        Some(r) => {
            let ids: Vec<i64> = r
                .source_chunk_ids
                .iter()
                .filter_map(|s| s.parse::<i64>().ok())
                .collect();
            let files = if ids.is_empty() {
                Vec::new()
            } else {
                let storage = state.storage.clone();
                store::blocking(move || store::chunks::chunk_file_paths(&storage, &ids))
                    .await
                    .map_err(internal)?
            };
            Ok(Response::new(GetCacheResponse {
                answer: r.answer_text,
                source_chunk_ids: r.source_chunk_ids,
                files,
                project: r.project,
            }))
        }
        None => Ok(Response::new(GetCacheResponse::default())),
    }
}

/// Run (or dry-run) cache cleanup + decay for a project (admin-gated).
///
/// # Errors
///
/// Returns `PERMISSION_DENIED` for non-admins, or `internal` on storage failure.
pub async fn handle_trigger_maintenance(
    state: &AppState,
    request: Request<TriggerMaintenanceRequest>,
) -> Result<Response<MaintenanceReport>, Status> {
    let ctx = auth::authenticate(request.metadata(), &state.storage)?;
    auth::require_admin(&ctx)?;

    let req = request.into_inner();
    let storage = state.storage.clone();
    let project = req.project.clone();
    let floor = state.config.maintenance.decay_score_floor;

    let report = crate::maintenance::run_maintenance(&project, &storage, floor, req.dry_run)
        .await
        .map_err(internal)?;

    Ok(Response::new(MaintenanceReport {
        duplicate_chunks_removed: i64::try_from(report.duplicate_chunks_removed)
            .unwrap_or(i64::MAX),
        low_confidence_patterns_removed: i64::try_from(report.low_confidence_patterns_removed)
            .unwrap_or(i64::MAX),
        decayed_chunks: i64::try_from(report.decayed_chunks).unwrap_or(i64::MAX),
        kept: i64::try_from(report.kept).unwrap_or(i64::MAX),
    }))
}

/// Record a query against history, attributing it to the authenticated user
/// (plan 019, E). Errors are intentionally swallowed: history recording must
/// never fail a user-facing query.
pub(crate) async fn record_query_history(
    state: &AppState,
    ctx: &AuthContext,
    project: &str,
    query_type: &str,
    query: &str,
) {
    let storage = state.storage.clone();
    let project = project.to_string();
    let query = query.to_string();
    let query_type = query_type.to_string();
    let user = ctx.username.clone();

    let _ = store::blocking(move || {
        let buffer_id = store::buffer_id_for_project(&storage, &project)
            .ok()
            .flatten();
        arags_memory::HistoryManager::new(storage).record_with_user(
            buffer_id,
            &query,
            Some(&query_type),
            None,
            None,
            None,
            &user,
        )
    })
    .await;
}

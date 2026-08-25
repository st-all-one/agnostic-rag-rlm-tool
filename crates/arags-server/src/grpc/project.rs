//! Project management RPCs: `CreateProject`, `ListProjects`, `GetProject`.

use tonic::{Response, Status};

use crate::grpc::error::{internal, not_found};
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{CreateProjectRequest, ListProjectsResponse, ProjectInfo};

fn project_to_info(
    row: &store::ProjectRow,
    created_at: Option<prost_types::Timestamp>,
) -> ProjectInfo {
    ProjectInfo {
        id: row.uuid.clone().unwrap_or_else(|| row.id.to_string()),
        name: row.name.clone(),
        root_path: row.path.clone(),
        chunk_count: row.total_chunks,
        file_count: row.total_files,
        created_at,
    }
}

/// Persist a project (buffer) and return its metadata.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_create_project(
    state: &AppState,
    req: CreateProjectRequest,
) -> Result<Response<ProjectInfo>, Status> {
    if req.name.trim().is_empty() {
        return Err(Status::invalid_argument("project name is required"));
    }
    if req.root_path.trim().is_empty() {
        return Err(Status::invalid_argument("project root_path is required"));
    }

    let name = req.name.clone();
    let path = req.root_path.clone();
    let storage = state.storage.clone();

    let project_id = store::blocking(move || store::insert_project(&storage, &name, &path))
        .await
        .map_err(internal)?;

    let storage = state.storage.clone();
    let row = store::blocking(move || store::get_project_by_id(&storage, project_id))
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("project just created is missing"))?;

    tracing::info!(project_id, name = %row.name, "project created");

    Ok(Response::new(project_to_info(
        &row,
        Some(ts(row.created_at)),
    )))
}

/// List all projects.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_list_projects(
    state: &AppState,
) -> Result<Response<ListProjectsResponse>, Status> {
    let storage = state.storage.clone();
    let projects = store::blocking(move || store::list_projects(&storage))
        .await
        .map_err(internal)?;

    tracing::info!(count = projects.len(), "listed projects");

    let infos = projects
        .iter()
        .map(|row| project_to_info(row, Some(ts(row.created_at))))
        .collect();

    Ok(Response::new(ListProjectsResponse { projects: infos }))
}

/// Fetch a single project by UUID or numeric id.
///
/// # Errors
///
/// Returns an error if storage access fails or the project is unknown.
pub(crate) async fn handle_get_project(
    state: &AppState,
    project: String,
) -> Result<Response<ProjectInfo>, Status> {
    let storage = state.storage.clone();
    let project = project.clone();
    let row = store::blocking(move || {
        if let Some(row) = store::get_project_by_uuid(&storage, &project)? {
            return Ok(Some(row));
        }
        store::get_project_by_name(&storage, &project)
    })
    .await
    .map_err(internal)?
    .ok_or_else(|| not_found("project not found"))?;

    Ok(Response::new(project_to_info(
        &row,
        Some(ts(row.created_at)),
    )))
}

fn ts(seconds: i64) -> prost_types::Timestamp {
    prost_types::Timestamp { seconds, nanos: 0 }
}

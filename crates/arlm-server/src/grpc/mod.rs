//! Tonic service implementation for arlm.
//!
//! The trait is implemented here, but each RPC group lives in its own module:
//!
//! - [`project`]: project management RPCs
//! - [`index`]: indexing RPC
//! - [`search`]: search + context building RPCs
//! - [`status`]: server status RPCs

pub mod auth;
pub mod error;
pub mod history;
pub mod index;
pub mod memory;
pub mod project;
pub mod query_cache;
pub mod search;
pub mod status;

use arlm_proto::proto::arlm_service_server::ArlmService;
use arlm_proto::proto::*;
use tonic::{Request, Response, Status, Streaming};

use crate::state::AppState;

/// gRPC service implementation for arlm.
pub struct ArlmGrpcService {
    state: AppState,
}

impl ArlmGrpcService {
    /// Create a new gRPC service from shared app state.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl ArlmService for ArlmGrpcService {
    // ── Project management ────────────────────────────────────────────────

    async fn create_project(
        &self,
        request: Request<CreateProjectRequest>,
    ) -> Result<Response<ProjectInfo>, Status> {
        let _timer = crate::timing::Timer::new("handler.create_project");
        crate::auth::authenticate(request.metadata(), &self.state.storage)?;
        project::handle_create_project(&self.state, request.into_inner()).await
    }

    async fn list_projects(
        &self,
        request: Request<()>,
    ) -> Result<Response<ListProjectsResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.list_projects");
        crate::auth::authenticate(request.metadata(), &self.state.storage)?;
        project::handle_list_projects(&self.state).await
    }

    async fn get_project(&self, request: Request<String>) -> Result<Response<ProjectInfo>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_project");
        crate::auth::authenticate(request.metadata(), &self.state.storage)?;
        project::handle_get_project(&self.state, request.into_inner()).await
    }

    // ── Indexing ──────────────────────────────────────────────────────────

    async fn index_project(
        &self,
        request: Request<Streaming<IndexChunk>>,
    ) -> Result<Response<IndexResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.index_project");
        crate::auth::authenticate(request.metadata(), &self.state.storage)?;
        index::handle_index_project(&self.state, request).await
    }

    // ── Search ────────────────────────────────────────────────────────────

    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.search");
        search::handle_search(&self.state, request).await
    }

    async fn build_context(
        &self,
        request: Request<ContextRequest>,
    ) -> Result<Response<ContextResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.build_context");
        crate::auth::authenticate(request.metadata(), &self.state.storage)?;
        search::handle_build_context(&self.state, request.into_inner()).await
    }

    // ── Server status ─────────────────────────────────────────────────────

    async fn get_server_status(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ServerStatus>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_server_status");
        status::handle_get_server_status(&self.state).await
    }

    // ── Auth (plan 018) ────────────────────────────────────────────────────

    async fn auth_refresh(
        &self,
        request: Request<AuthRefreshRequest>,
    ) -> Result<Response<AuthRefreshResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.auth_refresh");
        auth::handle_auth_refresh(&self.state, request.into_inner()).await
    }

    async fn invalidate_cache(
        &self,
        request: Request<InvalidateCacheRequest>,
    ) -> Result<Response<InvalidateCacheResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.invalidate_cache");
        query_cache::handle_invalidate_cache(&self.state, request).await
    }

    // ── Query-Answer Cache (plan 017, client-side digest-once) ────────────

    async fn query_with_cache(
        &self,
        request: Request<QueryWithCacheRequest>,
    ) -> Result<Response<QueryWithCacheResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.query_with_cache");
        query_cache::handle_query_with_cache(&self.state, request).await
    }

    async fn store_answer(
        &self,
        request: Request<StoreAnswerRequest>,
    ) -> Result<Response<StoreAnswerResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.store_answer");
        query_cache::handle_store_answer(&self.state, request).await
    }

    async fn get_answer_by_id(
        &self,
        request: Request<GetAnswerByIdRequest>,
    ) -> Result<Response<GetAnswerByIdResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_answer_by_id");
        query_cache::handle_get_answer_by_id(&self.state, request).await
    }

    // ── Memory / cache admin (plan 019) ──────────────────────────────────

    async fn list_memory(
        &self,
        request: Request<ListMemoryRequest>,
    ) -> Result<Response<ListMemoryResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.list_memory");
        memory::handle_list_memory(&self.state, request).await
    }

    async fn get_cache(
        &self,
        request: Request<GetCacheRequest>,
    ) -> Result<Response<GetCacheResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_cache");
        memory::handle_get_cache(&self.state, request).await
    }

    async fn trigger_maintenance(
        &self,
        request: Request<TriggerMaintenanceRequest>,
    ) -> Result<Response<MaintenanceReport>, Status> {
        let _timer = crate::timing::Timer::new("handler.trigger_maintenance");
        memory::handle_trigger_maintenance(&self.state, request).await
    }

    // ── History (plan 019, E) ────────────────────────────────────────────

    async fn get_history(
        &self,
        request: Request<GetHistoryRequest>,
    ) -> Result<Response<GetHistoryResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_history");
        history::handle_get_history(&self.state, request).await
    }
}

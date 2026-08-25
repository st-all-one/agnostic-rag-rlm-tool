//! Tonic service implementation for arags.
//!
//! The trait is implemented here, but each RPC group lives in its own module:
//!
//! - [`project`]: project management RPCs
//! - [`index`]: indexing RPC
//! - [`search`]: search + context building RPCs
//! - [`status`]: server status RPCs

pub mod auth;
pub mod error;
pub mod exploration;
pub mod history;
pub mod index;
pub mod memory;
pub mod project;
pub mod query_cache;
pub mod rlm;
pub mod search;
pub mod status;
pub mod util;

use arags_proto::proto::arags_service_server::AragsService;
use tonic::{Request, Response, Status, Streaming};

use crate::state::AppState;

use arags_proto::proto::{
    AuthRefreshRequest, AuthRefreshResponse, ClaimRlmJobRequest, ClaimRlmJobResponse,
    CompleteRlmJobRequest, CompleteRlmJobResponse, ContextRequest, ContextResponse,
    CreateProjectRequest, GetAnswerByIdRequest, GetAnswerByIdResponse, GetCacheRequest,
    GetCacheResponse, GetHistoryRequest, GetHistoryResponse, GetRlmJobStatusRequest, IndexChunk,
    IndexResponse, InvalidateCacheRequest, InvalidateCacheResponse, ListMemoryRequest,
    ListMemoryResponse, ListProjectsResponse, ListRlmNodesRequest, ListRlmNodesResponse,
    MaintenanceReport, ProjectInfo, QueryWithCacheRequest, QueryWithCacheResponse,
    ReviewRlmNodeRequest, ReviewRlmNodeResponse, RlmJobStatus, SearchExplorationsRequest,
    SearchExplorationsResponse, SearchRequest, SearchResponse, ServerStatus, StoreAnswerRequest,
    StoreAnswerResponse, TriggerMaintenanceRequest,
};
use arags_proto::proto::{
    FeedbackExplorationRequest, FeedbackExplorationResponse, GetExplorationByIdRequest,
    GetExplorationByIdResponse, InvalidateExplorationRequest, InvalidateExplorationResponse,
    PersistExplorationRequest, PersistExplorationResponse,
};

/// gRPC service implementation for arags.
pub struct AragsGrpcService {
    state: AppState,
}

impl AragsGrpcService {
    /// Create a new gRPC service from shared app state.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl AragsService for AragsGrpcService {
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

    // ── RLM recursive summaries ──────────────────────────────────────────

    async fn claim_rlm_job(
        &self,
        request: Request<ClaimRlmJobRequest>,
    ) -> Result<Response<ClaimRlmJobResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.claim_rlm_job");
        rlm::handle_claim_rlm_job(&self.state, request).await
    }

    async fn complete_rlm_job(
        &self,
        request: Request<CompleteRlmJobRequest>,
    ) -> Result<Response<CompleteRlmJobResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.complete_rlm_job");
        rlm::handle_complete_rlm_job(&self.state, request).await
    }

    async fn get_rlm_job_status(
        &self,
        request: Request<GetRlmJobStatusRequest>,
    ) -> Result<Response<RlmJobStatus>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_rlm_job_status");
        rlm::handle_get_rlm_job_status(&self.state, request).await
    }

    async fn review_rlm_node(
        &self,
        request: Request<ReviewRlmNodeRequest>,
    ) -> Result<Response<ReviewRlmNodeResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.review_rlm_node");
        rlm::handle_review_rlm_node(&self.state, request).await
    }

    async fn list_rlm_nodes(
        &self,
        request: Request<ListRlmNodesRequest>,
    ) -> Result<Response<ListRlmNodesResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.list_rlm_nodes");
        rlm::handle_list_rlm_nodes(&self.state, request).await
    }

    // ── Explorations (plan 022) ──────────────────────────────────────────

    async fn persist_exploration(
        &self,
        request: Request<PersistExplorationRequest>,
    ) -> Result<Response<PersistExplorationResponse>, Status> {
        exploration::handle_persist_exploration(&self.state, request).await
    }

    async fn search_explorations(
        &self,
        request: Request<SearchExplorationsRequest>,
    ) -> Result<Response<SearchExplorationsResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.search_explorations");
        exploration::search::handle_search_explorations(&self.state, request).await
    }

    async fn get_exploration_by_id(
        &self,
        request: Request<GetExplorationByIdRequest>,
    ) -> Result<Response<GetExplorationByIdResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_exploration_by_id");
        exploration::search::handle_get_exploration_by_id(&self.state, request).await
    }

    async fn feedback_exploration(
        &self,
        request: Request<FeedbackExplorationRequest>,
    ) -> Result<Response<FeedbackExplorationResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.feedback_exploration");
        exploration::feedback::handle_feedback_exploration(&self.state, request).await
    }

    async fn invalidate_exploration(
        &self,
        request: Request<InvalidateExplorationRequest>,
    ) -> Result<Response<InvalidateExplorationResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.invalidate_exploration");
        exploration::feedback::handle_invalidate_exploration(&self.state, request).await
    }
}

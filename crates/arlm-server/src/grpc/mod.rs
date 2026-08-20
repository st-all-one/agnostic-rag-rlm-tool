//! Tonic service implementation for arlm.
//!
//! The trait is implemented here, but each RPC group lives in its own module:
//!
//! - [`project`]: project management RPCs
//! - [`index`]: indexing RPC
//! - [`search`]: search + context building RPCs
//! - [`runs`]: RLM run RPCs
//! - [`session`]: session RPCs
//! - [`summarize`]: summarization RPCs
//! - [`status`]: server status RPCs

pub mod error;
pub mod index;
pub mod project;
pub mod runs;
pub mod search;
pub mod session;
pub mod status;
pub mod summarize;

use std::pin::Pin;

use arlm_proto::proto::arlm_service_server::ArlmService;
use arlm_proto::proto::*;
use futures::Stream;
use tonic::{Request, Response, Status};

use crate::state::AppState;

/// gRPC service implementation for arlm.
pub struct ArlmGrpcService {
    state: AppState,
}

/// A boxed server-streaming stream produced by handlers.
pub(crate) type EventStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

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
        project::handle_create_project(&self.state, request.into_inner()).await
    }

    async fn list_projects(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ListProjectsResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.list_projects");
        project::handle_list_projects(&self.state).await
    }

    async fn get_project(&self, request: Request<String>) -> Result<Response<ProjectInfo>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_project");
        project::handle_get_project(&self.state, request.into_inner()).await
    }

    // ── Indexing ──────────────────────────────────────────────────────────

    async fn index_project(
        &self,
        request: Request<IndexRequest>,
    ) -> Result<Response<IndexResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.index_project");
        index::handle_index_project(&self.state, request.into_inner()).await
    }

    // ── Search ────────────────────────────────────────────────────────────

    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.search");
        search::handle_search(&self.state, request.into_inner()).await
    }

    async fn build_context(
        &self,
        request: Request<ContextRequest>,
    ) -> Result<Response<ContextResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.build_context");
        search::handle_build_context(&self.state, request.into_inner()).await
    }

    // ── RLM runs ──────────────────────────────────────────────────────────

    async fn start_run(
        &self,
        request: Request<RunRequest>,
    ) -> Result<Response<RunResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.start_run");
        runs::handle_start_run(&self.state, request.into_inner()).await
    }

    async fn get_run(&self, request: Request<String>) -> Result<Response<RunResult>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_run");
        runs::handle_get_run(&self.state, request.into_inner()).await
    }

    async fn cancel_run(&self, request: Request<String>) -> Result<Response<()>, Status> {
        let _timer = crate::timing::Timer::new("handler.cancel_run");
        runs::handle_cancel_run(&self.state, request.into_inner()).await
    }

    type StreamRunStream = EventStream<RunEvent>;

    async fn stream_run(
        &self,
        request: Request<String>,
    ) -> Result<Response<Self::StreamRunStream>, Status> {
        let _timer = crate::timing::Timer::new("handler.stream_run");
        runs::handle_stream_run(&self.state, request.into_inner())
    }

    // ── Sessions ──────────────────────────────────────────────────────────

    async fn create_session(
        &self,
        request: Request<CreateSessionRequest>,
    ) -> Result<Response<SessionInfo>, Status> {
        let _timer = crate::timing::Timer::new("handler.create_session");
        session::handle_create_session(&self.state, request.into_inner()).await
    }

    async fn list_sessions(
        &self,
        request: Request<String>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.list_sessions");
        session::handle_list_sessions(&self.state, request.into_inner()).await
    }

    async fn get_session(&self, request: Request<String>) -> Result<Response<SessionInfo>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_session");
        session::handle_get_session(&self.state, request.into_inner()).await
    }

    async fn add_session_turn(
        &self,
        request: Request<AddSessionTurnRequest>,
    ) -> Result<Response<SessionTurn>, Status> {
        let _timer = crate::timing::Timer::new("handler.add_session_turn");
        session::handle_add_session_turn(&self.state, request.into_inner()).await
    }

    // ── Summarization ─────────────────────────────────────────────────────

    async fn trigger_summarize(
        &self,
        request: Request<SummarizeRequest>,
    ) -> Result<Response<SummarizeResponse>, Status> {
        let _timer = crate::timing::Timer::new("handler.trigger_summarize");
        summarize::handle_trigger_summarize(&self.state, request.into_inner()).await
    }

    async fn get_summary_status(
        &self,
        request: Request<String>,
    ) -> Result<Response<SummaryStatus>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_summary_status");
        summarize::handle_get_summary_status(&self.state, request.into_inner()).await
    }

    type StreamSummarizeProgressStream = EventStream<SummarizeProgress>;

    async fn stream_summarize_progress(
        &self,
        request: Request<String>,
    ) -> Result<Response<Self::StreamSummarizeProgressStream>, Status> {
        let _timer = crate::timing::Timer::new("handler.stream_summarize_progress");
        summarize::handle_stream_summarize_progress(&self.state, request.into_inner())
    }

    // ── Server status ─────────────────────────────────────────────────────

    async fn get_server_status(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ServerStatus>, Status> {
        let _timer = crate::timing::Timer::new("handler.get_server_status");
        status::handle_get_server_status(&self.state).await
    }

    type StreamEventsStream = EventStream<RunEvent>;

    async fn stream_events(
        &self,
        _request: Request<()>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let _timer = crate::timing::Timer::new("handler.stream_events");
        status::handle_stream_events(&self.state)
    }
}

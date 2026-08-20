use arlm_proto::proto::arlm_service_server::ArlmService;
use arlm_proto::proto::*;
use tonic::{Request, Response, Status};

use crate::state::AppState;

/// gRPC service implementation for arlm.
pub struct ArlmGrpcService {
    state: AppState,
}

impl ArlmGrpcService {
    /// Create a new gRPC service.
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl ArlmService for ArlmGrpcService {
    async fn create_project(
        &self,
        request: Request<CreateProjectRequest>,
    ) -> Result<Response<ProjectInfo>, Status> {
        let req = request.into_inner();
        // TODO: Implement project creation
        Ok(Response::new(ProjectInfo {
            id: uuid::Uuid::new_v4().to_string(),
            name: req.name,
            root_path: req.root_path,
            chunk_count: 0,
            file_count: 0,
            created_at: Some(prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            }),
        }))
    }

    async fn list_projects(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ListProjectsResponse>, Status> {
        // TODO: Implement project listing
        Ok(Response::new(ListProjectsResponse { projects: vec![] }))
    }

    async fn get_project(
        &self,
        request: Request<String>,
    ) -> Result<Response<ProjectInfo>, Status> {
        let _project_id = request.into_inner();
        // TODO: Implement project retrieval
        Err(Status::unimplemented("get_project not yet implemented"))
    }

    async fn index_project(
        &self,
        request: Request<IndexRequest>,
    ) -> Result<Response<IndexResponse>, Status> {
        let _req = request.into_inner();
        // TODO: Implement indexing
        Err(Status::unimplemented("index_project not yet implemented"))
    }

    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        let project = req.project;
        let query = req.query;
        let max_results = if req.max_results > 0 {
            req.max_results as usize
        } else {
            10
        };

        // Get storage connection
        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        // Execute FTS5 search
            let results = conn.execute(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT c.id, c.file_path, c.start_line, c.end_line, \
                     bm25(chunks_fts) as score, cc.content \
                     FROM chunks_fts \
                     JOIN chunks c ON c.id = chunks_fts.rowid \
                     LEFT JOIN chunk_content cc ON cc.chunk_id = c.id \
                     WHERE chunks_fts MATCH ?1 \
                       AND c.project = ?2 \
                     ORDER BY score \
                     LIMIT ?3",
                )
                .map_err(|e| anyhow::anyhow!("failed to prepare search query: {e}"))?;

            let rows = stmt
                .query_map(rusqlite::params![query, project, max_results as i64], |row| {
                    Ok(SearchResult {
                        chunk_id: row.get(0)?,
                        text: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                        score: row.get(1)?,
                        file_path: row.get(2)?,
                        start_line: row.get(3).unwrap_or(0),
                        end_line: row.get(4).unwrap_or(0),
                        is_summary: false,
                        summary: None,
                    })
                })
                .map_err(|e| anyhow::anyhow!("failed to execute search query: {e}"))?;

            let mut results = Vec::new();
            for row in rows {
                if let Ok(result) = row {
                    results.push(result);
                }
            }

            Ok(results)
        }).map_err(|e| Status::internal(format!("search failed: {e}")))?;

        let total_count = results.len() as i32;
        Ok(Response::new(SearchResponse {
            results,
            total_count,
            duration_ms: 0.0,
        }))
    }

    async fn build_context(
        &self,
        request: Request<ContextRequest>,
    ) -> Result<Response<ContextResponse>, Status> {
        let _req = request.into_inner();
        // TODO: Implement context building
        Err(Status::unimplemented("build_context not yet implemented"))
    }

    async fn start_run(
        &self,
        request: Request<RunRequest>,
    ) -> Result<Response<RunResponse>, Status> {
        let req = request.into_inner();
        let run_id = uuid::Uuid::new_v4().to_string();

        // Store run in database
        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        conn.execute(|conn| {
            conn.execute(
                "INSERT INTO runs (id, project, task, backend, model, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
                rusqlite::params![
                    run_id,
                    req.project,
                    req.task,
                    req.backend,
                    req.model,
                    chrono::Utc::now().timestamp(),
                ],
            )?;
            Ok(())
        })
        .map_err(|e| Status::internal(format!("failed to create run: {e}")))?;

        Ok(Response::new(RunResponse {
            run_id,
            status: RunStatus::Running.into(),
        }))
    }

    async fn get_run(
        &self,
        request: Request<String>,
    ) -> Result<Response<RunResult>, Status> {
        let run_id = request.into_inner();

        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        let result = conn.execute(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, project, task, backend, model, status, result, started_at, finished_at FROM runs WHERE id = ?1",
                )
                .map_err(|e| anyhow::anyhow!("failed to prepare query: {e}"))?;

            let row = stmt
                .query_row(rusqlite::params![run_id], |row| {
                    Ok(RunResult {
                        id: row.get(0)?,
                        project: row.get(1)?,
                        task: row.get(2)?,
                        backend: row.get(3)?,
                        model: row.get(4)?,
                        status: row.get::<_, String>(5)?.parse().unwrap_or(RunStatus::Unknown),
                        result: row.get::<_, Option<String>>(6)?,
                        started_at: row.get(7)?,
                        finished_at: row.get(8)?,
                        ..Default::default()
                    })
                })
                .map_err(|e| anyhow::anyhow!("run not found: {e}"))?;

            Ok(row)
        })
        .map_err(|e| Status::internal(format!("failed to get run: {e}")))?;

        Ok(Response::new(result))
    }

    async fn cancel_run(
        &self,
        request: Request<String>,
    ) -> Result<Response<()>, Status> {
        let run_id = request.into_inner();

        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        conn.execute(|conn| {
            conn.execute(
                "UPDATE runs SET status = 'cancelled', finished_at = ?1 WHERE id = ?2",
                rusqlite::params![chrono::Utc::now().timestamp(), run_id],
            )?;
            Ok(())
        })
        .map_err(|e| Status::internal(format!("failed to cancel run: {e}")))?;

        Ok(Response::new(()))
    }

    type StreamRunStream = tokio_stream::wrappers::ReceiverStream<Result<RunEvent, Status>>;

    async fn stream_run(
        &self,
        request: Request<String>,
    ) -> Result<Response<Self::StreamRunStream>, Status> {
        let _run_id = request.into_inner();
        // TODO: Implement run streaming
        Err(Status::unimplemented("stream_run not yet implemented"))
    }

    async fn create_session(
        &self,
        request: Request<CreateSessionRequest>,
    ) -> Result<Response<SessionInfo>, Status> {
        let req = request.into_inner();
        let session_id = uuid::Uuid::new_v4().to_string();

        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        conn.execute(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, project, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                rusqlite::params![
                    session_id,
                    req.project,
                    req.title,
                    chrono::Utc::now().timestamp(),
                ],
            )?;
            Ok(())
        })
        .map_err(|e| Status::internal(format!("failed to create session: {e}")))?;

        Ok(Response::new(SessionInfo {
            id: session_id,
            project: req.project,
            title: req.title,
            turn_count: 0,
            created_at: Some(prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            }),
            updated_at: Some(prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            }),
        }))
    }

    async fn list_sessions(
        &self,
        request: Request<String>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let project = request.into_inner();

        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        let sessions = conn.execute(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, project, title, created_at, updated_at FROM sessions WHERE project = ?1 ORDER BY updated_at DESC",
                )
                .map_err(|e| anyhow::anyhow!("failed to prepare query: {e}"))?;

            let rows = stmt
                .query_map(rusqlite::params![project], |row| {
                    Ok(SessionInfo {
                        id: row.get(0)?,
                        project: row.get(1)?,
                        title: row.get(2)?,
                        turn_count: 0,
                        created_at: row.get::<_, Option<i64>>(3)?.map(|ts| prost_types::Timestamp {
                            seconds: ts,
                            nanos: 0,
                        }),
                        updated_at: row.get::<_, Option<i64>>(4)?.map(|ts| prost_types::Timestamp {
                            seconds: ts,
                            nanos: 0,
                        }),
                    })
                })
                .map_err(|e| anyhow::anyhow!("failed to query sessions: {e}"))?;

            let mut sessions = Vec::new();
            for row in rows {
                if let Ok(session) = row {
                    sessions.push(session);
                }
            }
            Ok(sessions)
        })
        .map_err(|e| Status::internal(format!("failed to list sessions: {e}")))?;

        Ok(Response::new(ListSessionsResponse { sessions }))
    }

    async fn get_session(
        &self,
        request: Request<String>,
    ) -> Result<Response<SessionInfo>, Status> {
        let session_id = request.into_inner();

        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        let session = conn.execute(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, project, title, created_at, updated_at FROM sessions WHERE id = ?1",
                )
                .map_err(|e| anyhow::anyhow!("failed to prepare query: {e}"))?;

            let row = stmt
                .query_row(rusqlite::params![session_id], |row| {
                    Ok(SessionInfo {
                        id: row.get(0)?,
                        project: row.get(1)?,
                        title: row.get(2)?,
                        turn_count: 0,
                        created_at: row.get::<_, Option<i64>>(3)?.map(|ts| prost_types::Timestamp {
                            seconds: ts,
                            nanos: 0,
                        }),
                        updated_at: row.get::<_, Option<i64>>(4)?.map(|ts| prost_types::Timestamp {
                            seconds: ts,
                            nanos: 0,
                        }),
                    })
                })
                .map_err(|e| anyhow::anyhow!("session not found: {e}"))?;

            Ok(row)
        })
        .map_err(|e| Status::internal(format!("failed to get session: {e}")))?;

        Ok(Response::new(session))
    }

    async fn add_session_turn(
        &self,
        request: Request<AddSessionTurnRequest>,
    ) -> Result<Response<SessionTurn>, Status> {
        let req = request.into_inner();
        let turn_id = uuid::Uuid::new_v4().to_string();

        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        conn.execute(|conn| {
            // Insert the turn
            conn.execute(
                "INSERT INTO session_turns (id, session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    turn_id,
                    req.session_id,
                    req.role,
                    req.content,
                    chrono::Utc::now().timestamp(),
                ],
            )?;

            // Update session's updated_at
            conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![chrono::Utc::now().timestamp(), req.session_id],
            )?;

            Ok(())
        })
        .map_err(|e| Status::internal(format!("failed to add session turn: {e}")))?;

        Ok(Response::new(SessionTurn {
            id: turn_id,
            session_id: req.session_id,
            role: req.role,
            content: req.content,
            created_at: Some(prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            }),
        }))
    }

    async fn trigger_summarize(
        &self,
        request: Request<SummarizeRequest>,
    ) -> Result<Response<SummarizeResponse>, Status> {
        let req = request.into_inner();

        // Get the buffer_id for the project
        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        let buffer_id = conn.execute(|conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM buffers WHERE project = ?1")
                .map_err(|e| anyhow::anyhow!("failed to prepare query: {e}"))?;

            let row = stmt.query_row(rusqlite::params![req.project], |row| row.get(0));
            row.map_err(|e| anyhow::anyhow!("buffer not found: {e}"))
        })
        .map_err(|e| Status::internal(format!("failed to get buffer: {e}")))?;

        // Trigger summarization in background
        let storage = self.state.storage.clone();
        let project = req.project.clone();
        tokio::spawn(async move {
            let summarizer = crate::summarizer::Summarizer::new(
                storage,
                Arc::new(arlm_llm::NoopLlm),
            );
            match summarizer.summarize_project(buffer_id, 4).await {
                Ok(result) => {
                    tracing::info!(
                        project,
                        file_summaries = result.file_summaries,
                        module_summaries = result.module_summaries,
                        project_summaries = result.project_summaries,
                        "summarization completed"
                    );
                }
                Err(e) => {
                    tracing::error!(project, error = %e, "summarization failed");
                }
            }
        });

        Ok(Response::new(SummarizeResponse {
            status: "started".to_string(),
            estimated_files: 0,
        }))
    }

    async fn get_summary_status(
        &self,
        request: Request<String>,
    ) -> Result<Response<SummaryStatus>, Status> {
        let project = request.into_inner();

        // Get summary count for the project
        let conn = self.state.storage.connection().map_err(|e| {
            Status::internal(format!("failed to get storage connection: {e}"))
        })?;

        let (total_summaries, file_summaries, module_summaries, project_summaries) =
            conn.execute(|conn| {
                let total: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM summaries WHERE buffer_id IN (SELECT id FROM buffers WHERE project = ?1)",
                        rusqlite::params![project],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                let file: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM summaries WHERE scope = 'file' AND buffer_id IN (SELECT id FROM buffers WHERE project = ?1)",
                        rusqlite::params![project],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                let module: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM summaries WHERE scope = 'module' AND buffer_id IN (SELECT id FROM buffers WHERE project = ?1)",
                        rusqlite::params![project],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                let project_count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM summaries WHERE scope = 'project' AND buffer_id IN (SELECT id FROM buffers WHERE project = ?1)",
                        rusqlite::params![project],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                Ok((total, file, module, project_count))
            })
            .map_err(|e| Status::internal(format!("failed to get summary status: {e}")))?;

        Ok(Response::new(SummaryStatus {
            running: false,
            current_file: String::new(),
            files_remaining: 0,
            estimated_cost_usd: 0.0,
            total_summaries: total_summaries as i32,
            file_summaries: file_summaries as i32,
            module_summaries: module_summaries as i32,
            project_summaries: project_summaries as i32,
        }))
    }

    type StreamSummarizeProgressStream =
        tokio_stream::wrappers::ReceiverStream<Result<SummarizeProgress, Status>>;

    async fn stream_summarize_progress(
        &self,
        request: Request<String>,
    ) -> Result<Response<Self::StreamSummarizeProgressStream>, Status> {
        let _project = request.into_inner();
        // TODO: Implement summarization progress streaming
        Err(Status::unimplemented(
            "stream_summarize_progress not yet implemented",
        ))
    }

    async fn get_server_status(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ServerStatus>, Status> {
        // Get storage statistics
        let (total_projects, total_chunks) = {
            let conn = self.state.storage.connection().map_err(|e| {
                Status::internal(format!("failed to get storage connection: {e}"))
            })?;

            conn.execute(|conn| {
                let project_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
                    .unwrap_or(0);
                let chunk_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
                    .unwrap_or(0);
                Ok((project_count as i32, chunk_count as i64))
            })
            .map_err(|e| Status::internal(format!("failed to get stats: {e}")))?
        };

        // Get write queue stats
        let write_queue_stats = self.state.write_queue.stats();
        let write_queue = Some(WriteQueueStats {
            pending_writes: write_queue_stats.pending as i32,
            batched_last_flush: write_queue_stats.flushed as i32,
            avg_latency_ms: 0.0,
        });

        Ok(Response::new(ServerStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_seconds: 0,
            active_runs: 0,
            total_projects,
            total_chunks,
            total_summaries: 0,
            write_queue,
            summarize: None,
        }))
    }

    type StreamEventsStream = tokio_stream::wrappers::ReceiverStream<Result<RunEvent, Status>>;

    async fn stream_events(
        &self,
        _request: Request<()>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        // TODO: Implement event streaming
        Err(Status::unimplemented("stream_events not yet implemented"))
    }
}

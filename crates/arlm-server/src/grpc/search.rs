//! Search and context-building RPCs: `Search`, `BuildContext`.
//!
//! Both use the `chunks_fts` FTS5 index over `chunk_texts`, joined against
//! `chunks` metadata and filtered by the project buffer. Queries are sanitised
//! before being passed to the FTS5 MATCH operator to avoid injection errors.

use std::time::Instant;

use arlm_proto::proto::*;
use arlm_storage::Storage;
use rusqlite::{Connection, params};
use tonic::{Response, Status};

use crate::grpc::error::{internal, invalid_arg, not_found};
use crate::state::AppState;
use crate::store;

/// Map a project reference (UUID or name) to its numeric buffer id.
async fn buffer_id_for(state: &AppState, project: &str) -> Result<Option<i64>, Status> {
    let project_owned = project.to_string();
    let storage = state.storage.clone();
    store::blocking(move || store::buffer_id_for_project(&storage, &project_owned))
        .await
        .map_err(internal)
        .map_err(internal)
}

/// Run the FTS5 BM25 query returning hydrated results for a buffer.
fn bm25_search(
    conn: &Connection,
    buffer_id: i64,
    query: &str,
    limit: i64,
) -> Result<Vec<SearchResult>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.file_path, c.line_start, c.line_end, bm25(chunks_fts) AS score, \
                COALESCE(cc.content, '') \
         FROM chunks_fts \
         JOIN chunks c ON c.id = chunks_fts.rowid \
         LEFT JOIN chunk_texts cc ON cc.chunk_id = c.id \
         WHERE chunks_fts.content MATCH ?1 AND c.buffer_id = ?2 \
         ORDER BY score \
         LIMIT ?3",
    )?;

    let rows = stmt.query_map(params![query, buffer_id, limit], |row| {
        Ok(SearchResult {
            chunk_id: row.get(0)?,
            file_path: row.get(1)?,
            start_line: row.get(2)?,
            end_line: row.get(3)?,
            score: row.get(4)?,
            text: row.get(5)?,
            is_summary: false,
            summary: None,
        })
    })?;

    rows.collect()
}

/// Sanitise a user query for FTS5 `MATCH`: keep only alphanumeric and
/// whitespace, collapsing everything else to a space.
fn sanitize_fts(query: &str) -> String {
    query
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect()
}

/// Search chunks in a project with BM25 ranking.
///
/// # Errors
///
/// Returns an error if storage access fails or the query is invalid.
pub(crate) async fn handle_search(
    state: &AppState,
    req: SearchRequest,
) -> Result<Response<SearchResponse>, Status> {
    let start = Instant::now();
    let project = req.project;
    let query = req.query;

    if query.trim().is_empty() {
        return Err(invalid_arg("search query is required"));
    }

    let buffer_id = buffer_id_for(state, &project)
        .await?
        .ok_or_else(|| not_found("project not found"))?;

    let max_results = if req.max_results > 0 {
        i64::from(req.max_results)
    } else {
        10
    };

    let fts_query = sanitize_fts(&query);
    let storage = state.storage.clone();
    let results = tokio::task::spawn_blocking(move || {
        let conn = storage.connection()?;
        conn.execute(|conn| {
            if let Ok(limit_rows) = conn.query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| {
                r.get::<_, i64>(0)
            }) {
                if limit_rows == 0 {
                    sync_fts(conn)?;
                }
            }
            bm25_search(conn, buffer_id, &fts_query, max_results).map_err(anyhow::Error::from)
        })
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;

    tracing::info!(
        project = %project,
        results = results.len(),
        elapsed_ms = start.elapsed().as_millis(),
        "search completed"
    );

    let total_count = i32::try_from(results.len()).unwrap_or(i32::MAX);
    Ok(Response::new(SearchResponse {
        results,
        total_count,
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
    }))
}

/// Ensure chunks_fts matches chunk_texts (repopulate when out of sync).
fn sync_fts(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM chunks_fts", [])?;
    let _ = conn.execute(
        "INSERT INTO chunks_fts(rowid, content) SELECT chunk_id, content FROM chunk_texts",
        [],
    )?;
    Ok(())
}

/// Build an LLM-ready context from the top relevant chunks of a project.
///
/// # Errors
///
/// Returns an error if storage access fails or the project is unknown.
pub(crate) async fn handle_build_context(
    state: &AppState,
    req: ContextRequest,
) -> Result<Response<ContextResponse>, Status> {
    let start = Instant::now();
    let project = req.project;
    let task = req.task;

    if task.trim().is_empty() {
        return Err(invalid_arg("task is required"));
    }

    let buffer_id = buffer_id_for(state, &project)
        .await?
        .ok_or_else(|| not_found("project not found"))?;

    let max_tokens = u32::try_from(req.max_tokens).unwrap_or(8_000);

    let fts_query = sanitize_fts(&task);
    let storage = state.storage.clone();
    let ctx = tokio::task::spawn_blocking(move || {
        assemble_context(&storage, buffer_id, &fts_query, max_tokens)
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;

    tracing::info!(
        project = %project,
        chunks = ctx.sources.len(),
        total_tokens = ctx.stats.as_ref().map_or(0, |s| s.total_tokens),
        elapsed_ms = start.elapsed().as_millis(),
        "build_context completed"
    );

    Ok(Response::new(ctx))
}

/// Assemble the context payload: markdown-style prose + sources + stats.
fn assemble_context(
    storage: &Storage,
    buffer_id: i64,
    query: &str,
    max_tokens: u32,
) -> anyhow::Result<ContextResponse> {
    let conn = storage.connection()?;
    let results = conn.execute(|conn| {
        if let Ok(limit_rows) = conn.query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| {
            r.get::<_, i64>(0)
        }) {
            if limit_rows == 0 {
                sync_fts(conn)?;
            }
        }
        // Fetch a generous candidate pool so the budget keeps the best matches.
        let results = bm25_search(conn, buffer_id, query, 50)?;
        let mut sources = Vec::with_capacity(results.len());
        let mut budget_used: u32 = 0;
        let mut body = String::from("# Project Context\n\n");

        for r in &results {
            let tokens = estimate_tokens(&r.text);
            if tokens > 0 && budget_used + tokens > max_tokens {
                continue;
            }
            budget_used += tokens;
            use std::fmt::Write as _;
            let _ = write!(
                body,
                "## {} (score {:.2})\n```\n{}\n```\n\n",
                r.file_path, r.score, r.text
            );
            sources.push(r.clone());
        }

        Ok((body, sources, budget_used))
    })?;

    let (body, sources, budget_used) = results;
    let raw_chunks = sources.len();

    Ok(ContextResponse {
        context: body,
        sources,
        stats: Some(ContextStats {
            total_tokens: i32::try_from(budget_used).unwrap_or(i32::MAX),
            raw_chunks_included: i32::try_from(raw_chunks).unwrap_or(i32::MAX),
            summary_chunks_included: 0,
            summary_ratio: 0.0,
        }),
    })
}

/// Rough token estimate: 1 token per 4 ASCII characters.
fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32).saturating_div(4)
}

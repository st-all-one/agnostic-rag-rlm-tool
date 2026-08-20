//! Indexing RPC: `IndexProject`.
//!
//! Orchestrates full project ingestion: file discovery → deterministic
//! chunking → entity extraction → SQLite persistence (chunks + chunk_texts +
//! FTS + entities) → vector persistence (LanceDB) → buffer count update.

use std::time::Instant;

use arlm_embedding::embedder::Embedder;
use arlm_embedding::embedder::fallback::FallbackEmbedder;
use arlm_embedding::pipeline::discover_files;
use arlm_proto::proto::*;
use arlm_storage::VectorEntry;
use rayon::prelude::*;
use tonic::{Response, Status};

use crate::grpc::error::{internal, not_found};
use crate::indexing;
use crate::state::AppState;
use crate::store;

const EMBEDDING_DIMS: i64 = 1024;
const EMBEDDING_MODEL: &str = "fallback-hash";

/// Index all text files of a project.
///
/// # Errors
///
/// Returns an error if the project is unknown or any persistence step fails.
pub(crate) async fn handle_index_project(
    state: &AppState,
    req: IndexRequest,
) -> Result<Response<IndexResponse>, Status> {
    let start = Instant::now();
    let project = req.project.clone();

    let project_storage = state.storage.clone();
    let project_for_buffer = project.clone();
    let buffer_id =
        store::blocking(move || store::buffer_id_for_project(&project_storage, &project_for_buffer))
            .await
            .map_err(internal)?
            .ok_or_else(|| not_found("project not found"))?;

    let root = std::path::PathBuf::from(&req.root_path);
    if !root.is_dir() {
        return Err(Status::invalid_argument(format!(
            "invalid root_path: {}",
            req.root_path
        )));
    }

    // Phase 1: discover files (blocking I/O on the blocking pool).
    let excludes = req.exclude_patterns.clone();
    let root_clone = root.clone();
    let files = tokio::task::spawn_blocking(move || discover_files(&root_clone, &excludes))
        .await
        .map_err(internal)?
        .map_err(internal)?;

    tracing::info!(project = %project, files = files.len(), "indexing discovered files");

    // Phase 2: read + chunk + hash in parallel (CPU bound).
    let chunks: Vec<(String, Vec<indexing::IndexedChunk>)> = files
        .par_iter()
        .filter_map(|path| {
            let content = std::fs::read_to_string(path).ok()?;
            Some((
                path.to_string_lossy().into_owned(),
                indexing::index_file(path, &content),
            ))
        })
        .collect();

    let distinct_files = chunks.len();
    let total_chunks: usize = chunks.iter().map(|(_, cs)| cs.len()).sum();
    tracing::info!(project = %project, distinct_files, total_chunks, "chunked project files");

    // Phase 3: persist chunks + texts + FTS + entities + collect (id, content)
    // pairs for the optional vector pass.
    let storage = state.storage.clone();
    let persisted: Vec<(i64, String)> = store::blocking(move || {
        let mut persisted = Vec::with_capacity(total_chunks);
        for (_, file_chunks) in &chunks {
            for c in file_chunks {
                let hash_bytes = hex::decode(&c.hash).unwrap_or_default();
                let lang = c.language.as_deref();
                let chunk_type = Some(c.chunk_type.as_str());
                let chunk_id = store::insert_chunk(
                    &storage,
                    buffer_id,
                    &c.file_path,
                    c.line_start,
                    c.line_end,
                    &hash_bytes,
                    lang,
                    chunk_type,
                    Some(0),
                )?;
                store::insert_chunk_text(&storage, chunk_id, &c.content)?;
                store::insert_fts_row(&storage, chunk_id, &c.content)?;
                let entities = arlm_storage::Storage::extract_entities(&c.content, &c.file_path);
                store::insert_entities(&storage, chunk_id, &entities)?;
                persisted.push((chunk_id, c.content.clone()));
            }
        }
        Ok(persisted)
    })
    .await
    .map_err(internal)?;

    // Phase 4: persist vectors to LanceDB when available.
    if let Some(vector_store) = &state.vector_store {
        let embedder = FallbackEmbedder::new(EMBEDDING_DIMS as usize);
        let mut entries = Vec::with_capacity(persisted.len());
        for (chunk_id, content) in &persisted {
            let vector = embedder.embed(content).map_err(internal)?;
            entries.push(VectorEntry {
                chunk_id: u64::try_from(*chunk_id).unwrap_or(u64::MAX),
                buffer_id: u64::try_from(buffer_id).unwrap_or(u64::MAX),
                vector,
            });
        }
        if let Err(e) = vector_store.insert_vectors(&entries).await {
            tracing::error!(error = %e, "failed to persist vectors, indexing continues");
        }
    }

    // Phase 5: refresh buffer aggregate counts.
    let storage = state.storage.clone();
    store::blocking(move || {
        store::update_buffer_counts(
            &storage,
            buffer_id,
            i64::try_from(total_chunks).unwrap_or(i64::MAX),
            i64::try_from(distinct_files).unwrap_or(i64::MAX),
            EMBEDDING_MODEL,
            EMBEDDING_DIMS,
        )
    })
    .await
    .map_err(internal)?;

    tracing::info!(
        project = %project,
        files_indexed = distinct_files,
        chunks_created = total_chunks,
        elapsed_ms = start.elapsed().as_millis(),
        "project indexed"
    );

    Ok(Response::new(IndexResponse {
        run_id: uuid::Uuid::now_v7().to_string(),
        files_indexed: i64::try_from(distinct_files).unwrap_or(i64::MAX),
        chunks_created: i64::try_from(total_chunks).unwrap_or(i64::MAX),
        summaries_generated: 0,
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
    }))
}
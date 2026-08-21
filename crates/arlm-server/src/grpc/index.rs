//! Indexing RPC: `IndexProject` (client-streaming).
//!
//! The client discovers and reads files from its OWN filesystem, then streams
//! each file's content here. This handler never touches the client's
//! filesystem — it only receives bytes over gRPC, chunks them deterministically,
//! hashes, extracts entities and persists to SQLite + (optionally) LanceDB.
//! Removing server-side path knowledge closes the arbitrary-file-read footgun
//! described in the security review.

use std::path::Path;
use std::time::Instant;

use arlm_storage::VectorEntry;
use futures::stream::{self, StreamExt};
use tonic::{Request, Response, Status, Streaming};

use arlm_proto::proto::index_chunk;
use arlm_proto::proto::*;

use crate::grpc::error::internal;
use crate::indexing;
use crate::state::AppState;
use crate::store;

/// Default number of chunks per embedding request when `ARLM_EMBED_BATCH` is
/// unset. Matches the Ollama server's internal `OLLAMA_BATCH_SIZE`.
const DEFAULT_EMBED_BATCH: usize = 64;

/// Default number of concurrent embedding batches when `ARLM_INDEX_CONCURRENCY`
/// is unset. Should track Ollama's `OLLAMA_NUM_PARALLEL`.
const DEFAULT_INDEX_CONCURRENCY: usize = 4;

/// Decode a streamed file's content, transparently decompressing if the client
/// sent it zstd-compressed.
fn decode_content(file: &IndexFile) -> Result<String, Status> {
    let bytes = if file.compressed {
        zstd::stream::decode_all(&mut &file.content[..]).map_err(internal)?
    } else {
        file.content.clone()
    };
    String::from_utf8(bytes).map_err(internal)
}

/// Index a project from a client stream of file bytes.
///
/// # Errors
///
/// Returns an error if the stream is malformed, the project is unknown, or any
/// persistence step fails.
pub(crate) async fn handle_index_project(
    state: &AppState,
    request: Request<Streaming<IndexChunk>>,
) -> Result<Response<IndexResponse>, Status> {
    let start = Instant::now();
    let mut stream = request.into_inner();

    let mut project: Option<String> = None;
    let mut buffer_id: Option<i64> = None;
    let mut chunks: Vec<(String, Vec<indexing::IndexedChunk>)> = Vec::new();
    let mut distinct_files: usize = 0;

    while let Some(msg) = stream.message().await.map_err(internal)? {
        match msg.body {
            Some(index_chunk::Body::Init(init)) => {
                project = Some(init.project.clone());
                let pid = store::ensure_project(&state.storage, &init.project, &init.root_path)
                    .map_err(internal)?;
                buffer_id = Some(pid);
            }
            Some(index_chunk::Body::File(file)) => {
                let content = decode_content(&file)?;
                let chunk_list = indexing::index_file(Path::new(&file.rel_path), &content);
                distinct_files += 1;
                chunks.push((file.rel_path.clone(), chunk_list));
            }
            None => {}
        }
    }

    let project = project
        .ok_or_else(|| Status::invalid_argument("index stream did not send an init message"))?;
    let buffer_id =
        buffer_id.ok_or_else(|| Status::invalid_argument("index stream missing init"))?;

    let total_chunks: usize = chunks.iter().map(|(_, cs)| cs.len()).sum();

    // Phase 1: persist chunks + texts + FTS + entities.
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

    // Phase 2: persist vectors to LanceDB when available.
    if let Some(vector_store) = &state.vector_store {
        // Embedding batches and concurrency are tunable via env so the Docker
        // image can be dialed to match Ollama's OLLAMA_NUM_PARALLEL without a
        // rebuild (see OLLAMA_EMBED_PROPOSED.md).
        let embed_batch = std::env::var("ARLM_EMBED_BATCH")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_EMBED_BATCH);
        let concurrency = std::env::var("ARLM_INDEX_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_INDEX_CONCURRENCY);

        let embedder = state.embedder.clone();
        let buffer_id_u = u64::try_from(buffer_id).unwrap_or(u64::MAX);

        // Split the persisted chunks into batches and embed each batch
        // concurrently. `ureq` (Ollama HTTP client) is synchronous, so each
        // batch runs inside `spawn_blocking`; `buffer_unordered` bounds the
        // number of in-flight blocking tasks to `concurrency`.
        let batches: Vec<Vec<(i64, String)>> =
            persisted.chunks(embed_batch).map(|c| c.to_vec()).collect();

        let results = stream::iter(batches)
            .map(|batch| {
                let emb = embedder.clone();
                tokio::task::spawn_blocking(move || {
                    let texts: Vec<&str> = batch.iter().map(|(_, c)| c.as_str()).collect();
                    emb.embed_batch(&texts).map(|vectors| {
                        // Ollama preserves input order, so zipping is safe.
                        batch
                            .into_iter()
                            .zip(vectors)
                            .map(|((cid, _), v)| VectorEntry {
                                chunk_id: u64::try_from(cid).unwrap_or(u64::MAX),
                                buffer_id: buffer_id_u,
                                vector: v,
                            })
                            .collect::<Vec<_>>()
                    })
                })
            })
            .buffer_unordered(concurrency)
            .collect::<Vec<_>>()
            .await;

        let mut entries: Vec<VectorEntry> = Vec::with_capacity(persisted.len());
        for r in results {
            match r {
                Ok(Ok(mut ves)) => entries.append(&mut ves),
                Ok(Err(e)) => tracing::warn!(error = %e, "batch embedding failed"),
                Err(e) => tracing::warn!(error = %e, "embedding task panicked"),
            }
        }

        if let Err(e) = vector_store.insert_vectors(&entries).await {
            tracing::error!(error = %e, "failed to persist vectors, indexing continues");
        }
    }

    // Phase 3: bump aggregate counts by this stream's contribution.
    let storage = state.storage.clone();
    let embedding_model = state.embedder.name().to_string();
    let embedding_dims = state.embedder.dimensions() as i64;
    store::blocking(move || {
        store::increment_buffer_counts(
            &storage,
            buffer_id,
            i64::try_from(total_chunks).unwrap_or(i64::MAX),
            i64::try_from(distinct_files).unwrap_or(i64::MAX),
            &embedding_model,
            embedding_dims,
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

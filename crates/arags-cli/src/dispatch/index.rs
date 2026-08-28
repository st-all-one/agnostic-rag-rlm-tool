//! Upload of files to the server: discovery → partitioning → streaming.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use arags_proto::proto::index_chunk;

use crate::auth_client::AragsClient;
use crate::output::Format;
use tokio::runtime::Runtime;
use tracing::{info, warn};

use super::discover::discover_files;

/// Default zstd compression level for uploads (level 3 ≈ best speed/ratio
/// balance for source text).
const UPLOAD_ZSTD_LEVEL: i32 = 3;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_index(
    rt: &Runtime,
    client: &mut AragsClient,
    project_path: &Path,
    canonical_name: &str,
    ignore_patterns: &[String],
    force_include: &[String],
    format: Format,
) -> Result<()> {
    let absolute = std::fs::canonicalize(project_path)
        .with_context(|| format!("failed to resolve path: {}", project_path.display()))?;
    let project_str = canonical_name.to_string();

    // Combine CLI ignore patterns with the project's `.arags.toml` ignore list
    // (and the `ARAGS_INDEX_IGNORE` env var).
    let mut ignore = ignore_patterns.to_vec();
    ignore.extend(
        crate::user_config::load()
            .map(|c| c.index_ignore_patterns())
            .unwrap_or_default(),
    );

    let files = discover_files(&absolute, &ignore, force_include)
        .map_err(|e| anyhow::anyhow!("file discovery failed: {e}"))?;

    if files.is_empty() {
        let rendered = match format {
            Format::FullJson => crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({ "files_indexed": 0, "chunks_created": 0 }))
                .to_json_string(),
            _ => format!("No files to index in {}", absolute.display()),
        };
        print!("{rendered}");
        return Ok(());
    }

    let parallelism = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .clamp(1, 8);
    let groups = partition_files(&files, parallelism);
    let total = files.len() as u64;

    let progress = Arc::new(indicatif::ProgressBar::new(total));
    progress.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} files ({eta})")
            .map_err(|e| anyhow::anyhow!("invalid progress template: {e}"))?,
    );
    progress.set_message("Uploading");

    let mut totals = (0i64, 0i64);
    let mut handles = Vec::with_capacity(groups.len());
    let upload_start = Instant::now();
    for group in groups {
        let mut client = client.clone();
        let pb = progress.clone();
        let project = project_str.clone();
        let root = absolute.clone();
        let handle = rt
            .spawn(async move { stream_index_group(&mut client, project, root, group, pb).await });
        handles.push(handle);
    }
    for handle in handles {
        let (files_idx, chunks_idx) = rt
            .block_on(handle)
            .map_err(|e| anyhow::anyhow!("upload task failed: {e}"))??;
        totals.0 += files_idx;
        totals.1 += chunks_idx;
    }
    progress.finish_and_clear();
    info!(
        duration_ms = %upload_start.elapsed().as_millis(),
        files = totals.0,
        chunks = totals.1,
        "index pass complete"
    );

    let rendered = match format {
        Format::FullJson => crate::output::json::JsonOutput::ok()
            .with_data(serde_json::json!({
                "files_indexed": totals.0,
                "chunks_created": totals.1,
            }))
            .to_json_string(),
        _ => format!("Indexed {} files, {} chunks", totals.0, totals.1),
    };
    print!("{rendered}");
    Ok(())
}

/// Stream one disjoint group of files to the server as a single `IndexProject`
/// client-stream, returning the files/chunks counts reported by the server.
///
/// Each file's raw text is **zstd-compressed** before sending (the server
/// transparently decompresses; plan 020 D2 keeps chunking server-side).
pub(crate) async fn stream_index_group(
    client: &mut AragsClient,
    project: String,
    root: PathBuf,
    files: Vec<PathBuf>,
    progress: Arc<indicatif::ProgressBar>,
) -> anyhow::Result<(i64, i64)> {
    let (tx, rx) = mpsc::channel::<arags_proto::proto::IndexChunk>(32);
    let stream = ReceiverStream::new(rx);
    let response_fut = client.index_project(stream);
    let start = Instant::now();

    let send_handle = tokio::spawn(async move {
        if tx
            .send(arags_proto::proto::IndexChunk {
                body: Some(index_chunk::Body::Init(arags_proto::proto::IndexInit {
                    project,
                    root_path: root.to_string_lossy().to_string(),
                    force_include: vec![],
                    exclude_patterns: vec![],
                })),
            })
            .await
            .is_err()
        {
            return;
        }

        for file in &files {
            let Ok(content) = std::fs::read_to_string(file) else {
                progress.inc(1);
                continue;
            };
            let rel_path = file
                .strip_prefix(&root)
                .unwrap_or(file)
                .to_string_lossy()
                .to_string();
            // Compress when possible; fall back to raw bytes (flagged as
            // uncompressed) if encoding unexpectedly fails.
            let (content_bytes, compressed) =
                match zstd::stream::encode_all(content.as_bytes(), UPLOAD_ZSTD_LEVEL) {
                    Ok(c) => (c, true),
                    Err(e) => {
                        warn!(error = %e, path = %rel_path, "zstd encode failed; sending raw");
                        (content.as_bytes().to_vec(), false)
                    }
                };
            let size = i64::try_from(content_bytes.len()).unwrap_or(i64::MAX);
            if tx
                .send(arags_proto::proto::IndexChunk {
                    body: Some(index_chunk::Body::File(arags_proto::proto::IndexFile {
                        rel_path,
                        content: content_bytes,
                        compressed,
                        size_bytes: size,
                    })),
                })
                .await
                .is_err()
            {
                break;
            }
            progress.inc(1);
        }
    });

    let response = response_fut
        .await
        .map_err(|e| anyhow::anyhow!("index stream failed: {e}"))?;
    info!(
        duration_ms = %start.elapsed().as_millis(),
        "index_project stream complete"
    );
    send_handle
        .await
        .map_err(|e| anyhow::anyhow!("upload task failed: {e}"))?;

    let inner: arags_proto::proto::IndexResponse = response.into_inner();
    Ok((inner.files_indexed, inner.chunks_created))
}

/// Split `files` into `n` roughly equal, disjoint groups for parallel upload.
#[must_use]
pub(crate) fn partition_files(files: &[PathBuf], n: usize) -> Vec<Vec<PathBuf>> {
    let n = n.max(1).min(files.len().max(1));
    let mut groups: Vec<Vec<PathBuf>> = (0..n).map(|_| Vec::new()).collect();
    for (i, file) in files.iter().enumerate() {
        groups[i % n].push(file.clone());
    }
    groups.retain(|g| !g.is_empty());
    groups
}

#[cfg(test)]
mod tests;

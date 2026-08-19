use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::{data_dir, project_name};

pub struct IndexConfig<'a> {
    pub path: &'a Path,
    pub chunk_size: usize,
    pub ignore_patterns: &'a [String],
    pub watch: bool,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
}

#[allow(clippy::needless_pass_by_value)]
pub fn execute(config: IndexConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_index");

    let absolute_path = config
        .path
        .canonicalize()
        .with_context(|| format!("failed to resolve path: {}", config.path.display()))?;

    let pname = project_name(config.project);

    if config.verbose {
        output::info(&format!("Indexing {}...", absolute_path.display()));
    }

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let buffer = storage
        .get_buffer_by_name(&pname)
        .context("failed to check buffer")?;

    let _buffer_id = if let Some(buf) = buffer {
        buf.id
    } else {
        storage
            .insert_buffer(&arlm_storage::sqlite::buffers::NewBuffer {
                name: pname.clone(),
                path: absolute_path.to_string_lossy().to_string(),
            })
            .context("failed to create buffer")?
    };

    let knowledge = arlm_memory::KnowledgeEngine::new(storage);
    let opts = arlm_memory::knowledge::IndexOptions {
        max_chunk_bytes: config.chunk_size * 4,
        ignore_patterns: config.ignore_patterns.to_vec(),
        ..Default::default()
    };

    let progress = indicatif::ProgressBar::new_spinner();
    progress.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} [{elapsed_precise}]")
            .map_err(|e| anyhow::anyhow!("invalid template: {e}"))?,
    );
    progress.set_message("Indexing files...");

    let result = knowledge
        .index_directory(&pname, &absolute_path, &opts)
        .context("failed to index directory")?;

    // Auto-populate FTS5 index after indexing
    let bm25 = arlm_search::Bm25Search::new(knowledge.storage())
        .context("failed to create BM25 search for FTS population")?;
    let fts_count = bm25.populate_fts().context("failed to populate FTS index")?;

    progress.finish_and_clear();

    match config.format {
        Format::Json => {
            let dur: u64 = result.duration_ms.try_into().unwrap_or(u64::MAX);
            let output = crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({
                    "project": pname,
                    "path": absolute_path.display().to_string(),
                    "files_processed": result.files_processed,
                    "chunks_created": result.chunks_created,
                    "duration_ms": dur,
                }))
                .with_metadata("duration_ms", dur);
            output.print();
        }
        Format::Tree => {
            let dur_ms = u64::try_from(result.duration_ms).unwrap_or(u64::MAX);
            #[allow(clippy::cast_precision_loss)]
            let dur_secs = dur_ms as f64 / 1000.0;
            output::success(&format!(
                "Indexed {} files → {} chunks in {dur_secs:.1}s",
                result.files_processed, result.chunks_created,
            ));
            output::info(&format!("Database: {}/knowledge.db", data_dir().display()));
        }
        Format::Markdown => {
            let dur_ms = u64::try_from(result.duration_ms).unwrap_or(u64::MAX);
            #[allow(clippy::cast_precision_loss)]
            let dur_secs = dur_ms as f64 / 1000.0;
            println!(
                "# Indexing Complete\n\n- **Files:** {}\n- **Chunks:** {}\n- **Duration:** {dur_secs:.1}s\n",
                result.files_processed, result.chunks_created,
            );
        }
        Format::Prompt => {
            println!(
                "Indexed {} files into {} chunks. Project: {}.",
                result.files_processed, result.chunks_created, pname,
            );
        }
    }

    // Watch mode: monitor for changes and reindex
    if config.watch {
        use arlm_memory::watch::{WatchMonitor, WatchOptions};

        output::info("Watching for file changes... (Ctrl+C to stop)");

        let watch_opts = WatchOptions {
            debounce_ms: 500,
            recursive: true,
        };

        let handle = WatchMonitor::watch(&absolute_path, &watch_opts)
            .context("failed to start file watcher")?;

        loop {
            match handle.recv() {
                Ok(event) => {
                    let is_relevant = event.paths.iter().any(|p| {
                        p.extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|ext| {
                                matches!(ext, "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" | "rb" | "md" | "txt" | "json" | "yaml" | "toml")
                            })
                    });

                    if !is_relevant {
                        continue;
                    }

                    output::info(&format!(
                        "Detected {} change(s), reindexing...",
                        event.paths.len()
                    ));

                    let storage = arlm_storage::Storage::open(&data_dir())
                        .context("failed to open storage")?;

                    let knowledge = arlm_memory::KnowledgeEngine::new(storage);
                    let opts = arlm_memory::knowledge::IndexOptions {
                        max_chunk_bytes: config.chunk_size * 4,
                        ignore_patterns: config.ignore_patterns.to_vec(),
                        ..Default::default()
                    };

                    match knowledge.index_directory(&pname, &absolute_path, &opts) {
                        Ok(result) => {
                            // Re-populate FTS after reindex
                            if let Ok(bm25) = arlm_search::Bm25Search::new(knowledge.storage()) {
                                let _ = bm25.populate_fts();
                            }
                            output::success(&format!(
                                "Reindexed {} files → {} chunks",
                                result.files_processed, result.chunks_created,
                            ));
                        }
                        Err(e) => {
                            output::error(&format!("Reindex failed: {e}"));
                        }
                    }
                }
                Err(e) => {
                    output::error(&format!("Watch error: {e}"));
                    break;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_index_empty_dir() {
        let tmp = TempDir::new().unwrap();
        // SAFETY: test-only, single-threaded
        unsafe { std::env::set_var("ARLM_DATA_DIR", tmp.path()) };
        let project = TempDir::new().unwrap();
        let project_path = tmp.path().join("test-project");
        let config = IndexConfig {
            path: project.path(),
            chunk_size: 512,
            ignore_patterns: &[],
            watch: false,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
        };
        let result = execute(config);
        assert!(result.is_ok());
    }
}

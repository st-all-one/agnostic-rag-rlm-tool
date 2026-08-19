use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::project_dirs;

pub struct IndexConfig<'a> {
    pub path: &'a Path,
    pub chunk_size: usize,
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

    let project_name = config
        .project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let data_dir = project_dirs().join(project_name);

    if config.verbose {
        output::info(&format!("Indexing {}...", absolute_path.display()));
    }

    let storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(project_name)
        .context("failed to check buffer")?;

    let _buffer_id = if let Some(buf) = buffer {
        buf.id
    } else {
        storage
            .insert_buffer(&arlm_storage::sqlite::buffers::NewBuffer {
                name: project_name.to_string(),
                path: absolute_path.to_string_lossy().to_string(),
            })
            .context("failed to create buffer")?
    };

    let knowledge = arlm_memory::KnowledgeEngine::new(storage);
    let opts = arlm_memory::knowledge::IndexOptions {
        max_chunk_bytes: config.chunk_size * 4,
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
        .index_directory(project_name, &absolute_path, &opts)
        .context("failed to index directory")?;

    progress.finish_and_clear();

    match config.format {
        Format::Json => {
            let dur: u64 = result.duration_ms.try_into().unwrap_or(u64::MAX);
            let output = crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({
                    "project": project_name,
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
                result.files_processed,
                result.chunks_created,
            ));
            output::info(&format!("Database: {}/knowledge.db", data_dir.display()));
        }
        Format::Markdown => {
            let dur_ms = u64::try_from(result.duration_ms).unwrap_or(u64::MAX);
            #[allow(clippy::cast_precision_loss)]
            let dur_secs = dur_ms as f64 / 1000.0;
            println!(
                "# Indexing Complete\n\n- **Files:** {}\n- **Chunks:** {}\n- **Duration:** {dur_secs:.1}s\n",
                result.files_processed,
                result.chunks_created,
            );
        }
        Format::Prompt => {
            println!(
                "Indexed {} files into {} chunks. Project: {}.",
                result.files_processed, result.chunks_created, project_name,
            );
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
        let project = TempDir::new().unwrap();
        let project_path = tmp.path().join("test-project");
        let config = IndexConfig {
            path: project.path(),
            chunk_size: 512,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
        };
        let result = execute(config);
        assert!(result.is_ok());
    }
}

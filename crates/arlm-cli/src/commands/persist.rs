use std::path::Path;

use anyhow::Result;
use arlm_memory::AnalysisPersistOptions;
use arlm_memory::PersistEngine;
use arlm_memory::SearchPersistOptions;
use tracing::info;

use crate::output::Format;
use crate::util::project_name;

pub struct PersistArgs<'a> {
    pub title: Option<String>,
    pub query: Option<String>,
    pub project: &'a Path,
    pub format: Format,
}

pub fn execute(args: PersistArgs<'_>) -> Result<()> {
    let engine = PersistEngine::new(args.project)?;

    let title = args.title.unwrap_or_else(|| "Manual persist".to_string());
    let query = args.query.unwrap_or_default();

    let result = engine.persist_search(&SearchPersistOptions {
        query,
        tier: "manual".to_string(),
        project: title.clone(),
        entities: Vec::new(),
        tags: vec!["manual".to_string()],
        body: format!("# {title}\n\nPersisted manually via `arlm persist`."),
    })?;

    let output = serde_json::json!({
        "path": result.path.to_string_lossy(),
        "relative_path": result.relative_path,
    });

    match args.format {
        crate::output::Format::FullJson | Format::Jsonl => {
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("Persisted to: {}", result.relative_path);
        }
    }

    Ok(())
}

/// Persist arbitrary command output (`content`) as a wiki page.
///
/// Wraps `content` in a markdown article titled `title` and writes it to the
/// project's wiki under the `analyses` scope. `content` is embedded as a code
/// block when the requested `format` is JSON, otherwise it is written
/// verbatim (it is already a rendered markdown/prompt document).
///
/// # Errors
///
/// Returns an error if the wiki directory cannot be created or the page
/// cannot be written.
pub fn save_page(title: &str, content: &str, project: &Path, format: Format) -> Result<()> {
    let engine = PersistEngine::new(project)?;
    let pname = project_name(project);

    let body = match format {
        Format::FullJson | Format::Jsonl => format!("# {title}\n\n```json\n{content}\n```\n"),
        Format::Path | Format::Markdown | Format::Text => format!("# {title}\n\n{content}"),
    };

    let result = engine.persist_analysis(&AnalysisPersistOptions {
        title: title.to_string(),
        project: pname,
        tags: vec!["cli".to_string(), "persisted".to_string()],
        body,
    })?;

    info!(path = %result.relative_path, title, "persisted page");
    Ok(())
}

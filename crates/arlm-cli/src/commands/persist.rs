use anyhow::Result;
use arlm_memory::PersistEngine;
use arlm_memory::SearchPersistOptions;

pub struct PersistArgs<'a> {
    pub title: Option<String>,
    pub query: Option<String>,
    pub project: &'a std::path::Path,
    pub format: crate::output::Format,
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
        crate::output::Format::Json => {
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("Persisted to: {}", result.relative_path);
        }
    }

    Ok(())
}

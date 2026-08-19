use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::project_dirs;

pub fn execute_create(title: &str, project: &Path, format: Format) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_session_create");

    let project_name = project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let storage = open_storage(project)?;
    let mgr = arlm_memory::SessionManager::new(storage).context("failed to create session manager")?;
    let session_id = mgr
        .create(project_name, title)
        .context("failed to create session")?;

    match format {
        Format::Json => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "session_id": session_id,
                "project": project_name,
                "title": title,
            }));
            output.print();
        }
        Format::Tree => {
            output::success(&format!("Session created: {session_id}"));
            println!("  Project: {project_name}");
            println!("  Title: {title}");
        }
        Format::Markdown => {
            println!("# Session Created\n\n- **ID:** {session_id}\n- **Project:** {project_name}\n- **Title:** {title}");
        }
        Format::Prompt => {
            println!("Session created: {session_id} (project: {project_name}, title: {title})");
        }
    }

    Ok(())
}

pub fn execute_resume(session_id: &str, project: &Path, format: Format) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_session_resume");

    let storage = open_storage(project)?;
    let mgr = arlm_memory::SessionManager::new(storage).context("failed to create session manager")?;

    let session = mgr
        .get(session_id)
        .context("failed to get session")?
        .context("session not found")?;

    let contexts = mgr.get_contexts(session_id).context("failed to get contexts")?;
    let history = mgr.get_history(session_id, 10).context("failed to get history")?;

    match format {
        Format::Json => {
            let ctx: Vec<serde_json::Value> = contexts
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "version": c.version,
                        "created_at": c.created_at,
                    })
                })
                .collect();
            let hist: Vec<serde_json::Value> = history
                .iter()
                .map(|(q, r, t)| {
                    serde_json::json!({
                        "query": q,
                        "result": r,
                        "created_at": t,
                    })
                })
                .collect();
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "session_id": session.id,
                "project": session.project_name,
                "title": session.title,
                "contexts": ctx,
                "history": hist,
            }));
            output.print();
        }
        Format::Tree => {
            output::success(&format!("Session: {} ({})", session.id, session.title));
            println!("  Project: {}", session.project_name);
            println!("  Contexts: {}", contexts.len());
            println!("  History entries: {}", history.len());
            if let Some((q, r, _)) = history.first() {
                println!("\n  Latest query: {q}");
                if let Some(res) = r {
                    println!("  Result: {res}");
                }
            }
        }
        Format::Markdown => {
            println!(
                "# Session: {}\n\n- **Project:** {}\n- **Title:** {}\n",
                session.id, session.project_name, session.title
            );
        }
        Format::Prompt => {
            println!(
                "Session {} resumed. Project: {}. Contexts: {}. History: {} entries.",
                session.id,
                session.project_name,
                contexts.len(),
                history.len()
            );
        }
    }

    Ok(())
}

pub fn execute_list(project: &Path, format: Format) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_session_list");

    let storage = open_storage(project)?;
    let mgr =
        arlm_memory::SessionManager::new(storage).context("failed to create session manager")?;

    let conn = mgr.get_storage().conn();
    let conn = conn.lock();

    let mut stmt = conn
        .prepare(
            "SELECT id, project_name, title, created_at FROM sessions ORDER BY created_at DESC LIMIT 20",
        )
        .context("failed to prepare list query")?;

    let sessions: Vec<(String, String, String, i64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .filter_map(std::result::Result::ok)
        .collect();

    drop(stmt);
    drop(conn);

    match format {
        Format::Json => {
            let items: Vec<serde_json::Value> = sessions
                .iter()
                .map(|(id, proj, title, ts)| {
                    serde_json::json!({
                        "session_id": id,
                        "project": proj,
                        "title": title,
                        "created_at": ts,
                    })
                })
                .collect();
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "sessions": items,
                "count": sessions.len(),
            }));
            output.print();
        }
        Format::Tree => {
            if sessions.is_empty() {
                output::warn("No sessions found.");
            } else {
                output::success(&format!("{} session(s):", sessions.len()));
                for (id, proj, title, _) in &sessions {
                    println!("  {id} — {proj} — {title}");
                }
            }
        }
        Format::Markdown => {
            println!("# Sessions\n");
            if sessions.is_empty() {
                println!("No sessions found.");
            } else {
                for (id, proj, title, _) in &sessions {
                    println!("- **{id}** — {proj} — {title}");
                }
            }
        }
        Format::Prompt => {
            if sessions.is_empty() {
                println!("No sessions found.");
            } else {
                for (id, proj, title, _) in &sessions {
                    println!("  {id} ({proj}): {title}");
                }
            }
        }
    }

    Ok(())
}

fn open_storage(project: &Path) -> Result<arlm_storage::Storage> {
    let project_name = project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");
    let data_dir = project_dirs().join(project_name);
    arlm_storage::Storage::open(&data_dir).context("failed to open storage")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_session_create_and_list() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("test-proj");
        std::fs::create_dir_all(&project).unwrap();

        let result = execute_create("My Analysis", &project, Format::Json);
        assert!(result.is_ok());

        let result = execute_list(&project, Format::Json);
        assert!(result.is_ok());
    }
}

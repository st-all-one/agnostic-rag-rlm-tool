//! `arags init`: scaffold the local `.arags.toml` and optionally index.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::runtime::Runtime;

use crate::output::Format;
use crate::user_config::EffectiveUserConfig;

use super::connect;
use super::index::run_index;

pub(crate) fn run_init(
    rt: &Runtime,
    cfg: &EffectiveUserConfig,
    project: &Path,
    name: Option<String>,
    format: Format,
    do_index: bool,
) -> Result<()> {
    // Validate global identity (auth). The refresh token lives only in the
    // global `~/.arags/arags.toml`; we never copy it into the local file.
    match cfg.auth() {
        Some(auth) if auth.refresh_token.is_some() => {}
        _ => {
            bail!(
                "no global identity configured. Run `arags-server admin create-refresh` and \
                 store the token in `~/.arags/arags.toml` under `[auth]`."
            );
        }
    }

    // The canonical project name is a CONCEPTUAL entity, not a filesystem path
    // (issue `agnostic-rlm-rs-f5db`). It must be provided manually so that
    // worktrees of the same repo index into one shared knowledge buffer.
    let name = resolve_init_name(name, project)?;

    let local_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".arags.toml");

    if local_path.exists() {
        println!(
            "{} already exists; leaving it untouched.",
            local_path.display()
        );
    } else {
        let ignore = seed_ignore_from_gitignore();
        // No `[server]` section on purpose (agnostic-rlm-rs-152a): a
        // hardcoded localhost stamp would override the operator's global
        // `~/.arags/arags.toml` in the field-by-field merge. Absent here, the
        // merge falls back to the global addr (default `127.0.0.1:50051`).
        let content = toml::to_string_pretty(&LocalAragsToml {
            project: LocalProject {
                name: name.clone(),
                ignore: if ignore.is_empty() {
                    None
                } else {
                    Some(ignore)
                },
            },
        })
        .context("failed to serialize .arags.toml")?;
        std::fs::write(&local_path, content)
            .with_context(|| format!("failed to write {}", local_path.display()))?;
        println!("Created {}", local_path.display());
        append_gitignore(&local_path)?;
    }

    if do_index {
        let mut client = connect(rt, cfg)?;
        run_index(rt, &mut client, project, &name, &[], &[], format)?;
    } else {
        println!("Skipping index (--no-index). Run `arags index` to ingest.");
    }
    Ok(())
}

/// Resolve the canonical project name for `arags init`.
///
/// Priority: explicit `--name` flag → interactive prompt (TTY only, prefilled
/// with a suggestion) → hard error (non-interactive with no flag, since a name
/// must never be silently derived from the path).
fn resolve_init_name(name: Option<String>, project: &Path) -> Result<String> {
    let suggestion = suggest_project_name(project);
    let chosen = match name {
        Some(n) => n,
        None => {
            if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
                prompt_canonical_name(&suggestion)?
            } else {
                bail!(
                    "canonical project name required. Pass `--name <NAME>` (the project is a \
                     conceptual knowledge entity, not a path). Suggestion: {suggestion}"
                );
            }
        }
    };
    let trimmed = chosen.trim().to_string();
    if !crate::user_config::is_valid_canonical_name(&trimmed) {
        bail!(
            "invalid canonical project name {trimmed:?}: must be a logical identifier (e.g. \
             `my-service`), not `.`, `..`, or an absolute path."
        );
    }
    Ok(trimmed)
}

/// Prompt for the canonical project name on a TTY, offering `suggestion` as a
/// hint. Re-prompts on empty input; never silently falls back to the suggestion.
fn prompt_canonical_name(suggestion: &str) -> Result<String> {
    use std::io::Write as _;
    loop {
        print!("Project name (knowledge entity) [{suggestion}]: ");
        std::io::stdout()
            .flush()
            .context("failed to flush stdout")?;
        let mut buf = String::new();
        std::io::stdin()
            .read_line(&mut buf)
            .context("failed to read project name from stdin")?;
        let input = buf.trim();
        if !input.is_empty() {
            return Ok(input.to_string());
        }
        // Empty input: re-prompt rather than accept the suggestion silently.
        print!("A project name is required (not derived from the path): ");
        std::io::stdout()
            .flush()
            .context("failed to flush stdout")?;
    }
}

/// Local `.arags.toml` shape written by `arags init`.
#[derive(serde::Serialize)]
struct LocalAragsToml {
    project: LocalProject,
}

#[derive(serde::Serialize)]
struct LocalProject {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ignore: Option<Vec<String>>,
}

/// Best-effort project name *suggestion*: git remote, else directory basename.
/// Used only to prefill the interactive prompt — never applied as the value
/// (issue `agnostic-rlm-rs-f5db`).
#[must_use]
pub(crate) fn suggest_project_name(project: &Path) -> String {
    if let Ok(output) = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(project)
        .output()
    {
        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout);
            if let Some(name) = url
                .trim()
                .rsplit('/')
                .next()
                .and_then(|s| s.strip_suffix(".git"))
            {
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }
    project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

/// Seed ignore patterns from the project's `.gitignore`, if present.
fn seed_ignore_from_gitignore() -> Vec<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let gitignore = cwd.join(".gitignore");
    let Ok(content) = std::fs::read_to_string(&gitignore) else {
        return vec![
            ".git/".to_string(),
            "target/".to_string(),
            "node_modules/".to_string(),
        ];
    };
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Append `.arags.toml` to `.gitignore` (idempotent).
fn append_gitignore(local_path: &Path) -> Result<()> {
    use std::io::Write as _;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let gitignore = cwd.join(".gitignore");
    let entry = local_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".arags.toml");
    if let Ok(existing) = std::fs::read_to_string(&gitignore) {
        if existing.lines().any(|l| l.trim() == entry) {
            return Ok(());
        }
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&gitignore)
        .with_context(|| format!("failed to open {}", gitignore.display()))?;
    writeln!(f, "{entry}").context("failed to append to .gitignore")?;
    Ok(())
}

#[cfg(test)]
mod tests;

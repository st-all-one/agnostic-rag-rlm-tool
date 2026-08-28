//! `arags init`: a real bootstrap utility that scaffolds the local
//! `.arags.toml` (canonical project config) and optionally registers a watch
//! daemon / indexes.

use std::io::{IsTerminal, Write as _};
use std::path::Path;

use anyhow::{Context, Result, bail};
use tokio::runtime::Runtime;

use crate::output::Format;
use crate::user_config::{
    EffectiveUserConfig, LocalConfig, ProjectSection, ServerSection, WatchSection, load_local_at,
};

use super::connect;
use super::index::run_index;
use super::projects::run_register;

/// Resolved CLI flags for `arags init`.
#[derive(Debug, Clone, Default)]
pub(crate) struct InitFlags {
    /// Explicit canonical project name (knowledge entity).
    pub name: Option<String>,
    /// Explicit ignore globs (multi-value).
    pub ignore: Vec<String>,
    /// Explicit local server-address override written to `.arags.toml`.
    pub server_addr: Option<String>,
    /// Register the watch daemon now (`--register`/`--no-register`).
    pub register: bool,
    /// Index the project after writing the config (`--index`/`--no-index`).
    pub do_index: bool,
    /// Never prompt; fail if any required value is missing.
    pub non_interactive: bool,
}

/// Local `.arags.toml` shape written by `arags init`.
///
/// Only the fields the CLI may author are serialized; `[auth]` is intentionally
/// absent (global-only) and `[llm]` is never written here (it stays merged from
/// the global scope). `watch` is preserved verbatim on re-init so an existing
/// registration is not clobbered.
#[derive(serde::Serialize)]
pub(crate) struct InitWriteToml {
    project: ProjectSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<ServerSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    watch: Option<WatchSection>,
}

pub(crate) fn run_init(
    rt: &Runtime,
    cfg: &EffectiveUserConfig,
    project: &Path,
    flags: &InitFlags,
    format: Format,
) -> Result<()> {
    let local_path = project.join(".arags.toml");
    let existing = load_local_at(&local_path).unwrap_or_default();
    let gitignore_patterns = seed_ignore_from_gitignore(project);

    let is_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let interactive = !flags.non_interactive && is_tty;

    // Resolve the canonical name: explicit flag → existing config → interactive
    // prompt (TTY) → hard error (non-interactive with nothing to go on).
    let name = if let Some(n) = &flags.name {
        n.clone()
    } else if let Some(n) = existing.project.as_ref().and_then(|p| p.name.clone()) {
        n
    } else {
        resolve_init_name(None, project, flags.non_interactive)?
    };

    // Resolve the remaining fields: explicit flags win, then existing config,
    // then (interactive only) prompts, then gitignore-seeded defaults.
    let (ignore, server_addr, register) = if interactive {
        let default_ignore: Vec<String> = if !flags.ignore.is_empty() {
            flags.ignore.clone()
        } else if let Some(existing_ignore) = existing_ignore(&existing) {
            existing_ignore
        } else if !gitignore_patterns.is_empty() {
            gitignore_patterns.clone()
        } else {
            Vec::new()
        };
        let default_server = if let Some(s) = &flags.server_addr {
            s.clone()
        } else if let Some(s) = existing_server_addr(&existing) {
            s
        } else {
            cfg.server.addr.clone().unwrap_or_default()
        };
        let ignore = prompt_ignore(&default_ignore)?;
        let server_addr = prompt_server_addr(&default_server)?;
        let register = prompt_bool("Register watch daemon now?", flags.register)?;
        (ignore, server_addr, register)
    } else {
        let ignore = if !flags.ignore.is_empty() {
            flags.ignore.clone()
        } else if let Some(existing_ignore) = existing_ignore(&existing) {
            existing_ignore
        } else {
            gitignore_patterns.clone()
        };
        let server_addr = flags
            .server_addr
            .clone()
            .or_else(|| existing_server_addr(&existing));
        (ignore, server_addr, flags.register)
    };

    let write = build_init_write(&name, &ignore, server_addr.as_deref(), &existing);

    if interactive {
        let preview = toml::to_string_pretty(&write).context("failed to serialize preview")?;
        eprintln!("--- .arags.toml that will be written ---");
        eprintln!("{preview}----------------------------------------");
        if !prompt_bool("Write this configuration?", true)? {
            eprintln!("Aborted; nothing written.");
            return Ok(());
        }
    }

    let existed = local_path.exists();
    let content = toml::to_string_pretty(&write).context("failed to serialize .arags.toml")?;
    std::fs::write(&local_path, &content)
        .with_context(|| format!("failed to write {}", local_path.display()))?;
    if existed {
        eprintln!("Updated {}", local_path.display());
    } else {
        eprintln!("Created {}", local_path.display());
    }
    append_gitignore(project, &local_path)?;

    // On completion: optional health-check + identity-conflict hook. Both are
    // best-effort — a down server only warns (issue `agnostic-rag-rlm-tool-e5d8`).
    run_server_checks(rt, cfg, project, &name, flags.non_interactive)?;

    if register {
        run_register(project, &name)?;
    }

    if flags.do_index {
        match connect(rt, cfg) {
            Ok(mut client) => run_index(rt, &mut client, project, &name, &[], &[], format)?,
            Err(e) => {
                eprintln!(
                    "! skipping index: could not connect to server ({e}). Run `arags index` later."
                );
            }
        }
    } else {
        eprintln!("Skipping index (--no-index). Run `arags index` to ingest.");
    }
    Ok(())
}

/// Pure, testable merge of init inputs into the local `.arags.toml` shape.
///
/// Precedence for every field: explicit CLI value → existing local config →
/// (in the caller) gitignore-seeded / prompted default. The `[watch]` section
/// is carried over verbatim so a prior registration survives a re-init.
#[must_use]
pub(crate) fn build_init_write(
    cli_name: &str,
    cli_ignore: &[String],
    cli_server: Option<&str>,
    existing: &LocalConfig,
) -> InitWriteToml {
    let project = ProjectSection {
        name: Some(cli_name.to_string()),
        ignore: if cli_ignore.is_empty() {
            None
        } else {
            Some(cli_ignore.to_vec())
        },
    };
    let server = cli_server.map(|addr| ServerSection {
        addr: Some(addr.to_string()),
        ..ServerSection::default()
    });
    InitWriteToml {
        project,
        server,
        watch: existing.watch.clone(),
    }
}

/// Resolve the canonical project name for `arags init`.
///
/// Priority: explicit `--name` flag → interactive prompt (TTY only, prefilled
/// with a suggestion) → hard error (non-interactive with no flag, since a name
/// must never be silently derived from the path).
pub(crate) fn resolve_init_name(
    name: Option<String>,
    project: &Path,
    non_interactive: bool,
) -> Result<String> {
    let suggestion = suggest_project_name(project);
    let chosen = match name {
        Some(n) => n,
        None => {
            if !non_interactive && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
            {
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
        print!("A project name is required (not derived from the path): ");
        std::io::stdout()
            .flush()
            .context("failed to flush stdout")?;
    }
}

/// Prompt for a yes/no answer, defaulting to `default`.
fn prompt_bool(prompt: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!("{prompt} [{hint}]: ");
        std::io::stdout()
            .flush()
            .context("failed to flush stdout")?;
        let mut buf = String::new();
        std::io::stdin()
            .read_line(&mut buf)
            .context("failed to read answer from stdin")?;
        let input = buf.trim().to_lowercase();
        if input.is_empty() {
            return Ok(default);
        }
        match input.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                print!("Please answer y or n: ");
                std::io::stdout()
                    .flush()
                    .context("failed to flush stdout")?;
            }
        }
    }
}

/// Prompt for the ignore patterns; `default` is shown as a hint and used on
/// empty input. Input is parsed as comma-separated globs.
fn prompt_ignore(default: &[String]) -> Result<Vec<String>> {
    let hint = default.join(", ");
    print!("Ignore patterns (comma-separated) [{hint}]: ");
    std::io::stdout()
        .flush()
        .context("failed to flush stdout")?;
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .context("failed to read ignore patterns from stdin")?;
    let input = buf.trim();
    if input.is_empty() {
        return Ok(default.to_vec());
    }
    Ok(input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Prompt for the server address; `default` is shown and used on empty input.
fn prompt_server_addr(default: &str) -> Result<Option<String>> {
    print!("Server address [{default}]: ");
    std::io::stdout()
        .flush()
        .context("failed to flush stdout")?;
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .context("failed to read server address from stdin")?;
    let input = buf.trim();
    if input.is_empty() {
        Ok(if default.is_empty() {
            None
        } else {
            Some(default.to_string())
        })
    } else {
        Ok(Some(input.to_string()))
    }
}

/// Best-effort project name *suggestion*: git remote, else directory basename.
/// Used only to prefill the interactive prompt — never applied as the value
/// (issue `agnostic-rag-rlm-tool-f5db`).
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
#[must_use]
pub(crate) fn seed_ignore_from_gitignore(project: &Path) -> Vec<String> {
    let gitignore = project.join(".gitignore");
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

/// Existing `[project] ignore` from a parsed local config.
#[must_use]
fn existing_ignore(existing: &LocalConfig) -> Option<Vec<String>> {
    existing
        .project
        .as_ref()
        .and_then(|p| p.ignore.clone())
        .filter(|v| !v.is_empty())
}

/// Existing `[server] addr` from a parsed local config.
#[must_use]
fn existing_server_addr(existing: &LocalConfig) -> Option<String> {
    existing
        .server
        .as_ref()
        .and_then(|s| s.addr.clone())
        .filter(|a| !a.is_empty())
}

/// Optional server checks run after the config is written (issue
/// `agnostic-rag-rlm-tool-e5d8`):
///
/// 1. **Health-check** — ping `GetServerStatus` and confirm the `refresh_token`
///    authenticated (the session token is attached by the client). A down
///    server only warns.
/// 2. **Identity-conflict hook** — reuse the existing `GetProject`-by-name RPC
///    to detect whether the chosen canonical name already exists on the server
///    with a *distinct* root (a different checkout claiming the same knowledge
///    entity). Mirrors the `agnostic-rag-rlm-tool-f5db` identity heuristic: an exact
///    root match is a benign re-init, a differing root is a real conflict. In
///    non-interactive mode a conflict is a hard failure; interactively it is a
///    warning only.
fn run_server_checks(
    rt: &Runtime,
    cfg: &EffectiveUserConfig,
    project: &Path,
    name: &str,
    non_interactive: bool,
) -> Result<()> {
    let has_token = cfg
        .auth()
        .and_then(|a| a.refresh_token.as_ref())
        .is_some_and(|t| !t.is_empty());
    if !has_token {
        eprintln!(
            "! no global identity (refresh token) configured; skipping server health-check \
              and identity-conflict check. Run `arags-server admin create-refresh`."
        );
        return Ok(());
    }

    let mut client = match connect(rt, cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("! could not connect to server; skipping server checks ({e}).");
            return Ok(());
        }
    };

    match rt.block_on(client.get_server_status(())) {
        Ok(resp) => {
            let s = resp.into_inner();
            eprintln!(
                "✓ server reachable: v{} — {} projects, {} chunks",
                s.version, s.total_projects, s.total_chunks
            );
        }
        Err(e) => {
            eprintln!("! server health-check failed ({e}); continuing.");
        }
    }

    match rt.block_on(client.get_project(name.to_string())) {
        Ok(resp) => {
            let info = resp.into_inner();
            let current = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
            if info.root_path == current.to_string_lossy() {
                eprintln!("✓ canonical name '{name}' already registered for this root (reusing).");
            } else {
                let root = info.root_path;
                let msg = format!(
                    "canonical name '{name}' already exists on the server with a DIFFERENT root \
                      ({root}) — this would merge unrelated content into one knowledge buffer"
                );
                if non_interactive {
                    bail!("{msg}");
                }
                eprintln!("! WARNING: {msg}");
            }
        }
        Err(e) if e.code() == tonic::Code::NotFound => {
            eprintln!("✓ canonical name '{name}' is free on the server.");
        }
        Err(e) => {
            eprintln!("! could not query project on server ({e}); skipping conflict check.");
        }
    }
    Ok(())
}

/// Append `.arags.toml` to `.gitignore` (idempotent).
fn append_gitignore(project: &Path, local_path: &Path) -> Result<()> {
    let gitignore = project.join(".gitignore");
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

//! Loading and merging of user configuration files.
//!
//! Precedence: local `.arags.toml` overrides global `~/.arags/arags.toml`
//! field-by-field (plan 020); `[auth]` is global-only; legacy
//! `config.toml` names are never read.

use anyhow::{Context, Result};
use arags_llm::config::{BackendConfig, LlmConfig};
use std::path::PathBuf;

use super::{EffectiveUserConfig, GlobalConfig, LocalConfig, ProjectSection, ServerSection};

pub fn load() -> Result<EffectiveUserConfig> {
    load_from(&global_path(), &local_path())
}

/// Pure, testable core of [`load`]: merge an explicit global file with an
/// explicit local file (either may not exist).
///
/// # Errors
///
/// Returns an error if either file exists but cannot be parsed.
pub fn load_from(global: &std::path::Path, local: &std::path::Path) -> Result<EffectiveUserConfig> {
    let global = read_toml_file::<GlobalConfig>(global, "global arags.toml")?;
    let local = read_toml_file::<LocalConfig>(local, "local .arags.toml")?;
    Ok(merge(global, local))
}

/// Parse a local `.arags.toml` at an explicit path (missing file = default).
/// Used by the watch daemon, which runs detached with an unknown cwd.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be parsed.
pub fn load_local_at(local: &std::path::Path) -> Result<LocalConfig> {
    read_toml_file::<LocalConfig>(local, "local .arags.toml")
}

/// Merge a parsed global scope with a parsed local scope (plan 020).
#[must_use]
pub fn merge(global: GlobalConfig, local: LocalConfig) -> EffectiveUserConfig {
    // `[auth]` is global-only: the local scope cannot even carry it
    // (`LocalConfig` has no `auth` field), so it always comes from global.
    let auth = global.auth;

    // `[llm]`: merge backends list-wise (local over global per backend) when
    // both scopes define it; otherwise take whichever is present.
    let llm = match (global.llm, local.llm) {
        (Some(g), Some(l)) => Some(LlmConfig {
            backends: merge_backends(&g.backends, &l.backends),
        }),
        (Some(g), None) => Some(g),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    };

    // `[server]`: merge field-by-field (granular; local wins per field).
    let (local_server, global_server) = (local.server, global.server);
    let server = ServerSection {
        addr: local_server
            .as_ref()
            .and_then(|s| s.addr.clone())
            .or_else(|| global_server.as_ref().and_then(|s| s.addr.clone())),
        tls_ca: local_server
            .as_ref()
            .and_then(|s| s.tls_ca.clone())
            .or_else(|| global_server.as_ref().and_then(|s| s.tls_ca.clone())),
        tls_cert: local_server
            .as_ref()
            .and_then(|s| s.tls_cert.clone())
            .or_else(|| global_server.as_ref().and_then(|s| s.tls_cert.clone())),
        tls_key: local_server
            .as_ref()
            .and_then(|s| s.tls_key.clone())
            .or_else(|| global_server.as_ref().and_then(|s| s.tls_key.clone())),
    };

    // `[project]`: merge field-by-field.
    let local_project = local.project;
    let global_project = global.project;
    let project = ProjectSection {
        name: local_project
            .as_ref()
            .and_then(|p| p.name.clone())
            .or_else(|| global_project.as_ref().and_then(|p| p.name.clone())),
        ignore: local_project
            .as_ref()
            .and_then(|p| p.ignore.clone())
            .or_else(|| global_project.as_ref().and_then(|p| p.ignore.clone())),
    };

    // `[watch]` is local-only (registration is per-project).
    let watch = local.watch;

    // `[volunteer]` is global-only (opt-in identity of this machine's user).
    let volunteer = global.volunteer;

    EffectiveUserConfig {
        auth,
        llm,
        server,
        project,
        watch,
        volunteer,
    }
}

/// Persist `[watch]` in the local `.arags.toml`, preserving every other
/// existing field (round-trips through `toml::Value`).
///
/// # Errors
///
/// Fails on read/parse/write of the local config file.
pub fn set_watch_enabled(local: &std::path::Path, enabled: bool, project: &str) -> Result<()> {
    let mut doc: toml::Value = if local.exists() {
        let raw = std::fs::read_to_string(local)
            .with_context(|| format!("failed to read {}", local.display()))?;
        raw.parse()
            .with_context(|| format!("failed to parse {}", local.display()))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let root = doc
        .as_table_mut()
        .context("local .arags.toml is not a TOML table")?;
    let mut watch = toml::map::Map::new();
    watch.insert("enabled".into(), toml::Value::Boolean(enabled));
    if enabled {
        watch.insert("project".into(), toml::Value::String(project.to_string()));
    }
    root.insert("watch".into(), toml::Value::Table(watch));
    let serialized =
        toml::to_string_pretty(&doc).context("failed to serialize local .arags.toml")?;
    std::fs::write(local, serialized)
        .with_context(|| format!("failed to write {}", local.display()))
}

/// Address precedence: configured `server.addr` first (local already won over
/// global in [`merge`]), then the `ARAGS_SERVER_ADDR` env override, then the
/// localhost default. Plan 020 keeps the env var working "as if set".
#[must_use]
pub fn resolve_addr(configured: Option<&str>, env: Option<&str>) -> String {
    const DEFAULT: &str = "127.0.0.1:50051";
    configured
        .or(env)
        .map_or(DEFAULT.to_string(), str::to_string)
}

/// Read + parse a TOML config file; a missing file is an empty default.
fn read_toml_file<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
    label: &str,
) -> Result<T> {
    if !path.exists() {
        // `Default` is only derived for the exact config structs.
        return toml::from_str("").with_context(|| format!("failed to parse empty {label}"));
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {label}"))
}

/// Merge two backend lists: local backends override global backends that share
/// the same logical name, and any purely-local backend is appended.
#[must_use]
pub fn merge_backends(global: &[BackendConfig], local: &[BackendConfig]) -> Vec<BackendConfig> {
    let mut out: Vec<BackendConfig> = Vec::with_capacity(global.len() + local.len());
    for g in global {
        match local.iter().find(|l| same_backend(l, g)) {
            Some(l) => out.push(l.clone()),
            None => out.push(g.clone()),
        }
    }
    for l in local {
        if !out.iter().any(|b| same_backend(b, l)) {
            out.push(l.clone());
        }
    }
    out
}

/// Two backends are "the same" when they share a name, a model, or a family.
fn same_backend(a: &BackendConfig, b: &BackendConfig) -> bool {
    if let (Some(an), Some(bn)) = (&a.name, &b.name) {
        return an == bn;
    }
    if let (Some(am), Some(bm)) = (&a.model, &b.model) {
        return am == bm;
    }
    a.family == b.family
}

fn global_path() -> PathBuf {
    home_dir().join(".arags").join("arags.toml")
}

fn local_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".arags.toml")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

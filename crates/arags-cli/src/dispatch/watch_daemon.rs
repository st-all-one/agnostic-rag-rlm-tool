//! Hidden `arags __watch <root>` daemon: watch a project root and re-stream
//! only the changed files after each quiet window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tokio::runtime::Runtime;

use crate::auth_client::AragsClient;
use crate::user_config::EffectiveUserConfig;

use super::discover::discover_files;
use super::index::{partition_files, stream_index_group};

/// Upload parallelism inside the daemon (small on purpose: background work
/// must not saturate the user's machine or the server).
const WATCH_UPLOAD_PARALLELISM: usize = 2;

/// Entry point of the hidden `arags __watch <root>` daemon: watch `root`,
/// and after each 1-minute quiet window re-stream only the changed files.
pub(crate) fn run_watch_daemon(rt: &Runtime, cfg: &EffectiveUserConfig, root: &Path) -> Result<()> {
    let local = crate::user_config::load_local_at(&root.join(".arags.toml")).unwrap_or_default();
    let project_name = local
        .watch
        .and_then(|w| w.project)
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    let mut ignore = cfg.ignore_patterns();
    if let Some(local_ignore) = local.project.and_then(|p| p.ignore) {
        ignore.extend(local_ignore);
    }
    let force_include: Vec<String> = Vec::new();
    let mut client = super::connect(rt, cfg)?;
    let mut known = snapshot_state(root, &ignore, &force_include);

    tracing::info!(
        root = %root.display(),
        %project_name,
        "watch daemon started"
    );

    let rt_ref = &rt;
    let client_ref = &mut client;
    let known_ref = &mut known;
    crate::watcher::watch_loop(root, &mut |changed: &[PathBuf]| {
        flush_changed(
            rt_ref,
            client_ref,
            root,
            &project_name,
            &ignore,
            &force_include,
            known_ref,
            changed,
        )
    })
}

/// mtime+size fingerprint used to decide whether a file really changed.
type FileState = (u128, u64);

#[allow(clippy::too_many_arguments)]
fn flush_changed(
    rt: &Runtime,
    client: &mut AragsClient,
    root: &Path,
    project_name: &str,
    ignore: &[String],
    force_include: &[String],
    known: &mut HashMap<String, FileState>,
    changed: &[PathBuf],
) -> Result<()> {
    use std::collections::HashSet;
    if changed.is_empty() {
        return Ok(());
    }

    // Re-discover so ignore rules apply to new files as well.
    let current = discover_files(root, ignore, force_include)
        .map_err(|e| anyhow::anyhow!("file discovery failed: {e}"))?;
    let mut current_set: HashSet<String> = HashSet::with_capacity(current.len());
    for path in &current {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        current_set.insert(rel);
    }

    // Changed ∧ still present ∧ still includable; skip unchanged fingerprints.
    let changed_set: HashSet<String> = changed
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let mut to_send: Vec<PathBuf> = Vec::new();
    for path in &current {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        if !changed_set.contains(rel.as_str()) {
            continue;
        }
        let state = file_state(path);
        if known.get(&rel) == Some(&state) {
            continue; // e.g. editor touch without content change
        }
        to_send.push(path.clone());
    }
    if to_send.is_empty() {
        tracing::debug!(count = changed.len(), "no surviving changes to index");
        return Ok(());
    }

    let groups = partition_files(&to_send, WATCH_UPLOAD_PARALLELISM);
    for group in groups {
        let pb = Arc::new(indicatif::ProgressBar::hidden());
        let (files_idx, chunks_idx) = rt.block_on(stream_index_group(
            client,
            project_name.to_string(),
            root.to_path_buf(),
            group,
            pb,
        ))?;
        tracing::info!(files = files_idx, chunks = chunks_idx, "re-indexed changes");
    }

    // Refresh fingerprints for everything we just sent.
    for path in &to_send {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        known.insert(rel, file_state(path));
    }
    Ok(())
}

/// Current `(mtime_nanos, size)` fingerprint of a file (zeroed on failure).
fn file_state(path: &Path) -> FileState {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let nanos = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| {
                    u128::from(d.as_secs()) * 1_000_000_000 + u128::from(d.subsec_nanos())
                });
            let size = meta.len();
            (nanos, size)
        }
        Err(_) => (0, 0),
    }
}

/// Discovery + fingerprints for the whole tree (daemon startup baseline).
fn snapshot_state(
    root: &Path,
    ignore: &[String],
    force_include: &[String],
) -> HashMap<String, FileState> {
    let mut map = HashMap::new();
    let Ok(files) = discover_files(root, ignore, force_include) else {
        return map;
    };
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        map.insert(rel, file_state(&path));
    }
    map
}

#[cfg(test)]
mod tests;

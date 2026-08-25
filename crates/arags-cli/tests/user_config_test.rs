//! Tests for the 2-scope user config (plan 020): granular local→global merge,
//! global-only `[auth]`, legacy-file rejection and watch-flag round-trips.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arags_cli::user_config::{load_from, load_local_at, resolve_addr, set_watch_enabled};
use tempfile::TempDir;

fn write(path: &std::path::Path, content: &str) {
    std::fs::write(path, content).expect("test write");
}

const GLOBAL: &str = r#"
[auth]
username = "dev1"
refresh_token = "tok-123"

[llm]
[[llm.backends]]
name = "default"
family = "openai"
model = "gpt-4o-mini"
api_key = "sk-x"

[server]
addr = "https://arags.corp.internal:50051"

[project]
name = "global-name"
ignore = ["target/"]
"#;

#[test]
fn test_user_config_merge_local_overrides_global_granular() {
    let dir = TempDir::new().unwrap();
    let g = dir.path().join("global.toml");
    let l = dir.path().join("local.toml");
    write(&g, GLOBAL);
    // Only `addr` is overridden; everything else falls back to global.
    write(
        &l,
        "[server]\naddr = \"http://localhost:50051\"\n\n[project]\nignore = [\"dist/\"]\n",
    );

    let cfg = load_from(&g, &l).unwrap();
    assert_eq!(cfg.server_addr(), "http://localhost:50051");
    // Absent locally → falls back to the global value, granularly.
    assert_eq!(cfg.project.name.as_deref(), Some("global-name"));
    assert_eq!(cfg.ignore_patterns(), vec!["dist/".to_string()]);
}

#[test]
fn test_user_config_nested_merge_recursive() {
    let dir = TempDir::new().unwrap();
    let g = dir.path().join("global.toml");
    let l = dir.path().join("local.toml");
    write(
        &g,
        "[llm]\n[[llm.backends]]\nname = \"default\"\nfamily = \"openai\"\nmodel = \"gpt-4o-mini\"\n\n[[llm.backends]]\nname = \"spare\"\nfamily = \"ollama\"\nbase_url = \"http://localhost:11434\"\n",
    );
    // Local redefines only the `default` backend (by name); `spare` from
    // global survives the merge.
    write(
        &l,
        "[llm]\n[[llm.backends]]\nname = \"default\"\nfamily = \"ollama\"\nmodel = \"qwen2.5-coder:7b\"\n",
    );

    let cfg = load_from(&g, &l).unwrap();
    let llm = cfg.llm_config().unwrap();
    assert_eq!(llm.backends.len(), 2);
    assert_eq!(llm.backends[0].family.to_string().to_lowercase(), "ollama");
    assert_eq!(llm.backends[0].model.as_deref(), Some("qwen2.5-coder:7b"));
    assert_eq!(llm.backends[1].name.as_deref(), Some("spare"));
}

#[test]
fn test_auth_only_global() {
    let dir = TempDir::new().unwrap();
    let g = dir.path().join("global.toml");
    let l = dir.path().join("local.toml");
    write(&g, GLOBAL);
    // A local `[auth]` must be ignored entirely (no credentials in repo).
    write(
        &l,
        "[auth]\nusername = \"evil\"\nrefresh_token = \"stolen\"\n",
    );

    let cfg = load_from(&g, &l).unwrap();
    let auth = cfg.auth().unwrap();
    assert_eq!(auth.username.as_deref(), Some("dev1"));
    assert_eq!(auth.refresh_token.as_deref(), Some("tok-123"));
}

#[test]
fn test_legacy_config_toml_ignored() {
    let dir = TempDir::new().unwrap();
    // Legacy-named files are present but MUST NOT be read (plan 020 D4):
    // `load_from` is only ever pointed at arags.toml / .arags.toml.
    let legacy = dir.path().join("config.toml");
    write(
        &legacy,
        "[auth]\nusername = \"old\"\nrefresh_token = \"legacy\"\n\n[server]\naddr = \"legacy:1\"\n",
    );
    // Pointing at the *new* names (which do not exist) yields defaults —
    // the legacy file content never leaks into the effective config.
    let cfg = load_from(
        &dir.path().join("arags.toml"),
        &dir.path().join(".arags.toml"),
    )
    .unwrap();
    assert!(cfg.server.addr.is_none());
    assert!(cfg.auth.is_none());
}

#[test]
fn test_client_uses_merged_server_addr_and_env_override() {
    // Pure precedence: config > env > default.
    assert_eq!(resolve_addr(Some("cfg:1"), None), "cfg:1");
    assert_eq!(resolve_addr(None, Some("env:2")), "env:2");
    assert_eq!(resolve_addr(Some("cfg:1"), Some("env:2")), "cfg:1");
    assert_eq!(resolve_addr(None, None), "127.0.0.1:50051");
}

#[test]
fn test_missing_files_default() {
    let dir = TempDir::new().unwrap();
    let cfg = load_from(&dir.path().join("none.toml"), &dir.path().join("none.toml")).unwrap();
    assert!(cfg.auth.is_none());
    assert!(cfg.llm.is_none());
    assert_eq!(cfg.project.name, None);
}

#[test]
fn test_server_tls_fields_merge_granularly() {
    let dir = TempDir::new().unwrap();
    let g = dir.path().join("global.toml");
    let l = dir.path().join("local.toml");
    write(
        &g,
        "[server]\naddr = \"https://a:1\"\ntls_ca = \"/etc/arags/ca.crt\"\ntls_cert = \"/etc/arags/client.crt\"\ntls_key = \"/etc/arags/client.key\"\n",
    );
    // Local overrides only `addr`; TLS knobs fall back to global.
    write(&l, "[server]\naddr = \"http://localhost:50051\"\n");

    let cfg = load_from(&g, &l).unwrap();
    assert_eq!(cfg.server_addr(), "http://localhost:50051");
    assert_eq!(cfg.server.tls_ca.as_deref(), Some("/etc/arags/ca.crt"));
    assert_eq!(
        cfg.server.tls_cert.as_deref(),
        Some("/etc/arags/client.crt")
    );
    assert_eq!(cfg.server.tls_key.as_deref(), Some("/etc/arags/client.key"));
}

#[test]
fn test_malformed_local_file_is_error() {
    let dir = TempDir::new().unwrap();
    let g = dir.path().join("global.toml");
    let l = dir.path().join("local.toml");
    write(&g, GLOBAL);
    write(&l, "not [ valid toml ===");
    assert!(load_from(&g, &l).is_err());
}

/// Plan 020: `server.toml` (data plane) and the user config are disjoint
/// files. A server-shaped file parsed as **user** config must not leak any
/// of its data-plane values into the effective user config.
#[test]
fn test_user_config_ignores_server_toml_semantics() {
    let dir = TempDir::new().unwrap();
    let server_toml = r#"
listen_addr = "0.0.0.0:50051"
data_dir = "/var/lib/arags"
pool_size = 4
flush_interval_ms = 100
max_batch_size = 50

[embedder]
max_tokens = 512
overlap_tokens = 64

[search]
tier = "hybrid"

[history]
retention_days = 90
"#;
    let path = dir.path().join("server.toml");
    std::fs::write(&path, server_toml).unwrap();

    let cfg = load_from(&path, &dir.path().join(".arags.toml")).unwrap();
    assert!(cfg.auth.is_none());
    assert!(cfg.llm.is_none());
    assert!(
        cfg.server.addr.is_none(),
        "listen_addr must NOT become [server].addr"
    );
    assert_eq!(cfg.server.tls_ca, None);
    assert_eq!(cfg.project.name, None);
    assert_eq!(cfg.server_addr(), "127.0.0.1:50051");
}

#[test]
fn test_set_watch_enabled_roundtrip_preserves_fields() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".arags.toml");
    std::fs::write(
        &path,
        "[project]\nname = \"demo\"\nignore = [\"target/\"]\n\n[server]\naddr = \"10.0.0.5:50051\"\n",
    )
    .unwrap();

    set_watch_enabled(&path, true, "demo").unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("[project]"));
    assert!(raw.contains("enabled = true"));

    let local = load_local_at(&path).unwrap();
    assert!(local.watch.as_ref().is_some_and(|w| w.enabled));
    assert_eq!(local.watch.and_then(|w| w.project).as_deref(), Some("demo"));
    assert_eq!(
        local.project.as_ref().and_then(|p| p.name.as_deref()),
        Some("demo")
    );

    // Unregister clears the flag but keeps the section and other data.
    set_watch_enabled(&path, false, "").unwrap();
    let local = load_local_at(&path).unwrap();
    assert!(!local.watch.unwrap().enabled);
}

#[test]
fn test_set_watch_enabled_creates_missing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".arags.toml");
    set_watch_enabled(&path, true, "p").unwrap();
    let local = load_local_at(&path).unwrap();
    assert!(local.watch.is_some_and(|w| w.enabled));
}

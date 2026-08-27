use super::*;
use std::path::PathBuf;

mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::io::Write as _;

    fn temp_config(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        (dir, path)
    }

    #[test]
    fn test_server_config_loads_from_arags_server_config_env() {
        // `load_from_path` is the env-free core of `load()`; the default
        // path comes from `ARAGS_SERVER_CONFIG` (else /etc/arags/server.toml).
        let (_d, path) = temp_config("listen_addr = \"0.0.0.0:9999\"\n");
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:9999");

        // Missing file → built-in defaults.
        let d = tempfile::tempdir().unwrap();
        let cfg = ServerConfig::load_from_path(&d.path().join("absent.toml")).unwrap();
        assert_eq!(cfg.listen_addr, default_listen_addr());
        assert_eq!(cfg.embedder.batch_size, default_batch_size());
        assert!(cfg.embedder.model_dir.is_none());
    }

    #[test]
    fn test_server_config_overrides_apply() {
        let d = tempfile::tempdir().unwrap();
        let cfg = ServerConfig::load_from_path(&d.path().join("absent.toml")).unwrap();
        let cfg = cfg.with_overrides(
            Some("0.0.0.0:9998".to_owned()),
            Some("/tmp/arags-override".to_owned()),
            Some("/models".to_owned()),
            None,
            None,
            None,
            None,
        );

        assert_eq!(cfg.listen_addr, "0.0.0.0:9998");
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/arags-override"));
        assert_eq!(
            cfg.embedder.model_dir.as_deref(),
            Some(std::path::Path::new("/models"))
        );
    }

    #[test]
    fn test_server_config_has_no_llm_section() {
        // A `server.toml` without `[llm]` parses fine; a stray `[llm]`
        // section must NOT silently map onto any field of the schema.
        let (_d, path) =
            temp_config("listen_addr = \"127.0.0.1:50051\"\ndata_dir = \"/tmp/arags\"\n");
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/arags"));
    }

    #[test]
    fn test_server_config_embedder_chunk_size_applied() {
        let (_d, path) = temp_config(
            "[embedder]\nmax_tokens = 1024\noverlap_tokens = 128\nbatch_size = 8\nquantization = \"none\"\ncache = false\n",
        );
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.embedder.max_tokens, 1024);
        assert_eq!(cfg.embedder.overlap_tokens, 128);
        assert_eq!(cfg.embedder.batch_size, 8);
        assert_eq!(
            cfg.embedder.resolved_quantization(),
            arags_embedding::embedder::config::Quantization::None
        );
        assert!(!cfg.embedder.cache);
    }

    #[test]
    fn test_server_config_search_and_mtls_defaults() {
        let defaults = ServerConfig::default();
        assert_eq!(defaults.search.top_k, 10);
        assert_eq!(defaults.search.max_tokens, 8000);
        assert_eq!(defaults.search.tier, "hybrid");
        assert!(defaults.mtls_ca().is_none());

        let (_d, path) = temp_config(
            "mtls_ca = \"/etc/arags/tls/ca.crt\"\n\n[search]\ntop_k = 42\nmax_tokens = 100\n",
        );
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.search.top_k, 42);
        assert_eq!(cfg.mtls_ca(), Some(&PathBuf::from("/etc/arags/tls/ca.crt")));
    }

    #[test]
    fn quorum_config_defaults_when_section_absent() {
        // A `server.toml` without `[quorum]` must parse with built-in defaults
        // (issue `agnostic-rlm-rs-a5d7`); existing configs keep working.
        let (_d, path) = temp_config("listen_addr = \"127.0.0.1:50051\"\n");
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.quorum.n, 3);
        assert_eq!(cfg.quorum.quorum_sim_threshold, 0.85);
        assert_eq!(cfg.quorum.fusion_strategy, FusionStrategy::Consensus);
        assert_eq!(cfg.quorum.strikes_limit, 3);

        // Explicit `[quorum]` overrides individual fields.
        let (_d, path) = temp_config(
            "[quorum]\nn = 5\nquorum_sim_threshold = 0.9\nfusion_strategy = \"longest\"\nstrikes_limit = 2\n",
        );
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.quorum.n, 5);
        assert_eq!(cfg.quorum.quorum_sim_threshold, 0.9);
        assert_eq!(cfg.quorum.fusion_strategy, FusionStrategy::Longest);
        assert_eq!(cfg.quorum.strikes_limit, 2);
    }

    #[test]
    fn exploration_validation_mode_defaults_to_quorum() {
        // A `[exploration]` section without `validation_mode` must parse as
        // `Quorum` (issue `agnostic-rlm-rs-e89e`), and `require_review` stays
        // its own default (`false`).
        let (_d, path) = temp_config("[exploration]\nenabled = true\n");
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.exploration.validation_mode, ValidationMode::Quorum);
        assert!(!cfg.exploration.require_review);

        // Explicit `review` mode parses and keeps `require_review`.
        let (_d, path) =
            temp_config("[exploration]\nvalidation_mode = \"review\"\nrequire_review = true\n");
        let cfg = ServerConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.exploration.validation_mode, ValidationMode::Review);
        assert!(cfg.exploration.require_review);
    }

    #[test]
    fn test_server_config_index_embed_threads_reserves_cores() {
        // Default must leave at least 1 core for serving, and reserve cores
        // when the host has >= 3 (issue `agnostic-rlm-rs-6690`).
        let cfg = ServerConfig::default();
        assert!(cfg.index_embed_threads >= 1, "must leave at least 1 core");
        let total = num_cpus::get();
        if total >= 3 {
            assert!(
                cfg.index_embed_threads < total,
                "must reserve cores for query serving when >=3 cores"
            );
        }

        // Env-style override via the testable `with_overrides` core.
        let cfg = ServerConfig::default().with_overrides(
            None,
            None,
            None,
            None,
            None,
            None,
            Some("2".to_owned()),
        );
        assert_eq!(cfg.index_embed_threads, 2);

        // Invalid / zero override is ignored (falls back to previous value).
        let cfg = ServerConfig::default().with_overrides(
            None,
            None,
            None,
            None,
            None,
            None,
            Some("0".to_owned()),
        );
        assert!(cfg.index_embed_threads >= 1);
    }
}

mod disjoint_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use tempfile::TempDir;

    /// Plan 020: the server must NOT read the user's `~/.arags/arags.toml` /
    /// `.arags.toml`. Parsing a user-config-shaped file as `ServerConfig`
    /// leaves every data-plane field at its default.
    #[test]
    fn test_server_config_ignores_user_arags_toml_semantics() {
        let dir = TempDir::new().unwrap();
        let user_toml = r#"
[auth]
username = "dev1"
refresh_token = "tok"

[llm]
[[llm.backends]]
name = "default"
family = "ollama"
model = "llama3.2"

[server]
addr = "https://arags.corp.internal:50051"

[project]
name = "meu-repo"
"#;
        let path = dir.path().join("arags.toml");
        std::fs::write(&path, user_toml).unwrap();

        let cfg = ServerConfig::load_from_path(&path).unwrap();
        // `[server].addr` (client connect target) must NOT become listen_addr.
        assert_eq!(cfg.listen_addr, default_listen_addr());
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.embedder.max_tokens, default_max_tokens());
        assert!(cfg.mtls_ca.is_none());
    }
}

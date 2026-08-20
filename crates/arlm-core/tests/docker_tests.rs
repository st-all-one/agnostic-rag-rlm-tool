#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use arlm_core::docker::{DockerComposeConfig, DockerConfig, DockerExecutor};
use tempfile::TempDir;

#[test]
fn test_docker_config_default() {
    let config = DockerConfig::default();
    assert_eq!(config.image, "python:3.12-slim");
    assert!(!config.network_enabled);
}

#[test]
fn test_docker_executor_creation() {
    let tmp = TempDir::new().unwrap();
    let executor = DockerExecutor::default_executor(tmp.path().to_path_buf());
    assert_eq!(executor.work_dir(), tmp.path());
}

#[test]
fn test_docker_compose_fragment() {
    let config = DockerComposeConfig {
        service_name: "arlm-server".to_string(),
        image: "arlm:latest".to_string(),
        ports: vec!["50051:50051".to_string()],
        environment: vec!["RUST_LOG=info".to_string()],
        volumes: vec!["./data:/data".to_string()],
        command: None,
        health_check: Some("grpc_health_probe -addr=:50051".to_string()),
    };

    let fragment = config.to_compose_fragment();
    assert!(fragment.contains("arlm-server:"));
    assert!(fragment.contains("image: arlm:latest"));
    assert!(fragment.contains("50051:50051"));
    assert!(fragment.contains("healthcheck:"));
}

#[test]
fn test_is_available() {
    let _ = DockerExecutor::is_available();
}

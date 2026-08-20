//! Docker execution environment for sandboxed code execution.
//!
//! Provides a Docker-based executor that runs code in isolated containers
//! for security when executing untrusted model-generated code.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Configuration for the Docker executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Docker image to use for execution.
    pub image: String,
    /// Maximum execution time per command.
    pub timeout: Duration,
    /// Memory limit (e.g., "512m").
    pub memory_limit: String,
    /// CPU limit (e.g., "1.0").
    pub cpu_limit: String,
    /// Whether to enable network access.
    pub network_enabled: bool,
    /// Additional volumes to mount (host:container).
    pub volumes: Vec<String>,
    /// Working directory inside the container.
    pub work_dir: String,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            image: "python:3.12-slim".to_string(),
            timeout: Duration::from_secs(60),
            memory_limit: "512m".to_string(),
            cpu_limit: "1.0".to_string(),
            network_enabled: false,
            volumes: Vec::new(),
            work_dir: "/workspace".to_string(),
        }
    }
}

/// Result of a Docker execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub container_id: Option<String>,
}

/// Docker-based code executor for sandboxed execution.
pub struct DockerExecutor {
    config: DockerConfig,
    work_dir: PathBuf,
}

impl DockerExecutor {
    /// Create a new Docker executor.
    pub fn new(config: DockerConfig, work_dir: PathBuf) -> Self {
        Self { config, work_dir }
    }

    /// Create a Docker executor with default configuration.
    pub fn default_executor(work_dir: PathBuf) -> Self {
        Self::new(DockerConfig::default(), work_dir)
    }

    /// Check if Docker is available.
    pub fn is_available() -> bool {
        Command::new("docker")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Execute a Python script in a Docker container.
    pub fn execute_python(&self, code: &str) -> Result<DockerResult> {
        let script_path = self.work_dir.join("exec.py");
        std::fs::write(&script_path, code).context("failed to write script")?;

        self.run_container(
            &self.config.image,
            &["python", "/workspace/exec.py"],
            Some(code),
        )
    }

    /// Execute a bash command in a Docker container.
    pub fn execute_bash(&self, command: &str) -> Result<DockerResult> {
        self.run_container(
            &self.config.image,
            &["bash", "-c", command],
            None,
        )
    }

    /// Run a Docker container with the given command.
    fn run_container(
        &self,
        image: &str,
        cmd: &[&str],
        _stdin: Option<&str>,
    ) -> Result<DockerResult> {
        let start = Instant::now();

        // Build docker run command
        let mut args = vec!["run", "--rm"];

        // Memory limit
        args.push("--memory");
        args.push(&self.config.memory_limit);

        // CPU limit
        args.push("--cpus");
        args.push(&self.config.cpu_limit);

        // Network
        if !self.config.network_enabled {
            args.push("--network=none");
        }

        // Volumes
        let volume_mount = format!("{}:/workspace:ro", self.work_dir.display());
        args.push("--volume");
        args.push(&volume_mount);

        // Work directory
        args.push("--workdir");
        args.push(&self.config.work_dir);

        // Read-only root filesystem
        args.push("--read-only");

        // Tmpfs for /tmp
        args.push("--tmpfs=/tmp:size=100m");

        // Image
        args.push(image);

        // Command
        args.extend_from_slice(cmd);

        tracing::debug!(image, cmd = ?cmd, "running docker container");

        let output = Command::new("docker")
            .args(&args)
            .output()
            .context("failed to run docker")?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(DockerResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            duration_ms,
            container_id: None,
        })
    }

    /// Execute a code block with language detection.
    pub fn execute(&self, language: &str, code: &str) -> Result<DockerResult> {
        match language {
            "python" | "py" => self.execute_python(code),
            "bash" | "sh" => self.execute_bash(code),
            _ => self.execute_bash(code),
        }
    }

    /// Get the work directory.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// Get the configuration.
    pub fn config(&self) -> &DockerConfig {
        &self.config
    }
}

/// Docker Compose service configuration for multi-container setups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerComposeConfig {
    /// Service name.
    pub service_name: String,
    /// Docker image.
    pub image: String,
    /// Port mappings.
    pub ports: Vec<String>,
    /// Environment variables.
    pub environment: Vec<String>,
    /// Volume mounts.
    pub volumes: Vec<String>,
    /// Command to run.
    pub command: Option<String>,
    /// Health check command.
    pub health_check: Option<String>,
}

impl DockerComposeConfig {
    /// Generate a docker-compose.yml fragment for this service.
    pub fn to_compose_fragment(&self) -> String {
        let mut fragment = format!("  {}:\n", self.service_name);
        fragment.push_str(&format!("    image: {}\n", self.image));

        if !self.ports.is_empty() {
            fragment.push_str("    ports:\n");
            for port in &self.ports {
                fragment.push_str(&format!("      - \"{port}\"\n"));
            }
        }

        if !self.environment.is_empty() {
            fragment.push_str("    environment:\n");
            for env in &self.environment {
                fragment.push_str(&format!("      - \"{env}\"\n"));
            }
        }

        if !self.volumes.is_empty() {
            fragment.push_str("    volumes:\n");
            for vol in &self.volumes {
                fragment.push_str(&format!("      - \"{vol}\"\n"));
            }
        }

        if let Some(ref cmd) = self.command {
            fragment.push_str(&format!("    command: \"{cmd}\"\n"));
        }

        if let Some(ref health) = self.health_check {
            fragment.push_str("    healthcheck:\n");
            fragment.push_str(&format!("      test: [\"CMD\", \"{}\"]\n", health));
            fragment.push_str("      interval: 30s\n");
            fragment.push_str("      timeout: 10s\n");
            fragment.push_str("      retries: 3\n");
        }

        fragment
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // This test will fail if Docker is not installed
        // but should not panic
        let _ = DockerExecutor::is_available();
    }
}

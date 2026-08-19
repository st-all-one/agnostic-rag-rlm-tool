use std::path::Path;

use anyhow::{Context, Result};

use crate::output;
use crate::util::project_dirs;

pub struct ServeConfig<'a> {
    pub port: u16,
    pub host: &'a str,
    pub project: &'a Path,
    pub verbose: bool,
}

pub async fn execute(config: ServeConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_serve");

    let project_name = config
        .project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let data_dir = project_dirs().join(project_name);
    let _storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    output::info(&format!(
        "Starting arlm server on {}:{}",
        config.host, config.port
    ));
    output::info(&format!("Project: {project_name}"));

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", config.host, config.port))
        .await
        .context("failed to bind TCP listener")?;

    output::success(&format!(
        "Server listening on http://{}:{}",
        config.host, config.port
    ));
    println!("\nEndpoints:");
    println!("  POST /context  — Build context for a task");
    println!("  POST /search   — Search the project");
    println!("  GET  /status   — Project status");
    println!("  GET  /history  — Query history");
    println!("\nPress Ctrl+C to stop.\n");

    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                if config.verbose {
                    output::error(&format!("Accept error: {e}"));
                }
                continue;
            }
        };

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream).await {
                tracing::debug!("Connection error: {e}");
            }
        });
    }
}

async fn handle_connection(mut stream: tokio::net::TcpStream) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.context("failed to read")?;

    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");

    let (status, body) = if first_line.starts_with("GET /status") {
        ("200 OK", r#"{"status":"ok","message":"arlm server running"}"#)
    } else if first_line.starts_with("POST /context") || first_line.starts_with("POST /search") {
        ("200 OK", r#"{"status":"ok","message":"endpoint not yet implemented"}"#)
    } else if first_line.starts_with("GET /history") {
        ("200 OK", r#"{"status":"ok","history":[]}"#)
    } else {
        ("404 Not Found", r#"{"status":"error","message":"not found"}"#)
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    );

    stream
        .write_all(response.as_bytes())
        .await
        .context("failed to write")?;
    stream.flush().await.context("failed to flush")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    #[test]
    fn test_serve_storage_opens() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("test-proj");
        std::fs::create_dir_all(&project).unwrap();
        let result = arlm_storage::Storage::open(&crate::util::project_dirs().join("test-proj"));
        assert!(result.is_ok());
    }
}

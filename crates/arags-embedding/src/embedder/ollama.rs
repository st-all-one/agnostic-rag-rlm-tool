use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use serde_json::Value;
use tracing::{debug, error};

use super::{Embedder, Embedding, EmbeddingError, EmbeddingResult};

/// Ollama-backed embedding via the local `/api/embed` HTTP endpoint.
///
/// Used as the production alternative to the in-process `candle` embedder
/// when a faster engine is desired (e.g. `all-minilm:22m`, which the Ollama
/// runtime serves ~3x faster than `candle` on CPU while keeping the same
/// 384-dimensional semantic space). Requires a running Ollama daemon on the
/// configured host/port; failures degrade to the server's hash embedder.
pub struct OllamaEmbedder {
    host: String,
    port: u16,
    model: String,
    dims: usize,
}

impl OllamaEmbedder {
    /// Connect to Ollama, verify the model exists by probing its dimensions
    /// with a single dummy embedding.
    ///
    /// # Errors
    ///
    /// Returns an error if the daemon is unreachable, the model is absent, or
    /// the probe response is malformed.
    pub fn new(base_url: &str, model: &str) -> anyhow::Result<Self> {
        let (host, port) = parse_base(base_url)?;
        let mut emb = Self {
            host,
            port,
            model: model.to_string(),
            dims: 0,
        };
        emb.dims = emb.probe_dims()?;
        Ok(emb)
    }

    fn post_embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Embedding>> {
        let start = Instant::now();
        let result = (|| -> anyhow::Result<Vec<Embedding>> {
            let body = serde_json::json!({ "model": self.model, "input": texts }).to_string();
            let resp = http_post(&self.host, self.port, "/api/embed", &body)?;
            let value: Value =
                serde_json::from_str(&resp).map_err(|e| anyhow!("ollama json: {e}"))?;
            let arr = value
                .get("embeddings")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("ollama response missing `embeddings`"))?;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                #[allow(clippy::cast_possible_truncation)]
                let vec = item
                    .as_array()
                    .ok_or_else(|| anyhow!("ollama embedding not an array"))?
                    .iter()
                    .map(|x| x.as_f64().map_or(0.0_f32, |f| f as f32))
                    .collect::<Vec<f32>>();
                out.push(vec);
            }
            Ok(out)
        })();
        match result {
            Ok(embeddings) => {
                debug!(
                    batch_size = texts.len(),
                    duration_ms = %start.elapsed().as_millis(),
                    dims = embeddings.first().map_or(0, Vec::len),
                    "ollama embedded batch"
                );
                Ok(embeddings)
            }
            Err(e) => {
                error!(error = %e, batch_size = texts.len(), "ollama post_embed failed");
                Err(e)
            }
        }
    }

    fn probe_dims(&self) -> anyhow::Result<usize> {
        let embeddings = self.post_embed(&["dimension-probe"])?;
        embeddings
            .first()
            .map(Vec::len)
            .ok_or_else(|| anyhow!("ollama probe returned no embeddings"))
    }
}

impl Embedder for OllamaEmbedder {
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        self.embed_batch(&[text]).and_then(|mut v| {
            v.pop()
                .ok_or_else(|| EmbeddingError::ModelNotLoaded("ollama empty".into()))
        })
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        self.post_embed(texts)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn name(&self) -> &'static str {
        "ollama"
    }
}

/// Split `http(s)://host:port` into `(host, port)`, defaulting the port to
/// Ollama's standard `11434` when omitted.
fn parse_base(url: &str) -> anyhow::Result<(String, u16)> {
    let s = url
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    match s.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port
                .parse()
                .map_err(|_| anyhow!("invalid ollama port: {port}"))?;
            Ok((host.to_string(), port))
        }
        None => Ok((s.to_string(), 11_434)),
    }
}

/// Minimal blocking HTTP POST (no TLS) over `TcpStream` for localhost Ollama.
///
/// Ollama answers with `Transfer-Encoding: chunked` for many payloads and with
/// `Content-Length` for others, so the body is decoded from either framing. A
/// naive `read_to_string`-until-EOF previously blocked on the socket read
/// timeout while the full body had already arrived — which presented as a
/// startup "deadlock" during vector-space bootstrap (issue `agnostic-rag-rlm-tool-3a68`).
///
/// The connect phase is bounded by a connect timeout (issue `agnostic-rag-rlm-tool-9288`):
/// a black-holed/unreachable Ollama now fails fast instead of stalling the
/// bootstrap embed loop indefinitely (the read timeout only covers the response).
fn http_post(host: &str, port: u16, path: &str, body: &str) -> anyhow::Result<String> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow!("ollama resolve {host}:{port}: {e}"))?
        .next()
        .ok_or_else(|| anyhow!("ollama {host}:{port} has no address"))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
        .map_err(|e| anyhow!("ollama connect {host}:{port}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        len = body.len()
    );
    let mut reader = BufReader::new(stream);
    reader.get_mut().write_all(request.as_bytes())?;

    // Read the response headers line-by-line until the blank separator.
    let mut header = String::with_capacity(256);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" {
            break;
        }
        header.push_str(&line);
    }

    let is_chunked = header.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("transfer-encoding:")
            && l.to_ascii_lowercase().contains("chunked")
    });

    let body_bytes = if is_chunked {
        read_chunked(&mut reader)?
    } else {
        let content_length = response_content_length(&header);
        if content_length > 0 {
            let mut buf = vec![0u8; content_length];
            reader.read_exact(&mut buf)?;
            buf
        } else {
            // No framing header: read until EOF (Connection: close).
            let mut buf = Vec::new();
            reader.read_to_end(&mut buf)?;
            buf
        }
    };
    Ok(String::from_utf8_lossy(&body_bytes).into_owned())
}

/// Decode an HTTP `Transfer-Encoding: chunked` body from `reader`.
///
/// Each chunk is `<hex-size>\r\n<data>\r\n`, terminated by `0\r\n\r\n`.
fn read_chunked<R: Read>(reader: &mut BufReader<R>) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        let hex = size_line.trim().split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(hex, 16)
            .map_err(|e| anyhow!("ollama chunk size parse `{hex}`: {e}"))?;
        if size == 0 {
            // Consume the trailing CRLF that terminates the final chunk.
            let mut term = [0u8; 2];
            reader.read_exact(&mut term)?;
            break;
        }
        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk)?;
        out.extend_from_slice(&chunk);
        // Each chunk body is followed by CRLF.
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
    Ok(out)
}

/// Extract `Content-Length` from an HTTP response header block (case
/// insensitive). Returns `0` when the header is absent or unparseable, in
/// which case the caller falls back to reading the body until EOF.
fn response_content_length(header: &str) -> usize {
    header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_base_default_port() {
        assert_eq!(
            parse_base("http://localhost:11434").unwrap(),
            ("localhost".into(), 11_434)
        );
        assert_eq!(
            parse_base("localhost").unwrap(),
            ("localhost".into(), 11_434)
        );
        assert_eq!(
            parse_base("http://ollama:9988/").unwrap(),
            ("ollama".into(), 9988)
        );
    }

    #[test]
    fn test_response_content_length_parses_header() {
        // Honoring `Content-Length` (not read-to-EOF) is what keeps large
        // Ollama batches from stalling on a kept-alive connection.
        assert_eq!(
            response_content_length("HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n"),
            13
        );
        assert_eq!(response_content_length("content-length: 0\r\n\r\n"), 0);
        assert_eq!(
            response_content_length("HTTP/1.1 200 OK\r\nX-Foo: bar\r\n\r\n"),
            0
        );
    }

    #[test]
    fn test_read_chunked_decodes_body() {
        // Ollama answers many payloads with `Transfer-Encoding: chunked`
        // (issue `agnostic-rag-rlm-tool-3a68`). The decoder must strip the framing.
        let raw: Vec<u8> = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".to_vec();
        let mut reader = BufReader::new(std::io::Cursor::new(raw));
        let decoded = read_chunked(&mut reader).unwrap();
        assert_eq!(decoded, b"hello world");
    }
}

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::anyhow;
use serde_json::Value;

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
        let body = serde_json::json!({ "model": self.model, "input": texts }).to_string();
        let resp = http_post(&self.host, self.port, "/api/embed", &body)?;
        let value: Value = serde_json::from_str(&resp).map_err(|e| anyhow!("ollama json: {e}"))?;
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
            v.pop().ok_or_else(|| EmbeddingError::ModelNotLoaded("ollama empty".into()))
        })
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        self.post_embed(texts).map_err(|e| EmbeddingError::Candle(e.to_string()))
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

/// Minimal blocking HTTP/1.1 POST (no TLS, `Connection: close`) over
/// `TcpStream`. Sufficient for localhost Ollama; reads the body via
/// `Content-Length` and falls back to whatever the server sends on close.
fn http_post(host: &str, port: u16, path: &str, body: &str) -> anyhow::Result<String> {
    let mut stream = TcpStream::connect((host, port))
        .map_err(|e| anyhow!("ollama connect {host}:{port}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    let request = format!(
        "POST {path} HTTP/1.0\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\n\r\n{body}",
        len = body.len()
    );
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    parse_http_body(&response)
}

/// Extract the JSON body from a raw HTTP response, honoring `Content-Length`.
fn parse_http_body(response: &str) -> anyhow::Result<String> {
    let idx = response
        .find("\r\n\r\n")
        .ok_or_else(|| anyhow!("ollama response missing header/body separator"))?;
    let header = &response[..idx];
    let mut content_length = 0usize;
    for line in header.lines() {
        if let Some(val) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = val.trim().parse().unwrap_or(0);
        }
    }
    let body_bytes = response.as_bytes().get(idx + 4..).unwrap_or(b"");
    if content_length > 0 && body_bytes.len() >= content_length {
        Ok(String::from_utf8_lossy(&body_bytes[..content_length]).into_owned())
    } else {
        Ok(String::from_utf8_lossy(body_bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_base_default_port() {
        assert_eq!(parse_base("http://localhost:11434").unwrap(), ("localhost".into(), 11_434));
        assert_eq!(parse_base("localhost").unwrap(), ("localhost".into(), 11_434));
        assert_eq!(parse_base("http://ollama:9988/").unwrap(), ("ollama".into(), 9988));
    }

    #[test]
    fn test_parse_http_body_content_length() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n{\"a\":[1,2,3]}extra";
        assert_eq!(parse_http_body(raw).unwrap(), "{\"a\":[1,2,3]}");
    }
}

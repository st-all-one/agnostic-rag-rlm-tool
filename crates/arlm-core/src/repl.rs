use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Callback trait for making LLM calls from the REPL.
///
/// Implementations should be thread-safe and handle errors gracefully.
pub trait LlmCallback: Send + Sync {
    /// Make a single LLM completion call.
    ///
    /// # Arguments
    /// * `prompt` - The prompt to send to the LLM
    /// * `model` - Optional model name override
    ///
    /// # Returns
    /// The LLM response as a string, or an error message.
    fn query(&self, prompt: &str, model: Option<&str>) -> String;

    /// Make a batch of LLM completion calls (default: sequential).
    ///
    /// # Arguments
    /// * `prompts` - List of prompts to send
    /// * `model` - Optional model name override
    ///
    /// # Returns
    /// List of responses in the same order as input prompts.
    fn query_batched(&self, prompts: &[String], model: Option<&str>) -> Vec<String> {
        prompts.iter().map(|p| self.query(p, model)).collect()
    }
}

/// Request from a subprocess to make an LLM call.
///
/// Public so integration tests can construct requests when exercising the
/// [`LlmQueryServer`] over a real socket.
#[derive(Debug, Serialize, Deserialize)]
pub struct LlmRequest {
    pub id: String,
    pub prompt: String,
    pub model: Option<String>,
    pub batch: bool,
    pub prompts: Option<Vec<String>>,
}

/// Response to send back to the subprocess.
///
/// Public so integration tests can assert on responses when exercising [`LlmQueryServer`].
#[derive(Debug, Serialize, Deserialize)]
pub struct LlmResponse {
    pub id: String,
    pub success: bool,
    pub response: Option<String>,
    pub responses: Option<Vec<String>>,
    pub error: Option<String>,
}

/// TCP server that handles LLM queries from subprocesses.
pub struct LlmQueryServer {
    listener: TcpListener,
    shutdown_tx: mpsc::Sender<()>,
    shutdown_rx: Option<mpsc::Receiver<()>>,
    callback: Arc<dyn LlmCallback>,
}

impl LlmQueryServer {
    /// Create a new LLM query server.
    ///
    /// # Arguments
    /// * `callback` - The LLM callback to use for making calls
    ///
    /// # Returns
    /// A tuple of (server, port) where port is the port the server is listening on.
    ///
    /// # Errors
    ///
    /// Returns an error string if binding the listening socket fails.
    pub fn new(callback: Arc<dyn LlmCallback>) -> Result<Self, String> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|e| format!("failed to bind: {e}"))?;
        let _port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        // Set non-blocking mode for accept
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("failed to set non-blocking: {e}"))?;

        Ok(Self {
            listener,
            shutdown_tx,
            shutdown_rx: Some(shutdown_rx),
            callback,
        })
    }

    /// Get the port the server is listening on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.listener.local_addr().map_or(0, |addr| addr.port())
    }

    /// Start the server in a background thread.
    ///
    /// Returns a shutdown sender that can be used to stop the server.
    ///
    /// # Errors
    ///
    /// Returns an error string if the server has already been started or a new
    /// fallback socket cannot be bound.
    pub fn start(&mut self) -> Result<mpsc::Sender<()>, String> {
        let listener = std::mem::replace(
            &mut self.listener,
            TcpListener::bind("127.0.0.1:0").map_err(|e| format!("failed to bind: {e}"))?,
        );
        let callback = self.callback.clone();
        let shutdown_rx = self.shutdown_rx.take().ok_or("server already started")?;

        thread::spawn(move || {
            Self::run_server(listener, callback, shutdown_rx);
        });

        Ok(self.shutdown_tx.clone())
    }

    #[allow(clippy::needless_pass_by_value)]
    fn run_server(
        listener: TcpListener,
        callback: Arc<dyn LlmCallback>,
        shutdown_rx: mpsc::Receiver<()>,
    ) {
        // Set non-blocking for accept loop
        listener
            .set_nonblocking(true)
            .map_err(|e| e.to_string())
            .ok();

        loop {
            // Check for shutdown
            if shutdown_rx.try_recv().is_ok() {
                break;
            }

            // Try to accept a connection
            match listener.accept() {
                Ok((stream, _)) => {
                    let callback = callback.clone();
                    thread::spawn(move || {
                        Self::handle_connection(stream, callback.as_ref());
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    fn handle_connection(stream: TcpStream, callback: &dyn LlmCallback) {
        let Ok(reader_stream) = stream.try_clone() else {
            return;
        };
        let mut writer = BufWriter::new(stream);
        let mut reader = BufReader::new(reader_stream);

        loop {
            // Read length-prefixed message (4 bytes big-endian)
            let mut len_bytes = [0u8; 4];
            match reader.read_exact(&mut len_bytes) {
                Ok(()) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(_) => break,
            }

            let len = u32::from_be_bytes(len_bytes) as usize;
            if len > 10 * 1024 * 1024 {
                // 10MB limit
                break;
            }

            let mut buf = vec![0u8; len];
            if reader.read_exact(&mut buf).is_err() {
                break;
            }

            let request: LlmRequest = match serde_json::from_slice(&buf) {
                Ok(r) => r,
                Err(_) => break,
            };

            let response = if request.batch {
                let prompts = request.prompts.unwrap_or_default();
                let responses = callback.query_batched(&prompts, request.model.as_deref());
                LlmResponse {
                    id: request.id,
                    success: true,
                    response: None,
                    responses: Some(responses),
                    error: None,
                }
            } else {
                let response = callback.query(&request.prompt, request.model.as_deref());
                LlmResponse {
                    id: request.id,
                    success: true,
                    response: Some(response),
                    responses: None,
                    error: None,
                }
            };

            // Send response
            let response_json = serde_json::to_vec(&response)
                .map_err(|e| e.to_string())
                .unwrap_or_default();
            let response_len = u32::try_from(response_json.len()).unwrap_or(u32::MAX);
            if writer.write_all(&response_len.to_be_bytes()).is_err() {
                break;
            }
            if writer.write_all(&response_json).is_err() {
                break;
            }
            if writer.flush().is_err() {
                break;
            }
        }
    }
}

/// Result of executing a code block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub language: String,
    pub code: String,
}

/// A parsed code block from LLM output.
#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub language: String,
    pub code: String,
}

/// Executor for running code blocks in a sandboxed subprocess.
pub struct CodeExecutor {
    work_dir: PathBuf,
    #[allow(dead_code)]
    timeout: Duration,
    /// Optional address of the LLM query server (host:port).
    llm_server_addr: Option<String>,
}

impl CodeExecutor {
    #[must_use]
    pub fn new(work_dir: PathBuf, timeout: Duration) -> Self {
        Self {
            work_dir,
            timeout,
            llm_server_addr: None,
        }
    }

    /// Create an executor with LLM query support.
    #[must_use]
    pub fn with_llm_server(work_dir: PathBuf, timeout: Duration, llm_server_addr: String) -> Self {
        Self {
            work_dir,
            timeout,
            llm_server_addr: Some(llm_server_addr),
        }
    }

    #[must_use]
    pub fn default_executor() -> Self {
        let work_dir = std::env::temp_dir().join(format!("arlm-repl-{}", uuid::Uuid::now_v7()));
        Self::new(work_dir, Duration::from_secs(30))
    }

    /// Create a default executor with LLM query support.
    #[must_use]
    pub fn default_executor_with_llm(llm_server_addr: String) -> Self {
        let work_dir = std::env::temp_dir().join(format!("arlm-repl-{}", uuid::Uuid::now_v7()));
        Self::with_llm_server(work_dir, Duration::from_secs(30), llm_server_addr)
    }

    /// Create the working directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn setup(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.work_dir)
    }

    /// Execute a single code block and return the result.
    ///
    /// # Errors
    ///
    /// Returns an error if the subprocess cannot be spawned.
    pub fn execute(&self, block: &CodeBlock) -> Result<ReplResult, String> {
        self.setup()
            .map_err(|e| format!("failed to create work dir: {e}"))?;

        let start = Instant::now();
        let result = match block.language.as_str() {
            "python" | "py" => self.exec_python(&block.code),
            _ => self.exec_bash(&block.code),
        };

        match result {
            Ok(mut r) => {
                #[allow(clippy::cast_possible_truncation)]
                {
                    r.duration_ms = start.elapsed().as_millis() as u64;
                }
                r.language.clone_from(&block.language);
                r.code.clone_from(&block.code);
                Ok(r)
            }
            Err(e) => Ok(ReplResult {
                stdout: String::new(),
                stderr: e,
                exit_code: -1,
                #[allow(clippy::cast_possible_truncation)]
                duration_ms: start.elapsed().as_millis() as u64,
                language: block.language.clone(),
                code: block.code.clone(),
            }),
        }
    }

    fn exec_python(&self, code: &str) -> Result<ReplResult, String> {
        let script_path = self.work_dir.join("exec.py");

        // Build the full script with helper functions if LLM server is available
        let full_code = if let Some(ref addr) = self.llm_server_addr {
            format!(
                r#"
import json
import socket
import uuid
import sys

_LLM_SERVER = "{addr}"

def _send_request(request):
    """Send an LLM request to the server and return the response."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.settimeout(120)
    sock.connect((_LLM_SERVER.split(':')[0], int(_LLM_SERVER.split(':')[1])))
    
    request_json = json.dumps(request).encode('utf-8')
    sock.sendall(len(request_json).to_bytes(4, 'big') + request_json)
    
    # Read response
    len_bytes = sock.recv(4)
    if len(len_bytes) < 4:
        sock.close()
        return {{'success': False, 'error': 'failed to read response length'}}
    
    response_len = int.from_bytes(len_bytes, 'big')
    response_data = b''
    while len(response_data) < response_len:
        chunk = sock.recv(response_len - len(response_data))
        if not chunk:
            break
        response_data += chunk
    
    sock.close()
    return json.loads(response_data)

def llm_query(prompt, model=None):
    """Query the LLM with a single prompt (no REPL, no recursion)."""
    request = {{
        'id': str(uuid.uuid4()),
        'prompt': prompt,
        'model': model,
        'batch': False,
    }}
    response = _send_request(request)
    if not response.get('success'):
        return f"Error: {{response.get('error', 'unknown error')}}"
    return response.get('response', '')

def llm_query_batched(prompts, model=None):
    """Query the LLM with multiple prompts concurrently."""
    request = {{
        'id': str(uuid.uuid4()),
        'prompt': '',
        'model': model,
        'batch': True,
        'prompts': prompts,
    }}
    response = _send_request(request)
    if not response.get('success'):
        return [f"Error: {{response.get('error', 'unknown error')}}"] * len(prompts)
    return response.get('responses', [''] * len(prompts))

def rlm_query(prompt, model=None):
    """Spawn a recursive RLM sub-call. Falls back to llm_query for now."""
    return llm_query(prompt, model)

def rlm_query_batched(prompts, model=None):
    """Spawn recursive RLM sub-calls. Falls back to llm_query_batched."""
    return llm_query_batched(prompts, model)

# Global answer dict for signaling completion
answer = {{"content": "", "ready": False}}

def SHOW_VARS():
    """Show available variables."""
    return "Variables: answer, llm_query, llm_query_batched, rlm_query, rlm_query_batched"

{code}
"#
            )
        } else {
            code.to_string()
        };

        std::fs::write(&script_path, &full_code)
            .map_err(|e| format!("failed to write script: {e}"))?;

        let output = Command::new("python3")
            .arg(&script_path)
            .current_dir(&self.work_dir)
            .output()
            .map_err(|e| format!("failed to spawn python3: {e}"))?;

        Ok(ReplResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            duration_ms: 0,
            language: String::new(),
            code: String::new(),
        })
    }

    fn exec_bash(&self, code: &str) -> Result<ReplResult, String> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(code)
            .current_dir(&self.work_dir)
            .output()
            .map_err(|e| format!("failed to spawn sh: {e}"))?;

        Ok(ReplResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            duration_ms: 0,
            language: String::new(),
            code: String::new(),
        })
    }

    /// Clean up the working directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be removed.
    pub fn cleanup(&self) -> std::io::Result<()> {
        if self.work_dir.exists() {
            std::fs::remove_dir_all(&self.work_dir)?;
        }
        Ok(())
    }
}

impl Drop for CodeExecutor {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Parse code blocks from LLM output. Looks for ```lang ... ``` patterns.
#[must_use]
pub fn find_code_blocks(text: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        // Look for opening ```
        if ch == '`' && text[i..].starts_with("```") {
            let after_fence = i + 3;
            // Find end of language tag line
            if let Some(lang_end) = text[after_fence..].find('\n') {
                let lang = text[after_fence..after_fence + lang_end].trim();
                // Find closing ```
                let code_start = after_fence + lang_end + 1;
                if let Some(close_pos) = text[code_start..].find("```") {
                    let code = text[code_start..code_start + close_pos].trim();
                    if !code.is_empty() {
                        blocks.push(CodeBlock {
                            language: lang.to_string(),
                            code: code.to_string(),
                        });
                    }
                    // Skip past closing ```
                    let skip_to = code_start + close_pos + 3;
                    // Advance chars iterator to skip_to
                    while let Some(&(pos, _)) = chars.peek() {
                        if pos >= skip_to {
                            break;
                        }
                        chars.next();
                    }
                }
            }
        }
    }

    blocks
}

/// Format a REPL result for inclusion in conversation history.
#[must_use]
pub fn format_repl_result(result: &ReplResult) -> String {
    let mut out = format!(
        "[{}] exit_code={} duration={}ms\n",
        result.language, result.exit_code, result.duration_ms
    );
    if !result.stdout.is_empty() {
        out.push_str("stdout:\n");
        out.push_str(&result.stdout);
        if !result.stdout.ends_with('\n') {
            out.push('\n');
        }
    }
    if !result.stderr.is_empty() {
        out.push_str("stderr:\n");
        out.push_str(&result.stderr);
        if !result.stderr.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

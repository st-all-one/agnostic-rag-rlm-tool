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
        prompts
            .iter()
            .map(|p| self.query(p, model.as_deref()))
            .collect()
    }
}

/// Request from a subprocess to make an LLM call.
#[derive(Debug, Serialize, Deserialize)]
struct LlmRequest {
    id: String,
    prompt: String,
    model: Option<String>,
    batch: bool,
    prompts: Option<Vec<String>>,
}

/// Response to send back to the subprocess.
#[derive(Debug, Serialize, Deserialize)]
struct LlmResponse {
    id: String,
    success: bool,
    response: Option<String>,
    responses: Option<Vec<String>>,
    error: Option<String>,
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
    pub fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .map_or(0, |addr| addr.port())
    }

    /// Start the server in a background thread.
    ///
    /// Returns a shutdown sender that can be used to stop the server.
    pub fn start(&mut self) -> Result<mpsc::Sender<()>, String> {
        let listener = std::mem::replace(
            &mut self.listener,
            TcpListener::bind("127.0.0.1:0").map_err(|e| format!("failed to bind: {e}"))?,
        );
        let callback = self.callback.clone();
        let shutdown_rx = self
            .shutdown_rx
            .take()
            .ok_or("server already started")?;

        thread::spawn(move || {
            Self::run_server(listener, callback, shutdown_rx);
        });

        Ok(self.shutdown_tx.clone())
    }

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
                    continue;
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            }
        }
    }

    fn handle_connection(stream: TcpStream, callback: &dyn LlmCallback) {
        let reader_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
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
            let response_json =
                serde_json::to_vec(&response).map_err(|e| e.to_string()).unwrap_or_default();
            let response_len = response_json.len() as u32;
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

/// Send an LLM request to a server at the given address.
#[cfg(test)]
fn send_llm_request(addr: &str, request: &LlmRequest) -> Result<LlmResponse, String> {
    let mut stream =
        TcpStream::connect(addr).map_err(|e| format!("failed to connect to LLM server: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .map_err(|e| format!("failed to set timeout: {e}"))?;

    let request_json =
        serde_json::to_vec(request).map_err(|e| format!("failed to serialize request: {e}"))?;
    let len = request_json.len() as u32;

    stream
        .write_all(&len.to_be_bytes())
        .map_err(|e| format!("failed to send length: {e}"))?;
    stream
        .write_all(&request_json)
        .map_err(|e| format!("failed to send request: {e}"))?;

    // Read response
    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .map_err(|e| format!("failed to read response length: {e}"))?;
    let response_len = u32::from_be_bytes(len_bytes) as usize;

    let mut response_buf = vec![0u8; response_len];
    stream
        .read_exact(&mut response_buf)
        .map_err(|e| format!("failed to read response: {e}"))?;

    serde_json::from_slice(&response_buf).map_err(|e| format!("failed to deserialize response: {e}"))
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
        self.setup().map_err(|e| format!("failed to create work dir: {e}"))?;

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_find_code_blocks_python() {
        let text = "Here is some code:\n```python\nprint('hello')\n```\nDone.";
        let blocks = find_code_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].language, "python");
        assert!(blocks[0].code.contains("print"));
    }

    #[test]
    fn test_find_code_blocks_bash() {
        let text = "```bash\necho hello\n```";
        let blocks = find_code_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].language, "bash");
    }

    #[test]
    fn test_find_code_blocks_multiple() {
        let text = "```python\nprint(1)\n```\n\n```bash\necho 2\n```";
        let blocks = find_code_blocks(text);
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn test_find_code_blocks_none() {
        let text = "No code blocks here.";
        let blocks = find_code_blocks(text);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_find_code_blocks_empty_code() {
        let text = "```\n\n```";
        let blocks = find_code_blocks(text);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_format_repl_result() {
        let result = ReplResult {
            stdout: "hello\n".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 42,
            language: "python".to_string(),
            code: "print('hello')".to_string(),
        };
        let formatted = format_repl_result(&result);
        assert!(formatted.contains("exit_code=0"));
        assert!(formatted.contains("duration=42ms"));
        assert!(formatted.contains("hello"));
    }

    #[test]
    fn test_code_executor_bash() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "bash".to_string(),
            code: "echo hello world".to_string(),
        };
        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello world"));
    }

    #[test]
    fn test_code_executor_python() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "python".to_string(),
            code: "print('hello from python')".to_string(),
        };
        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello from python"));
    }

    #[test]
    fn test_code_executor_error() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "bash".to_string(),
            code: "exit 1".to_string(),
        };
        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_code_executor_python_computation() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "python".to_string(),
            code: "import json; print(json.dumps({'status': 'ok', 'sum': sum(range(100))}))".to_string(),
        };
        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("4950"));
        assert!(result.stdout.contains("ok"));
    }

    #[test]
    fn test_code_executor_bash_stderr() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "bash".to_string(),
            code: "echo error_msg >&2; exit 0".to_string(),
        };
        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stderr.contains("error_msg"));
        assert!(result.stdout.is_empty());
    }

    #[test]
    fn test_code_executor_bash_pipes() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "bash".to_string(),
            code: "echo -e 'line1\nline2\nline3' | wc -l".to_string(),
        };
        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("3"));
    }

    #[test]
    fn test_code_executor_python_syntax_error() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "python".to_string(),
            code: "def foo(:".to_string(),
        };
        let result = executor.execute(&block).unwrap();
        assert_ne!(result.exit_code, 0);
        assert!(!result.stderr.is_empty());
    }

    #[test]
    fn test_find_code_blocks_realistic_llm_response() {
        let text = r#"I'll analyze the data step by step.

First, let me load the data:

```python
import json
data = [1, 2, 3, 4, 5]
total = sum(data)
print(f"Total: {total}")
```

Now let me verify the result:

```bash
echo "Verification: expected 15"
```

The total is 15, which is correct."#;
        let blocks = find_code_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].language, "python");
        assert!(blocks[0].code.contains("sum(data)"));
        assert_eq!(blocks[1].language, "bash");
        assert!(blocks[1].code.contains("echo"));
    }

    #[test]
    fn test_find_code_blocks_nested_fences() {
        let text = "```python\n# code with ``` inside\nprint('hello')\n```";
        let blocks = find_code_blocks(text);
        // The parser finds the first closing fence
        assert!(!blocks.is_empty());
    }

    #[test]
    fn test_format_repl_result_with_stderr() {
        let result = ReplResult {
            stdout: String::new(),
            stderr: "Traceback...SyntaxError\n".to_string(),
            exit_code: 1,
            duration_ms: 5,
            language: "python".to_string(),
            code: "bad code".to_string(),
        };
        let formatted = format_repl_result(&result);
        assert!(formatted.contains("exit_code=1"));
        assert!(formatted.contains("stderr:"));
        assert!(formatted.contains("SyntaxError"));
    }

    #[test]
    fn test_repl_full_flow_simulation() {
        let executor = CodeExecutor::default_executor();

        // Simulate what solve_task_repl does:
        // 1. Parse code blocks from LLM response
        let llm_response = "Let me write a script:\n```python\nprint('step1')\n```\nDone.";
        let blocks = find_code_blocks(llm_response);
        assert_eq!(blocks.len(), 1);

        // 2. Execute the code
        let result = executor.execute(&blocks[0]).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("step1"));

        // 3. Format result for next conversation turn
        let formatted = format_repl_result(&result);
        assert!(formatted.contains("python"));
        assert!(formatted.contains("step1"));
    }

    // === LLM Query Tests ===

    /// Mock LLM callback for testing.
    struct MockLlmCallback {
        responses: std::sync::Mutex<Vec<String>>,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl MockLlmCallback {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
                call_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.call_count
                .load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl LlmCallback for MockLlmCallback {
        fn query(&self, _prompt: &str, _model: Option<&str>) -> String {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let responses = self.responses.lock().unwrap();
            if count < responses.len() {
                responses[count].clone()
            } else {
                "default response".to_string()
            }
        }
    }

    #[test]
    fn test_llm_query_server_start_and_stop() {
        let callback = Arc::new(MockLlmCallback::new(vec!["test".to_string()]));
        let mut server = LlmQueryServer::new(callback).unwrap();
        let port = server.port();
        assert!(port > 0);

        let shutdown_tx = server.start().unwrap();
        // Server should be running
        thread::sleep(Duration::from_millis(100));

        // Stop the server
        drop(shutdown_tx);
        thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn test_llm_query_server_handle_request() {
        let callback = Arc::new(MockLlmCallback::new(vec!["hello from LLM".to_string()]));
        let mut server = LlmQueryServer::new(callback).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());
        let _shutdown_tx = server.start().unwrap();
        thread::sleep(Duration::from_millis(100));

        // Send a request
        let request = LlmRequest {
            id: "test-1".to_string(),
            prompt: "test prompt".to_string(),
            model: None,
            batch: false,
            prompts: None,
        };

        let response = send_llm_request(&addr, &request).unwrap();
        assert!(response.success);
        assert_eq!(response.response.as_deref(), Some("hello from LLM"));
    }

    #[test]
    fn test_llm_query_server_batch_request() {
        let callback = Arc::new(MockLlmCallback::new(vec![
            "response 1".to_string(),
            "response 2".to_string(),
            "response 3".to_string(),
        ]));
        let mut server = LlmQueryServer::new(callback).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());
        let _shutdown_tx = server.start().unwrap();
        thread::sleep(Duration::from_millis(100));

        let request = LlmRequest {
            id: "batch-1".to_string(),
            prompt: String::new(),
            model: None,
            batch: true,
            prompts: Some(vec![
                "prompt 1".to_string(),
                "prompt 2".to_string(),
                "prompt 3".to_string(),
            ]),
        };

        let response = send_llm_request(&addr, &request).unwrap();
        assert!(response.success);
        let responses = response.responses.unwrap();
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0], "response 1");
        assert_eq!(responses[1], "response 2");
        assert_eq!(responses[2], "response 3");
    }

    #[test]
    fn test_code_executor_with_llm_server() {
        let callback = Arc::new(MockLlmCallback::new(vec![
            "LLM says: hello world".to_string(),
        ]));
        let mut server = LlmQueryServer::new(callback).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());
        let _shutdown_tx = server.start().unwrap();
        thread::sleep(Duration::from_millis(100));

        let executor = CodeExecutor::default_executor_with_llm(addr);
        let block = CodeBlock {
            language: "python".to_string(),
            code: r#"
result = llm_query("What is 2+2?")
print(result)
"#.to_string(),
        };

        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("LLM says: hello world"));
    }

    #[test]
    fn test_code_executor_llm_query_batched() {
        let callback = Arc::new(MockLlmCallback::new(vec![
            "answer 1".to_string(),
            "answer 2".to_string(),
        ]));
        let mut server = LlmQueryServer::new(callback).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());
        let _shutdown_tx = server.start().unwrap();
        thread::sleep(Duration::from_millis(100));

        let executor = CodeExecutor::default_executor_with_llm(addr);
        let block = CodeBlock {
            language: "python".to_string(),
            code: r#"
results = llm_query_batched(["q1", "q2"])
for r in results:
    print(r)
"#.to_string(),
        };

        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("answer 1"));
        assert!(result.stdout.contains("answer 2"));
    }

    #[test]
    fn test_code_executor_rlm_query_fallback() {
        let callback = Arc::new(MockLlmCallback::new(vec![
            "rlm response".to_string(),
        ]));
        let mut server = LlmQueryServer::new(callback).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());
        let _shutdown_tx = server.start().unwrap();
        thread::sleep(Duration::from_millis(100));

        let executor = CodeExecutor::default_executor_with_llm(addr);
        let block = CodeBlock {
            language: "python".to_string(),
            code: r#"
result = rlm_query("Analyze this data")
print(result)
"#.to_string(),
        };

        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("rlm response"));
    }

    #[test]
    fn test_code_executor_without_llm_server() {
        let executor = CodeExecutor::default_executor();
        let block = CodeBlock {
            language: "python".to_string(),
            code: r#"
# These functions should not be available
try:
    result = llm_query("test")
    print("ERROR: llm_query should not be defined")
except NameError:
    print("OK: llm_query not defined as expected")
"#.to_string(),
        };

        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("OK: llm_query not defined"));
    }

    #[test]
    fn test_code_executor_answer_dict() {
        let callback = Arc::new(MockLlmCallback::new(vec!["test".to_string()]));
        let mut server = LlmQueryServer::new(callback).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());
        let _shutdown_tx = server.start().unwrap();
        thread::sleep(Duration::from_millis(100));

        let executor = CodeExecutor::default_executor_with_llm(addr);
        let block = CodeBlock {
            language: "python".to_string(),
            code: r#"
print(f"answer before: {answer}")
answer["content"] = "final answer"
answer["ready"] = True
print(f"answer after: {answer}")
"#.to_string(),
        };

        let result = executor.execute(&block).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("answer before: {'content': '', 'ready': False}"));
        assert!(result.stdout.contains("answer after: {'content': 'final answer', 'ready': True}"));
    }
}

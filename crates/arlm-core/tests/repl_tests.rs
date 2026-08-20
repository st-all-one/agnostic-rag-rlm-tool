#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use arlm_core::repl::{
    CodeBlock, CodeExecutor, LlmCallback, LlmQueryServer, LlmRequest, LlmResponse, ReplResult,
    find_code_blocks, format_repl_result,
};

/// Send an LLM request to a server at the given address (mirrors the in-crate helper).
fn send_llm_request(addr: &str, request: &LlmRequest) -> Result<LlmResponse, String> {
    let mut stream =
        TcpStream::connect(addr).map_err(|e| format!("failed to connect to LLM server: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .map_err(|e| format!("failed to set timeout: {e}"))?;

    let request_json =
        serde_json::to_vec(request).map_err(|e| format!("failed to serialize request: {e}"))?;
    #[allow(clippy::cast_possible_truncation)]
    let len = request_json.len() as u32;

    stream
        .write_all(&len.to_be_bytes())
        .map_err(|e| format!("failed to send length: {e}"))?;
    stream
        .write_all(&request_json)
        .map_err(|e| format!("failed to send request: {e}"))?;

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
        exit_code: 42,
        duration_ms: 42,
        language: "python".to_string(),
        code: "print('hello')".to_string(),
    };
    let formatted = format_repl_result(&result);
    assert!(formatted.contains("exit_code=42"));
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
    assert!(result.stdout.contains('3'));
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

    let llm_response = "Let me write a script:\n```python\nprint('step1')\n```\nDone.";
    let blocks = find_code_blocks(llm_response);
    assert_eq!(blocks.len(), 1);

    let result = executor.execute(&blocks[0]).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("step1"));

    let formatted = format_repl_result(&result);
    assert!(formatted.contains("python"));
    assert!(formatted.contains("step1"));
}

// === LLM Query Tests ===

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
    thread::sleep(Duration::from_millis(100));

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
    let callback = Arc::new(MockLlmCallback::new(vec!["LLM says: hello world".to_string()]));
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
"#
        .to_string(),
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
"#
        .to_string(),
    };

    let result = executor.execute(&block).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("answer 1"));
    assert!(result.stdout.contains("answer 2"));
}

#[test]
fn test_code_executor_rlm_query_fallback() {
    let callback = Arc::new(MockLlmCallback::new(vec!["rlm response".to_string()]));
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
"#
        .to_string(),
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
"#
        .to_string(),
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
"#
        .to_string(),
    };

    let result = executor.execute(&block).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("answer before: {'content': '', 'ready': False}"));
    assert!(result.stdout.contains("answer after: {'content': 'final answer', 'ready': True}"));
}

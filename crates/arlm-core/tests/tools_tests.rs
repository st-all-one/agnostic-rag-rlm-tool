#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use arlm_core::tools::{CodeSearch, ExecutableTool, SearchCodeTool, ToolRegistry};
use arlm_core::CustomTool;

struct FakeSearch;

impl CodeSearch for FakeSearch {
    fn search(&self, query: &str, limit: u64) -> Result<String, String> {
        Ok(format!("results for '{query}' (limit {limit})"))
    }
}

#[test]
fn test_search_code_tool_not_configured_is_honest() {
    let tool = SearchCodeTool::new();
    assert!(!tool.is_configured());
    let out = tool
        .execute(r#"{"query": "foo", "limit": 5}"#)
        .expect("execute ok");
    assert!(out.contains("search_code not configured"));
    assert!(out.contains("no code-search backend provided"));
    assert!(!out.contains("[placeholder"));
}

#[test]
fn test_search_code_tool_with_backend() {
    let tool = SearchCodeTool::with_search(Arc::new(FakeSearch));
    assert!(tool.is_configured());
    let out = tool
        .execute(r#"{"query": "foo", "limit": 5}"#)
        .expect("execute ok");
    assert_eq!(out, "results for 'foo' (limit 5)");
}

#[test]
fn test_search_code_tool_name_and_params() {
    let tool = SearchCodeTool::new();
    assert_eq!(tool.name(), "search_code");
    assert_eq!(tool.parameters(), Some("query: str, limit: int = 10"));
}

#[test]
fn test_tool_registry_register_and_execute() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SearchCodeTool::new()));
    assert!(registry.has_tool("search_code"));
    let out = registry
        .execute("search_code", r#"{"query": "x"}"#)
        .expect("exec");
    assert!(out.contains("not configured"));
    let tools = registry.to_custom_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search_code");
}

#[test]
fn test_custom_tool_via_registry_prompt() {
    let ct = CustomTool::function("read_file", "Read a file").with_parameters("path: str");
    assert_eq!(ct.name, "read_file");
    assert!(ct.callable);
}

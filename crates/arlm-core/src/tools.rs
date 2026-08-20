use std::sync::Arc;

use crate::types::CustomTool;

/// Trait for executable tools that can be called by the solver.
///
/// Implementations should be thread-safe and handle errors gracefully.
pub trait ExecutableTool: Send + Sync {
    /// Execute the tool with the given arguments.
    ///
    /// # Arguments
    /// * `args` - JSON string containing the tool arguments
    ///
    /// # Errors
    /// Returns an error message string on failure.
    fn execute(&self, args: &str) -> Result<String, String>;

    /// Get the tool's name.
    fn name(&self) -> &str;

    /// Get the tool's description.
    fn description(&self) -> &str;

    /// Get the tool's parameter schema (optional).
    fn parameters(&self) -> Option<&str> {
        None
    }
}

/// Registry of executable tools.
pub struct ToolRegistry {
    tools: std::collections::HashMap<String, Box<dyn ExecutableTool>>,
}

impl ToolRegistry {
    /// Create a new empty tool registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
        }
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Box<dyn ExecutableTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Execute a tool by name.
    ///
    /// # Errors
    /// Returns an error if the tool is not registered or execution fails.
    pub fn execute(&self, name: &str, args: &str) -> Result<String, String> {
        self.tools
            .get(name)
            .ok_or_else(|| format!("tool '{name}' not found"))?
            .execute(args)
    }

    /// Check if a tool exists.
    #[must_use]
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get all tool names.
    #[must_use]
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(std::string::String::as_str).collect()
    }

    /// Convert to `CustomTool` list for prompt injection.
    #[must_use]
    pub fn to_custom_tools(&self) -> Vec<CustomTool> {
        self.tools
            .values()
            .map(|tool| {
                let mut ct = CustomTool::function(tool.name(), tool.description());
                if let Some(params) = tool.parameters() {
                    ct = ct.with_parameters(params);
                }
                ct
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Abstraction over a code-search backend (e.g. `arlm-search`).
///
/// This trait decouples `arlm-core` from any concrete search implementation so the
/// crate stays free of a hard dependency on `arlm-search`. Callers inject a backend
/// via [`SearchCodeTool::with_search`]; when none is configured the tool returns an
/// honest "not configured" message instead of a fake placeholder.
pub trait CodeSearch: Send + Sync {
    /// Search the codebase for `query`, returning at most `limit` results.
    ///
    /// # Errors
    /// Returns an error message string when the backend fails.
    fn search(&self, query: &str, limit: u64) -> Result<String, String>;
}

/// Built-in tool: search codebase.
///
/// When a [`CodeSearch`] backend is injected it performs a real search; otherwise it
/// returns an explicit, honest message that no backend was configured (no fake output).
#[derive(Default)]
pub struct SearchCodeTool {
    search: Option<Arc<dyn CodeSearch>>,
}

impl SearchCodeTool {
    /// Create a search tool with no backend configured.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a search tool wired to a concrete search backend.
    #[must_use]
    pub fn with_search(search: Arc<dyn CodeSearch>) -> Self {
        Self {
            search: Some(search),
        }
    }

    /// Whether a search backend is currently configured.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.search.is_some()
    }
}

impl ExecutableTool for SearchCodeTool {
    fn execute(&self, args: &str) -> Result<String, String> {
        let parsed: serde_json::Value =
            serde_json::from_str(args).map_err(|e| format!("invalid JSON args: {e}"))?;
        let query = parsed["query"].as_str().ok_or("missing 'query' argument")?;
        let limit = parsed["limit"].as_u64().unwrap_or(10);

        match &self.search {
            Some(backend) => backend.search(query, limit),
            None => Ok(format!(
                "search_code not configured: no code-search backend provided (query={query}, limit={limit})"
            )),
        }
    }

    fn name(&self) -> &'static str {
        "search_code"
    }

    fn description(&self) -> &'static str {
        "Search the codebase for code matching a query"
    }

    fn parameters(&self) -> Option<&str> {
        Some("query: str, limit: int = 10")
    }
}

/// Built-in tool: read file.
pub struct ReadFileTool;

impl ExecutableTool for ReadFileTool {
    fn execute(&self, args: &str) -> Result<String, String> {
        let parsed: serde_json::Value =
            serde_json::from_str(args).map_err(|e| format!("invalid JSON args: {e}"))?;
        let path = parsed["path"].as_str().ok_or("missing 'path' argument")?;

        std::fs::read_to_string(path).map_err(|e| format!("failed to read file: {e}"))
    }

    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read the contents of a file"
    }

    fn parameters(&self) -> Option<&str> {
        Some("path: str")
    }
}

/// Built-in tool: list files.
pub struct ListFilesTool;

impl ExecutableTool for ListFilesTool {
    fn execute(&self, args: &str) -> Result<String, String> {
        let parsed: serde_json::Value =
            serde_json::from_str(args).map_err(|e| format!("invalid JSON args: {e}"))?;
        let path = parsed["path"].as_str().unwrap_or(".");

        let entries =
            std::fs::read_dir(path).map_err(|e| format!("failed to read directory: {e}"))?;

        let mut result = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("failed to read entry: {e}"))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("failed to get type: {e}"))?;
            let prefix = if file_type.is_dir() { "d " } else { "f " };
            result.push(format!("{prefix}{name}"));
        }

        Ok(result.join("\n"))
    }

    fn name(&self) -> &'static str {
        "list_files"
    }

    fn description(&self) -> &'static str {
        "List files and directories in a path"
    }

    fn parameters(&self) -> Option<&str> {
        Some("path: str = '.'")
    }
}

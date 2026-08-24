use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    /// Sampling seed for reproducible outputs (propagated to backends that support it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Function-calling tool definitions (propagated to backends that support it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
}

/// A function-calling tool definition.
///
/// `parameters` is a JSON Schema object describing the tool's arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => write!(f, "system"),
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub model: String,
    pub usage: UsageSummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Estimated cost in USD (filled by the transport layer via the default pricing table).
    #[serde(default)]
    pub cost_usd: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("HTTP error: {status}")]
    Http { status: u16, body: String },
    #[error("API error: {message}")]
    Api { message: String },
    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Request timed out after {0}ms")]
    Timeout(u64),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Backend error: {0}")]
    Backend(String),
}

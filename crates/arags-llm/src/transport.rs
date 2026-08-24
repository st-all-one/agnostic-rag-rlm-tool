use reqwest::Client;
use serde::Serialize;
use tracing::info;

use crate::pricing::estimate_default;
use crate::retry::{RetryConfig, retry_with_backoff};
use crate::types::{CompletionResponse, LlmError};

/// Extract an error message from a provider error body that follows the
/// `{ "error": { "message": "..." } }` convention (`OpenAI`, `Anthropic`,
/// `DeepSeek`, `MiMo`, `Gemini`). Falls back to the raw body when unparseable.
#[must_use]
pub fn extract_json_error_message(body: &str) -> String {
    #[derive(serde::Deserialize)]
    struct ErrBody {
        error: ErrMessage,
    }
    #[derive(serde::Deserialize)]
    struct ErrMessage {
        message: String,
    }
    match serde_json::from_str::<ErrBody>(body) {
        Ok(e) => e.error.message,
        Err(_) => body.to_string(),
    }
}

/// Shared HTTP completion transport for all backends.
///
/// Performs the POST, status/429 handling, error extraction and retry with
/// exponential backoff. Provider-specific behaviour is supplied via the
/// `extract_error` (error body -> message) and `map_success` (2xx body ->
/// [`CompletionResponse`]) closures. The `cost_usd` field of the response is
/// filled from the default pricing table.
pub(crate) async fn request_completion<R, E, M>(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
    body: &R,
    retry: &RetryConfig,
    extract_error: E,
    map_success: M,
) -> Result<CompletionResponse, LlmError>
where
    R: Serialize + Clone,
    E: Fn(u16, &str) -> String + Clone,
    M: Fn(&str) -> Result<CompletionResponse, LlmError> + Clone,
{
    let _timer = crate::Timer::new("http_completion");
    let url = url.to_string();
    let headers = headers.to_vec();
    let body = body.clone();

    retry_with_backoff(retry, || {
        let client = client.clone();
        let url = url.clone();
        let headers = headers.clone();
        let body = body.clone();
        let extract_error = extract_error.clone();
        let map_success = map_success.clone();
        async move {
            let mut builder = client.post(&url);
            for (k, v) in &headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            let resp = builder
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Connection(e.to_string()))?;

            let status = resp.status();
            let body_text = resp
                .text()
                .await
                .map_err(|e| LlmError::Connection(e.to_string()))?;

            if status == 429 {
                return Err(LlmError::RateLimited {
                    retry_after_ms: 1000,
                });
            }
            if !status.is_success() {
                let message = extract_error(status.as_u16(), &body_text);
                return Err(LlmError::Http {
                    status: status.as_u16(),
                    body: message,
                });
            }

            let mut response = map_success(&body_text)?;
            response.usage.cost_usd = estimate_default(&response.model, &response.usage);
            info!(model = %response.model, "completion finished");
            Ok(response)
        }
    })
    .await
}

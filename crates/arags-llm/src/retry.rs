use std::time::Duration;

use tracing::warn;

use crate::types::LlmError;

const DEFAULT_MAX_RETRIES: u32 = 3;
const DEFAULT_BASE_DELAY_MS: u64 = 1000;
const DEFAULT_MAX_DELAY_MS: u64 = 30_000;
const BACKOFF_FACTOR: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            base_delay_ms: DEFAULT_BASE_DELAY_MS,
            max_delay_ms: DEFAULT_MAX_DELAY_MS,
        }
    }
}

impl RetryConfig {
    #[must_use]
    pub fn new(max_retries: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            max_retries,
            base_delay_ms,
            max_delay_ms,
        }
    }

    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            clippy::cast_sign_loss
        )]
        {
            let delay_ms = (self.base_delay_ms as f64) * BACKOFF_FACTOR.powi(attempt as i32);
            let delay_ms = delay_ms.min(self.max_delay_ms as f64) as u64;
            Duration::from_millis(delay_ms)
        }
    }
}

/// Whether an error is transient and worth retrying.
fn is_retryable(error: &LlmError) -> bool {
    match error {
        LlmError::RateLimited { .. } | LlmError::Connection(_) | LlmError::Timeout(_) => true,
        LlmError::Http { status, .. } => *status == 429 || (500..600).contains(status),
        _ => false,
    }
}

/// Retry an async operation with exponential backoff.
///
/// Retries on rate limiting (429), server errors (5xx), connection errors and
/// timeouts. Other errors are returned immediately without retrying.
///
/// # Errors
///
/// Returns [`LlmError`] if all retries are exhausted or a non-retryable error occurs.
pub async fn retry_with_backoff<F, Fut, T>(config: &RetryConfig, mut f: F) -> Result<T, LlmError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, LlmError>>,
{
    let mut last_error = None;

    for attempt in 0..=config.max_retries {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let retryable = is_retryable(&e);
                if !retryable || attempt >= config.max_retries {
                    return Err(e);
                }

                let delay = match &e {
                    LlmError::RateLimited { retry_after_ms } => {
                        Duration::from_millis(*retry_after_ms)
                    }
                    _ => config.delay_for_attempt(attempt),
                };

                warn!(
                    attempt = attempt,
                    delay_ms = delay.as_millis(),
                    error = %e,
                    "transient error, retrying"
                );
                tokio::time::sleep(delay).await;
                last_error = Some(e);
            }
        }
    }

    last_error.map_or_else(
        || Err(LlmError::Backend("max retries exceeded".to_string())),
        Err,
    )
}

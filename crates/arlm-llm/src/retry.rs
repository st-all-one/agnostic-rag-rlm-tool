use std::time::Duration;

use tracing::info;

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

/// Retry an async operation with exponential backoff.
///
/// Retries on rate limiting (429) and server errors (5xx).
/// Other errors are returned immediately without retrying.
///
/// # Errors
///
/// Returns `LlmError` if all retries are exhausted or a non-retryable error occurs.
pub async fn retry_with_backoff<F, Fut, T>(config: &RetryConfig, mut f: F) -> Result<T, LlmError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, LlmError>>,
{
    let mut last_error = None;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            let delay = match &last_error {
                Some(LlmError::RateLimited { retry_after_ms }) => {
                    Duration::from_millis(*retry_after_ms)
                }
                _ => config.delay_for_attempt(attempt - 1),
            };
            info!(
                attempt = attempt,
                delay_ms = delay.as_millis(),
                "retrying after backoff"
            );
            tokio::time::sleep(delay).await;
        }

        match f().await {
            Ok(result) => return Ok(result),
            Err(LlmError::RateLimited { retry_after_ms }) if attempt < config.max_retries => {
                info!(
                    attempt = attempt,
                    retry_after_ms = retry_after_ms,
                    "rate limited, will retry"
                );
                last_error = Some(LlmError::RateLimited { retry_after_ms });
            }
            Err(LlmError::Http { status, .. }) if status == 429 || (500..600).contains(&status) => {
                if attempt < config.max_retries {
                    #[allow(clippy::cast_possible_truncation)]
                    let retry_after_ms = config.delay_for_attempt(attempt).as_millis() as u64;
                    info!(
                        attempt = attempt,
                        status = status,
                        retry_after_ms = retry_after_ms,
                        "server error, will retry"
                    );
                    last_error = Some(LlmError::RateLimited { retry_after_ms });
                } else {
                    return Err(LlmError::Http {
                        status,
                        body: String::new(),
                    });
                }
            }
            Err(e) => return Err(e),
        }
    }

    last_error.map_or_else(
        || Err(LlmError::Backend("max retries exceeded".to_string())),
        Err,
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::duration_suboptimal_units
)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay_ms, 1000);
        assert_eq!(config.max_delay_ms, 30_000);
    }

    #[test]
    fn test_delay_for_attempt() {
        let config = RetryConfig::default();
        assert_eq!(config.delay_for_attempt(0), Duration::from_millis(1000));
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(2000));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(4000));
    }

    #[test]
    fn test_delay_capped_at_max() {
        let config = RetryConfig::new(5, 1000, 5000);
        assert_eq!(config.delay_for_attempt(10), Duration::from_millis(5000));
    }

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let config = RetryConfig::default();
        let result = retry_with_backoff(&config, || async { Ok::<_, LlmError>(42) }).await;
        assert_eq!(result.expect("should succeed"), 42);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let config = RetryConfig::new(3, 10, 100);
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result: Result<i32, LlmError> = retry_with_backoff(&config, || {
            let c = counter_clone.clone();
            async move {
                let attempt = c.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(LlmError::Http {
                        status: 500,
                        body: "temporary".to_string(),
                    })
                } else {
                    Ok(99)
                }
            }
        })
        .await;

        assert_eq!(result.expect("should succeed"), 99);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let config = RetryConfig::new(2, 10, 100);
        let result: Result<i32, LlmError> = retry_with_backoff(&config, || async {
            Err(LlmError::Backend("permanent".to_string()))
        })
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_retry_auth_not_retried() {
        let config = RetryConfig::new(3, 10, 100);
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result: Result<i32, LlmError> = retry_with_backoff(&config, || {
            let c = counter_clone.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(LlmError::Auth("invalid key".to_string()))
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}

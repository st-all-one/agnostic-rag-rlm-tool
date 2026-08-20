#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_borrow,
    clippy::unnecessary_literal_bound,
    clippy::float_cmp,
    clippy::duration_suboptimal_units,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use arlm_llm::retry::{RetryConfig, retry_with_backoff};
use arlm_llm::types::LlmError;

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
    assert_eq!(
        config.delay_for_attempt(0),
        std::time::Duration::from_millis(1000)
    );
    assert_eq!(
        config.delay_for_attempt(1),
        std::time::Duration::from_millis(2000)
    );
    assert_eq!(
        config.delay_for_attempt(2),
        std::time::Duration::from_millis(4000)
    );
}

#[test]
fn test_delay_capped_at_max() {
    let config = RetryConfig::new(5, 1000, 5000);
    assert_eq!(
        config.delay_for_attempt(10),
        std::time::Duration::from_millis(5000)
    );
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

#[tokio::test]
async fn test_retry_connection_is_retried() {
    let config = RetryConfig::new(2, 1, 10);
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let result: Result<i32, LlmError> = retry_with_backoff(&config, || {
        let c = counter_clone.clone();
        async move {
            let attempt = c.fetch_add(1, Ordering::SeqCst);
            if attempt < 1 {
                Err(LlmError::Connection("refused".to_string()))
            } else {
                Ok(7)
            }
        }
    })
    .await;

    assert_eq!(result.expect("should succeed"), 7);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

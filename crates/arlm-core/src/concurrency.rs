use std::future::Future;

use anyhow::Result;
use futures::stream::{self, StreamExt};

/// Execute items concurrently with bounded parallelism using `buffer_unordered`.
///
/// Returns results in the same order as inputs (via collect on the stream).
///
/// # Errors
///
/// Returns the first error encountered; remaining tasks continue but their
/// results are discarded.
pub async fn map_concurrent<T, F, Fut, R>(items: Vec<T>, concurrency: usize, f: F) -> Result<Vec<R>>
where
    T: Send + 'static,
    F: Fn(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R>> + Send + 'static,
    R: Send + 'static,
{
    let results: Vec<Result<R>> = stream::iter(items)
        .map(|item| {
            let f = &f;
            async move { f(item).await }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    let mut out = Vec::with_capacity(results.len());
    for r in results {
        out.push(r?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_map_concurrent_basic() {
        let items = vec![1, 2, 3, 4, 5];
        let results = map_concurrent(items, 3, |x| async move { Ok(x * 2) })
            .await
            .expect("should succeed");
        assert_eq!(results, vec![2, 4, 6, 8, 10]);
    }

    #[tokio::test]
    async fn test_map_concurrent_respects_concurrency() {
        let counter = Arc::new(AtomicU32::new(0));
        let max_concurrent = Arc::new(AtomicU32::new(0));
        let max_concurrent_clone = max_concurrent.clone();
        let items: Vec<u32> = (0..10).collect();

        let results = map_concurrent(items, 2, move |x| {
            let counter = counter.clone();
            let max_concurrent = max_concurrent_clone.clone();
            async move {
                let current = counter.fetch_add(1, Ordering::SeqCst) + 1;
                max_concurrent.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                counter.fetch_sub(1, Ordering::SeqCst);
                Ok::<_, anyhow::Error>(x)
            }
        })
        .await
        .expect("should succeed");

        assert_eq!(results.len(), 10);
        assert!(max_concurrent.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn test_map_concurrent_error() {
        let items = vec![1, 2, 3];
        let result = map_concurrent(items, 2, |x| async move {
            if x == 2 {
                Err(anyhow::anyhow!("fail on 2"))
            } else {
                Ok(x)
            }
        })
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_map_concurrent_empty() {
        let items: Vec<i32> = vec![];
        let results = map_concurrent(items, 4, |x| async move { Ok(x) })
            .await
            .expect("should succeed");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_map_concurrent_single() {
        let items = vec![42];
        let results = map_concurrent(items, 1, |x| async move { Ok(x + 1) })
            .await
            .expect("should succeed");
        assert_eq!(results, vec![43]);
    }
}

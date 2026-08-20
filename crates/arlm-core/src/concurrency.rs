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


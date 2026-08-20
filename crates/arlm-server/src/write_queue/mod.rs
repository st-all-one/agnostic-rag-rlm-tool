use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use arlm_storage::Storage;
use tokio::sync::mpsc;

/// Statistics for the write queue.
#[derive(Debug, Clone, Default)]
pub struct WriteQueueStats {
    pub pending: u64,
    pub flushed: u64,
    pub failed: u64,
}

/// Operations that can be queued for batch writing.
#[derive(Debug)]
#[allow(dead_code)]
pub enum WriteOp {
    /// Insert a chunk.
    InsertChunk {
        buffer_id: i64,
        content: String,
        file_path: String,
        start_line: i32,
        end_line: i32,
    },
    /// Insert a summary.
    InsertSummary {
        buffer_id: i64,
        content: String,
        scope: String,
        source_chunk_ids: String,
        source_hash: String,
        confidence: f64,
    },
}

/// Batched write queue for SQLite operations.
///
/// Collects write operations and flushes them periodically to avoid
/// contention on the single SQLite writer.
#[derive(Clone)]
pub struct WriteQueue {
    sender: Arc<mpsc::UnboundedSender<WriteOp>>,
    stats: Arc<WriteQueueStatsInner>,
}

struct WriteQueueStatsInner {
    pending: AtomicU64,
    flushed: AtomicU64,
    failed: AtomicU64,
}

impl WriteQueue {
    /// Create a new write queue.
    ///
    /// Spawns a background task that drains the queue.
    pub fn new(storage: Storage, flush_interval: Duration, max_batch_size: usize) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();

        let stats = Arc::new(WriteQueueStatsInner {
            pending: AtomicU64::new(0),
            flushed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        });

        let stats_clone = stats.clone();
        tokio::spawn(Self::drain_loop(
            storage,
            receiver,
            flush_interval,
            max_batch_size,
            stats_clone,
        ));

        Self {
            sender: Arc::new(sender),
            stats,
        }
    }

    /// Enqueue a write operation.
    pub fn enqueue(&self, op: WriteOp) {
        self.stats.pending.fetch_add(1, Ordering::Relaxed);
        let _ = self.sender.send(op);
    }

    /// Get write queue statistics.
    pub fn stats(&self) -> WriteQueueStats {
        WriteQueueStats {
            pending: self.stats.pending.load(Ordering::Relaxed),
            flushed: self.stats.flushed.load(Ordering::Relaxed),
            failed: self.stats.failed.load(Ordering::Relaxed),
        }
    }

    async fn drain_loop(
        storage: Storage,
        mut receiver: mpsc::UnboundedReceiver<WriteOp>,
        flush_interval: Duration,
        max_batch_size: usize,
        stats: Arc<WriteQueueStatsInner>,
    ) {
        let mut buffer = Vec::with_capacity(max_batch_size);
        let mut interval = tokio::time::interval(flush_interval);

        loop {
            tokio::select! {
                op = receiver.recv() => {
                    match op {
                        Some(op) => {
                            stats.pending.fetch_sub(1, Ordering::Relaxed);
                            buffer.push(op);
                            if buffer.len() >= max_batch_size {
                                match Self::flush(&storage, &mut buffer).await {
                                    Ok(()) => {
                                        stats.flushed.fetch_add(1, Ordering::Relaxed);
                                    }
                                    Err(e) => {
                                        stats.failed.fetch_add(1, Ordering::Relaxed);
                                        tracing::error!("write queue flush error: {e}");
                                    }
                                }
                            }
                        }
                        None => break, // Channel closed
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        match Self::flush(&storage, &mut buffer).await {
                            Ok(()) => {
                                stats.flushed.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                stats.failed.fetch_add(1, Ordering::Relaxed);
                                tracing::error!("write queue flush error: {e}");
                            }
                        }
                    }
                }
            }
        }

        // Final flush
        if !buffer.is_empty() {
            if let Err(e) = Self::flush(&storage, &mut buffer).await {
                stats.failed.fetch_add(1, Ordering::Relaxed);
                tracing::error!("write queue final flush error: {e}");
            }
        }
    }

    async fn flush(storage: &Storage, buffer: &mut Vec<WriteOp>) -> Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }

        let conn = storage.conn();
        let conn = conn.lock();

        let tx = conn.unchecked_transaction()?;

        for op in buffer.drain(..) {
            match op {
                WriteOp::InsertChunk {
                    buffer_id,
                    content,
                    file_path,
                    start_line,
                    end_line,
                } => {
                    tx.execute(
                        "INSERT INTO chunks (buffer_id, content, file_path, start_line, end_line) VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![buffer_id, content, file_path, start_line, end_line],
                    )?;
                }
                WriteOp::InsertSummary {
                    buffer_id,
                    content,
                    scope,
                    source_chunk_ids,
                    source_hash,
                    confidence,
                } => {
                    tx.execute(
                        "INSERT INTO summaries (buffer_id, content, scope, source_chunk_ids, source_hash, confidence) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![buffer_id, content, scope, source_chunk_ids, source_hash, confidence],
                    )?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }
}

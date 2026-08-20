use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use arlm_storage::Storage;
use tokio::sync::mpsc;

use crate::timing::Timer;

/// Statistics for the write queue.
#[derive(Debug, Clone, Default)]
pub struct WriteQueueStats {
    pub pending: u64,
    pub flushed: u64,
    pub failed: u64,
}

/// Operations that can be queued for batch writing.
#[derive(Debug)]
pub enum WriteOp {
    /// Insert a chunk plus its text and FTS row.
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
/// contention on the single SQLite writer. Safe to use with the pooled
/// storage backend (server deployment).
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
    /// Spawns a background task that drains the queue on the blocking pool.
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
        if self.sender.send(op).is_err() {
            self.stats.failed.fetch_add(1, Ordering::Relaxed);
            tracing::error!("write queue channel closed");
        }
    }

    /// Get write queue statistics.
    #[must_use]
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
                                Self::flush_and_track(&storage, &mut buffer, &stats).await;
                            }
                        }
                        None => break, // Channel closed
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        Self::flush_and_track(&storage, &mut buffer, &stats).await;
                    }
                }
            }
        }

        // Final flush
        if !buffer.is_empty() {
            Self::flush_and_track(&storage, &mut buffer, &stats).await;
        }
    }

    async fn flush_and_track(
        storage: &Storage,
        buffer: &mut Vec<WriteOp>,
        stats: &Arc<WriteQueueStatsInner>,
    ) {
        let timer = Timer::new("write_queue_flush");
        match Self::flush(storage, buffer).await {
            Ok(()) => {
                stats.flushed.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                stats.failed.fetch_add(1, Ordering::Relaxed);
                tracing::error!(error = %e, "write queue flush error");
            }
        }
        drop(timer);
    }

    async fn flush(storage: &Storage, buffer: &mut Vec<WriteOp>) -> Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }

        let conn = storage.connection()?;
        conn.execute(|conn| {
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
                            "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count) \
                             VALUES (?1, ?2, 0, 0, ?3, ?4, x'00', NULL, NULL, NULL)",
                            rusqlite::params![buffer_id, file_path, start_line, end_line],
                        )?;
                        let chunk_id = tx.last_insert_rowid();
                        tx.execute(
                            "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, ?2)",
                            rusqlite::params![chunk_id, content],
                        )?;
                        tx.execute(
                            "INSERT INTO chunks_fts(rowid, content) VALUES (?1, ?2)",
                            rusqlite::params![chunk_id, content],
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
        })?;

        Ok(())
    }
}

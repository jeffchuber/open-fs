use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, RwLock, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::error::VfsError;

/// Sync mode for a mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// No syncing - local only.
    None,
    /// Write-through: writes go to both local and remote synchronously.
    WriteThrough,
    /// Write-back: writes go to local first, then flushed to remote in background.
    WriteBack,
    /// Pull-mirror: read-only, pulls from remote on cache miss.
    PullMirror,
}

impl Default for SyncMode {
    fn default() -> Self {
        SyncMode::None
    }
}

/// A pending write operation for write-back mode.
#[derive(Debug, Clone)]
pub struct PendingWrite {
    /// The path being written.
    pub path: String,
    /// The content to write.
    pub content: Vec<u8>,
    /// When this write was queued (for future observability/metrics).
    #[allow(dead_code)]
    pub queued_at: Instant,
    /// Number of retry attempts.
    pub attempts: u32,
}

/// Configuration for the sync engine.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Sync mode to use.
    pub mode: SyncMode,
    /// Maximum pending writes before blocking.
    pub max_pending: usize,
    /// Flush interval for write-back mode.
    pub flush_interval: Duration,
    /// Maximum retry attempts for failed writes.
    pub max_retries: u32,
    /// Backoff duration between retries.
    pub retry_backoff: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            mode: SyncMode::None,
            max_pending: 1000,
            flush_interval: Duration::from_secs(5),
            max_retries: 3,
            retry_backoff: Duration::from_secs(1),
        }
    }
}

/// Sync engine statistics.
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    /// Number of successful syncs.
    pub synced: u64,
    /// Number of pending writes.
    pub pending: usize,
    /// Number of failed syncs.
    pub failed: u64,
    /// Number of retries.
    pub retries: u64,
    /// Last sync time.
    pub last_sync: Option<Instant>,
}

/// Write operation to be processed by the sync engine.
#[derive(Debug)]
pub enum WriteOp {
    /// Write content to a path.
    Write { path: String, content: Vec<u8> },
    /// Delete a path.
    Delete { path: String },
    /// Append content to a path.
    Append { path: String, content: Vec<u8> },
}

/// Sync engine for managing write-back operations.
pub struct SyncEngine {
    config: SyncConfig,
    /// Queue of pending write operations.
    pending_writes: Arc<RwLock<VecDeque<PendingWrite>>>,
    /// Stats tracking.
    stats: Arc<RwLock<SyncStats>>,
    /// Channel for submitting writes.
    write_tx: Option<mpsc::Sender<WriteOp>>,
    /// Flush task handle.
    flush_handle: Mutex<Option<JoinHandle<()>>>,
    /// Flag to signal shutdown.
    shutdown: Arc<RwLock<bool>>,
}

impl SyncEngine {
    /// Create a new sync engine.
    pub fn new(config: SyncConfig) -> Self {
        SyncEngine {
            config,
            pending_writes: Arc::new(RwLock::new(VecDeque::new())),
            stats: Arc::new(RwLock::new(SyncStats::default())),
            write_tx: None,
            flush_handle: Mutex::new(None),
            shutdown: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the background flush task for write-back mode.
    pub async fn start<F, Fut>(&mut self, flush_fn: F)
    where
        F: Fn(String, Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), VfsError>> + Send,
    {
        if self.config.mode != SyncMode::WriteBack {
            return;
        }

        let (tx, mut rx) = mpsc::channel::<WriteOp>(self.config.max_pending);
        self.write_tx = Some(tx);

        let pending = Arc::clone(&self.pending_writes);
        let stats = Arc::clone(&self.stats);
        let shutdown = Arc::clone(&self.shutdown);
        let config = self.config.clone();
        let flush_fn = Arc::new(flush_fn);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.flush_interval);

            loop {
                tokio::select! {
                    // Receive new write operations
                    Some(op) = rx.recv() => {
                        match op {
                            WriteOp::Write { path, content } => {
                                let mut pending_guard = pending.write().await;
                                pending_guard.push_back(PendingWrite {
                                    path,
                                    content,
                                    queued_at: Instant::now(),
                                    attempts: 0,
                                });
                                let mut stats_guard = stats.write().await;
                                stats_guard.pending = pending_guard.len();
                            }
                            WriteOp::Delete { path } => {
                                // For deletes, we remove from pending and add a delete marker
                                let mut pending_guard = pending.write().await;
                                pending_guard.retain(|p| p.path != path);
                                // In a full implementation, we'd track deletes separately
                            }
                            WriteOp::Append { path, content } => {
                                // For appends, merge with existing pending write
                                let mut pending_guard = pending.write().await;
                                if let Some(existing) = pending_guard.iter_mut().find(|p| p.path == path) {
                                    existing.content.extend(content);
                                } else {
                                    pending_guard.push_back(PendingWrite {
                                        path,
                                        content,
                                        queued_at: Instant::now(),
                                        attempts: 0,
                                    });
                                }
                                let mut stats_guard = stats.write().await;
                                stats_guard.pending = pending_guard.len();
                            }
                        }
                    }

                    // Periodic flush
                    _ = interval.tick() => {
                        if *shutdown.read().await {
                            break;
                        }

                        // Flush pending writes
                        let writes_to_flush: Vec<PendingWrite> = {
                            let mut pending_guard = pending.write().await;
                            let writes: Vec<_> = pending_guard.drain(..).collect();
                            writes
                        };

                        for mut write in writes_to_flush {
                            let result = flush_fn(write.path.clone(), write.content.clone()).await;

                            let mut stats_guard = stats.write().await;
                            match result {
                                Ok(()) => {
                                    stats_guard.synced += 1;
                                    stats_guard.last_sync = Some(Instant::now());
                                    debug!("Synced: {}", write.path);
                                }
                                Err(e) => {
                                    write.attempts += 1;
                                    if write.attempts < config.max_retries {
                                        // Re-queue for retry
                                        let mut pending_guard = pending.write().await;
                                        pending_guard.push_back(write);
                                        stats_guard.pending = pending_guard.len();
                                        stats_guard.retries += 1;
                                        warn!("Sync failed, will retry: {}", e);
                                    } else {
                                        stats_guard.failed += 1;
                                        error!("Sync failed after {} attempts: {}", config.max_retries, e);
                                    }
                                }
                            }
                        }

                        let pending_guard = pending.read().await;
                        let mut stats_guard = stats.write().await;
                        stats_guard.pending = pending_guard.len();
                    }
                }
            }

            // Final flush on shutdown
            info!("Sync engine shutting down, flushing remaining writes");
            let writes_to_flush: Vec<PendingWrite> = {
                let mut pending_guard = pending.write().await;
                pending_guard.drain(..).collect()
            };

            for write in writes_to_flush {
                if let Err(e) = flush_fn(write.path.clone(), write.content).await {
                    error!("Failed to flush {} on shutdown: {}", write.path, e);
                }
            }
        });

        *self.flush_handle.lock().await = Some(handle);
    }

    /// Queue a write operation (for write-back mode).
    pub async fn queue_write(&self, path: String, content: Vec<u8>) -> Result<(), VfsError> {
        if let Some(tx) = &self.write_tx {
            tx.send(WriteOp::Write { path, content })
                .await
                .map_err(|_| VfsError::Config("Sync channel closed".to_string()))?;
            Ok(())
        } else {
            Err(VfsError::Config("Sync engine not started".to_string()))
        }
    }

    /// Queue a delete operation.
    pub async fn queue_delete(&self, path: String) -> Result<(), VfsError> {
        if let Some(tx) = &self.write_tx {
            tx.send(WriteOp::Delete { path })
                .await
                .map_err(|_| VfsError::Config("Sync channel closed".to_string()))?;
            Ok(())
        } else {
            Err(VfsError::Config("Sync engine not started".to_string()))
        }
    }

    /// Queue an append operation.
    pub async fn queue_append(&self, path: String, content: Vec<u8>) -> Result<(), VfsError> {
        if let Some(tx) = &self.write_tx {
            tx.send(WriteOp::Append { path, content })
                .await
                .map_err(|_| VfsError::Config("Sync channel closed".to_string()))?;
            Ok(())
        } else {
            Err(VfsError::Config("Sync engine not started".to_string()))
        }
    }

    /// Get sync statistics.
    pub async fn stats(&self) -> SyncStats {
        self.stats.read().await.clone()
    }

    /// Get number of pending writes.
    pub async fn pending_count(&self) -> usize {
        self.pending_writes.read().await.len()
    }

    /// Shutdown the sync engine.
    pub async fn shutdown(&self) {
        *self.shutdown.write().await = true;

        // Wait for flush task to complete
        let handle = self.flush_handle.lock().await.take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }

    /// Get the sync mode.
    pub fn mode(&self) -> SyncMode {
        self.config.mode
    }

    /// Check if write-back mode is active.
    pub fn is_write_back(&self) -> bool {
        self.config.mode == SyncMode::WriteBack
    }

    /// Check if write-through mode is active.
    pub fn is_write_through(&self) -> bool {
        self.config.mode == SyncMode::WriteThrough
    }
}

impl Drop for SyncEngine {
    fn drop(&mut self) {
        // Note: For proper cleanup, call shutdown() before dropping
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert_eq!(config.mode, SyncMode::None);
        assert_eq!(config.max_pending, 1000);
    }

    #[tokio::test]
    async fn test_sync_engine_write_back() {
        let config = SyncConfig {
            mode: SyncMode::WriteBack,
            flush_interval: Duration::from_millis(50),
            ..Default::default()
        };

        let mut engine = SyncEngine::new(config);

        // Track writes
        let write_count = Arc::new(AtomicU32::new(0));
        let write_count_clone = Arc::clone(&write_count);

        engine.start(move |_path, _content| {
            let count = Arc::clone(&write_count_clone);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }).await;

        // Queue some writes
        engine.queue_write("/a.txt".to_string(), b"aaa".to_vec()).await.unwrap();
        engine.queue_write("/b.txt".to_string(), b"bbb".to_vec()).await.unwrap();

        // Wait for flush
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(write_count.load(Ordering::SeqCst), 2);

        engine.shutdown().await;
    }

    #[tokio::test]
    async fn test_sync_stats() {
        let config = SyncConfig {
            mode: SyncMode::WriteBack,
            flush_interval: Duration::from_millis(50),
            ..Default::default()
        };

        let mut engine = SyncEngine::new(config);

        engine.start(|_path, _content| async { Ok(()) }).await;

        engine.queue_write("/test.txt".to_string(), b"test".to_vec()).await.unwrap();

        // Wait for flush
        tokio::time::sleep(Duration::from_millis(100)).await;

        let stats = engine.stats().await;
        assert_eq!(stats.synced, 1);
        assert!(stats.last_sync.is_some());

        engine.shutdown().await;
    }
}

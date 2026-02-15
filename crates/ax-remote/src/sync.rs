use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ax_config::BackoffStrategy;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use ax_core::VfsError;
use crate::wal::{WalOpType, WriteAheadLog};

/// Sync mode for a mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum SyncMode {
    /// No syncing - local only.
    #[default]
    None,
    /// Write-through: writes go to both local and remote synchronously.
    WriteThrough,
    /// Write-back: writes go to local first, then flushed to remote in background.
    WriteBack,
    /// Pull-mirror: read-only, pulls from remote on cache miss.
    PullMirror,
}


/// A pending write operation for write-back mode.
#[derive(Debug, Clone)]
pub struct PendingWrite {
    /// The path being written.
    pub path: String,
    /// The content to write.
    pub content: Vec<u8>,
    /// Monotonic operation id for ordering/tombstones.
    pub op_id: u64,
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
    /// Base backoff duration between retries.
    pub retry_backoff: Duration,
    /// Backoff strategy for retries.
    pub backoff_strategy: BackoffStrategy,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            mode: SyncMode::None,
            max_pending: 1000,
            flush_interval: Duration::from_secs(5),
            max_retries: 3,
            retry_backoff: Duration::from_secs(1),
            backoff_strategy: BackoffStrategy::Exponential,
        }
    }
}

/// Compute the backoff duration for a given retry attempt.
pub fn compute_backoff(base: Duration, attempt: u32, strategy: BackoffStrategy) -> Duration {
    match strategy {
        BackoffStrategy::Fixed => base,
        BackoffStrategy::Linear => base * (attempt + 1),
        BackoffStrategy::Exponential => base * 2u32.saturating_pow(attempt),
        _ => base * 2u32.saturating_pow(attempt), // default to exponential
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


/// Sync engine for managing write-back operations.
pub struct SyncEngine {
    config: SyncConfig,
    /// Queue of pending write operations.
    pending_writes: Arc<RwLock<VecDeque<PendingWrite>>>,
    /// Tombstones for deletes (path -> op_id).
    tombstones: Arc<RwLock<HashMap<String, u64>>>,
    /// In-flight flushes (paths currently being written).
    in_flight: Arc<Mutex<HashSet<String>>>,
    /// Notifies waiters when in-flight set changes.
    in_flight_notify: Arc<Notify>,
    /// Monotonic op id generator.
    op_seq: Arc<AtomicU64>,
    /// Stats tracking.
    stats: Arc<RwLock<SyncStats>>,
    /// Flush task handle.
    flush_handle: Mutex<Option<JoinHandle<()>>>,
    /// Outbox drain task handle.
    outbox_handle: Mutex<Option<JoinHandle<()>>>,
    /// Flag to signal shutdown.
    shutdown: Arc<RwLock<bool>>,
    /// Optional WAL for crash-safe writes.
    wal: Option<Arc<WriteAheadLog>>,
}

impl SyncEngine {
    /// Create a new sync engine.
    pub fn new(config: SyncConfig) -> Self {
        SyncEngine {
            config,
            pending_writes: Arc::new(RwLock::new(VecDeque::new())),
            tombstones: Arc::new(RwLock::new(HashMap::new())),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
            in_flight_notify: Arc::new(Notify::new()),
            op_seq: Arc::new(AtomicU64::new(0)),
            stats: Arc::new(RwLock::new(SyncStats::default())),
            flush_handle: Mutex::new(None),
            outbox_handle: Mutex::new(None),
            shutdown: Arc::new(RwLock::new(false)),
            wal: None,
        }
    }

    /// Create a new sync engine with WAL-backed durability.
    pub fn with_wal(config: SyncConfig, wal: Arc<WriteAheadLog>) -> Self {
        SyncEngine {
            config,
            pending_writes: Arc::new(RwLock::new(VecDeque::new())),
            tombstones: Arc::new(RwLock::new(HashMap::new())),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
            in_flight_notify: Arc::new(Notify::new()),
            op_seq: Arc::new(AtomicU64::new(0)),
            stats: Arc::new(RwLock::new(SyncStats::default())),
            flush_handle: Mutex::new(None),
            outbox_handle: Mutex::new(None),
            shutdown: Arc::new(RwLock::new(false)),
            wal: Some(wal),
        }
    }

    /// Get a reference to the WAL (if configured).
    pub fn wal(&self) -> Option<&Arc<WriteAheadLog>> {
        self.wal.as_ref()
    }

    /// Start the background flush task for write-back mode.
    pub async fn start<F, Fut>(&self, flush_fn: F)
    where
        F: Fn(String, Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), VfsError>> + Send,
    {
        if self.config.mode != SyncMode::WriteBack {
            return;
        }

        let mut handle_guard = self.flush_handle.lock().await;
        if handle_guard.is_some() {
            return;
        }

        let pending = Arc::clone(&self.pending_writes);
        let tombstones = Arc::clone(&self.tombstones);
        let stats = Arc::clone(&self.stats);
        let shutdown = Arc::clone(&self.shutdown);
        let config = self.config.clone();
        let in_flight = Arc::clone(&self.in_flight);
        let in_flight_notify = Arc::clone(&self.in_flight_notify);
        let flush_fn = Arc::new(flush_fn);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.flush_interval);

            loop {
                interval.tick().await;

                if *shutdown.read().await {
                    break;
                }

                flush_pending(
                    &pending,
                    &tombstones,
                    &stats,
                    &config,
                    &flush_fn,
                    &in_flight,
                    &in_flight_notify,
                )
                .await;
            }

            // Final flush on shutdown
            info!("Sync engine shutting down, flushing remaining writes");
            flush_pending(
                &pending,
                &tombstones,
                &stats,
                &config,
                &flush_fn,
                &in_flight,
                &in_flight_notify,
            )
            .await;
        });

        *handle_guard = Some(handle);
    }

    /// Queue a write operation (for write-back mode).
    pub async fn queue_write(&self, path: String, content: Vec<u8>) -> Result<(), VfsError> {
        // Log to WAL for crash safety
        if let Some(wal) = &self.wal {
            let wal_id = wal
                .log_write(WalOpType::Write, &path, Some(&content), "")
                .map_err(|e| VfsError::Config(format!("WAL log failed: {}", e)))?;
            wal.mark_applied(wal_id)
                .map_err(|e| VfsError::Config(format!("WAL mark_applied failed: {}", e)))?;

            wal.enqueue_outbox(WalOpType::Write, &path, Some(&content), "")
                .map_err(|e| VfsError::Config(format!("Outbox enqueue failed: {}", e)))?;
        }

        self.ensure_started().await?;

        let op_id = self.op_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let mut pending_guard = self.pending_writes.write().await;
        if pending_guard.len() >= self.config.max_pending {
            return Err(VfsError::Config("Sync queue full".to_string()));
        }
        pending_guard.push_back(PendingWrite {
            path,
            content,
            op_id,

            attempts: 0,
        });
        let pending_len = pending_guard.len();
        drop(pending_guard);
        let mut stats_guard = self.stats.write().await;
        stats_guard.pending = pending_len;
        Ok(())
    }

    /// Queue a delete operation.
    pub async fn queue_delete(&self, path: String) -> Result<(), VfsError> {
        if let Some(wal) = &self.wal {
            let wal_id = wal
                .log_write(WalOpType::Delete, &path, None, "")
                .map_err(|e| VfsError::Config(format!("WAL log failed: {}", e)))?;
            wal.mark_applied(wal_id)
                .map_err(|e| VfsError::Config(format!("WAL mark_applied failed: {}", e)))?;

            wal.enqueue_outbox(WalOpType::Delete, &path, None, "")
                .map_err(|e| VfsError::Config(format!("Outbox enqueue failed: {}", e)))?;
        }

        self.ensure_started().await?;

        let op_id = self.op_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let mut tomb_guard = self.tombstones.write().await;
        tomb_guard.insert(path.clone(), op_id);
        drop(tomb_guard);

        let mut pending_guard = self.pending_writes.write().await;
        pending_guard.retain(|p| p.path != path);
        let pending_len = pending_guard.len();
        drop(pending_guard);
        let mut stats_guard = self.stats.write().await;
        stats_guard.pending = pending_len;
        Ok(())
    }

    /// Queue an append operation.
    pub async fn queue_append(&self, path: String, content: Vec<u8>) -> Result<(), VfsError> {
        if let Some(wal) = &self.wal {
            let wal_id = wal
                .log_write(WalOpType::Append, &path, Some(&content), "")
                .map_err(|e| VfsError::Config(format!("WAL log failed: {}", e)))?;
            wal.mark_applied(wal_id)
                .map_err(|e| VfsError::Config(format!("WAL mark_applied failed: {}", e)))?;

            wal.enqueue_outbox(WalOpType::Append, &path, Some(&content), "")
                .map_err(|e| VfsError::Config(format!("Outbox enqueue failed: {}", e)))?;
        }

        self.ensure_started().await?;

        let op_id = self.op_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let mut pending_guard = self.pending_writes.write().await;
        if let Some(existing) = pending_guard.iter_mut().find(|p| p.path == path) {
            existing.content.extend(content);
            existing.op_id = op_id;
        } else {
            if pending_guard.len() >= self.config.max_pending {
                return Err(VfsError::Config("Sync queue full".to_string()));
            }
            pending_guard.push_back(PendingWrite {
                path,
                content,
                op_id,
    
                attempts: 0,
            });
        }
        let pending_len = pending_guard.len();
        drop(pending_guard);
        let mut stats_guard = self.stats.write().await;
        stats_guard.pending = pending_len;
        Ok(())
    }

    async fn ensure_started(&self) -> Result<(), VfsError> {
        if self.config.mode != SyncMode::WriteBack {
            return Err(VfsError::Config("Sync engine not in write-back mode".to_string()));
        }
        let handle_guard = self.flush_handle.lock().await;
        if handle_guard.is_some() {
            Ok(())
        } else {
            Err(VfsError::Config("Sync engine not started".to_string()))
        }
    }

    /// Acquire a per-path lock shared with the flush loop.
    pub async fn acquire_path_lock(&self, path: &str) {
        acquire_path_lock(path, &self.in_flight, &self.in_flight_notify).await;
    }

    /// Release a per-path lock shared with the flush loop.
    pub async fn release_path_lock(&self, path: &str) {
        release_path_lock(path, &self.in_flight, &self.in_flight_notify).await;
    }

    /// Snapshot pending write paths with their content sizes.
    pub async fn pending_paths(&self) -> Vec<(String, usize)> {
        let pending_guard = self.pending_writes.read().await;
        pending_guard
            .iter()
            .map(|p| (p.path.clone(), p.content.len()))
            .collect()
    }

    /// Check if a path has a pending write.
    pub async fn pending_contains(&self, path: &str) -> bool {
        let pending_guard = self.pending_writes.read().await;
        pending_guard.iter().any(|p| p.path == path)
    }

    /// Start a background task that drains the outbox with retry.
    pub async fn start_outbox_drain<F, Fut>(&self, sync_fn: F)
    where
        F: Fn(WalOpType, String, Option<Vec<u8>>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), VfsError>> + Send,
    {
        let wal = match &self.wal {
            Some(w) => Arc::clone(w),
            None => return,
        };

        let shutdown = Arc::clone(&self.shutdown);
        let stats = Arc::clone(&self.stats);
        let sync_fn = Arc::new(sync_fn);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));

            loop {
                interval.tick().await;

                if *shutdown.read().await {
                    break;
                }

                let entries = match wal.fetch_ready_outbox(10) {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("Failed to fetch outbox: {}", e);
                        continue;
                    }
                };

                for entry in entries {
                    if let Err(e) = wal.mark_processing(entry.id) {
                        warn!("Failed to mark processing: {}", e);
                        continue;
                    }

                    match sync_fn(entry.op_type, entry.path.clone(), entry.content).await {
                        Ok(()) => {
                            if let Err(e) = wal.complete_outbox(entry.id) {
                                warn!("Failed to complete outbox entry {}: {}", entry.id, e);
                            }
                            let mut s = stats.write().await;
                            s.synced += 1;
                            s.last_sync = Some(Instant::now());
                            debug!("Outbox synced: {}", entry.path);
                        }
                        Err(e) => {
                            if let Err(fail_err) = wal.fail_outbox(entry.id, &e.to_string()) {
                                warn!("Failed to record outbox failure: {}", fail_err);
                            }
                            let mut s = stats.write().await;
                            s.retries += 1;
                            warn!("Outbox sync failed for {}: {}", entry.path, e);
                        }
                    }
                }
            }

            info!("Outbox drain task shutting down");
        });

        *self.outbox_handle.lock().await = Some(handle);
    }

    /// Replay unapplied WAL entries for crash recovery.
    pub async fn recover_from_wal<F, Fut>(&self, apply_fn: F) -> Result<usize, VfsError>
    where
        F: Fn(WalOpType, String, Option<Vec<u8>>) -> Fut,
        Fut: std::future::Future<Output = Result<(), VfsError>>,
    {
        let wal = match &self.wal {
            Some(w) => w,
            None => return Ok(0),
        };

        let unapplied = wal
            .get_unapplied()
            .map_err(|e| VfsError::Config(format!("WAL recovery failed: {}", e)))?;

        let count = unapplied.len();
        if count == 0 {
            return Ok(0);
        }

        info!("Recovering {} unapplied WAL entries", count);

        for entry in unapplied {
            match apply_fn(entry.op_type, entry.path.clone(), entry.content).await {
                Ok(()) => {
                    wal.mark_applied(entry.id)
                        .map_err(|e| VfsError::Config(format!("WAL mark_applied failed: {}", e)))?;
                }
                Err(e) => {
                    error!("Failed to recover WAL entry {}: {}", entry.path, e);
                }
            }
        }

        Ok(count)
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

        let handle = self.flush_handle.lock().await.take();
        if let Some(h) = handle {
            if let Err(e) = h.await {
                tracing::warn!("Flush task failed during shutdown: {}", e);
            }
        }

        let outbox_handle = self.outbox_handle.lock().await.take();
        if let Some(h) = outbox_handle {
            if let Err(e) = h.await {
                tracing::warn!("Outbox drain task failed during shutdown: {}", e);
            }
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

async fn acquire_path_lock(
    path: &str,
    in_flight: &Arc<Mutex<HashSet<String>>>,
    notify: &Arc<Notify>,
) {
    loop {
        {
            let mut guard = in_flight.lock().await;
            if !guard.contains(path) {
                guard.insert(path.to_string());
                return;
            }
        }
        notify.notified().await;
    }
}

async fn release_path_lock(
    path: &str,
    in_flight: &Arc<Mutex<HashSet<String>>>,
    notify: &Arc<Notify>,
) {
    let mut guard = in_flight.lock().await;
    guard.remove(path);
    drop(guard);
    notify.notify_waiters();
}

async fn flush_pending<F, Fut>(
    pending: &Arc<RwLock<VecDeque<PendingWrite>>>,
    tombstones: &Arc<RwLock<HashMap<String, u64>>>,
    stats: &Arc<RwLock<SyncStats>>,
    config: &SyncConfig,
    flush_fn: &Arc<F>,
    in_flight: &Arc<Mutex<HashSet<String>>>,
    in_flight_notify: &Arc<Notify>,
) where
    F: Fn(String, Vec<u8>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<(), VfsError>> + Send,
{
    let writes_to_flush: Vec<PendingWrite> = {
        let mut pending_guard = pending.write().await;
        pending_guard.drain(..).collect()
    };

    {
        let mut stats_guard = stats.write().await;
        stats_guard.pending = 0;
    }

    for mut write in writes_to_flush {
        acquire_path_lock(&write.path, in_flight, in_flight_notify).await;

        let skip = {
            let tomb_guard = tombstones.read().await;
            tomb_guard
                .get(&write.path)
                .map(|&tomb_id| write.op_id <= tomb_id)
                .unwrap_or(false)
        };
        if skip {
            release_path_lock(&write.path, in_flight, in_flight_notify).await;
            continue;
        }

        if write.attempts > 0 {
            let backoff = compute_backoff(
                config.retry_backoff,
                write.attempts - 1,
                config.backoff_strategy,
            );
            tokio::time::sleep(backoff).await;
        }

        let result = flush_fn(write.path.clone(), write.content.clone()).await;

        release_path_lock(&write.path, in_flight, in_flight_notify).await;

        match result {
            Ok(()) => {
                let mut stats_guard = stats.write().await;
                stats_guard.synced += 1;
                stats_guard.last_sync = Some(Instant::now());
                debug!("Synced: {}", write.path);
            }
            Err(e) => {
                let is_transient = is_transient_error(&e);
                write.attempts += 1;

                if is_transient && write.attempts < config.max_retries {
                    let mut pending_guard = pending.write().await;
                    pending_guard.push_back(write);
                    let pending_len = pending_guard.len();
                    drop(pending_guard);

                    let mut stats_guard = stats.write().await;
                    stats_guard.pending = pending_len;
                    stats_guard.retries += 1;
                    warn!("Sync failed (transient), will retry: {}", e);
                } else {
                    let mut stats_guard = stats.write().await;
                    stats_guard.failed += 1;
                    if is_transient {
                        error!(
                            "Sync failed after {} attempts: {}",
                            config.max_retries, e
                        );
                    } else {
                        error!("Sync failed (non-transient, not retrying): {}", e);
                    }
                }
            }
        }
    }

    let pending_len = pending.read().await.len();
    let mut stats_guard = stats.write().await;
    stats_guard.pending = pending_len;
}

/// Check if a VfsError wraps a transient backend error.
fn is_transient_error(err: &VfsError) -> bool {
    match err {
        VfsError::Backend(boxed) => {
            if let Some(backend_err) = boxed.downcast_ref::<ax_core::BackendError>() {
                backend_err.is_transient()
            } else {
                false
            }
        }
        VfsError::Io(io_err) => matches!(
            io_err.kind(),
            std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::Interrupted
        ),
        _ => false,
    }
}

impl Drop for SyncEngine {
    fn drop(&mut self) {
        // Use try_write/try_lock to avoid futures::executor::block_on deadlocks
        // when dropping inside a tokio runtime (e.g. single-threaded test runtime).
        if let Ok(mut shutdown) = self.shutdown.try_write() {
            *shutdown = true;
        }

        // Abort background tasks if running.
        if let Ok(mut handle_guard) = self.flush_handle.try_lock() {
            if let Some(handle) = handle_guard.take() {
                handle.abort();
            }
        }

        if let Ok(mut outbox_guard) = self.outbox_handle.try_lock() {
            if let Some(handle) = outbox_guard.take() {
                handle.abort();
            }
        }

        // Best-effort check for pending writes.
        if let Ok(pending) = self.pending_writes.try_read() {
            if !pending.is_empty() {
                warn!(
                    "SyncEngine dropped with {} pending writes. Call shutdown() before dropping to ensure all writes are flushed.",
                    pending.len()
                );
            }
        }
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
        assert_eq!(config.backoff_strategy, BackoffStrategy::Exponential);
    }

    #[tokio::test]
    async fn test_sync_engine_write_back() {
        let config = SyncConfig {
            mode: SyncMode::WriteBack,
            flush_interval: Duration::from_millis(50),
            ..Default::default()
        };

        let engine = SyncEngine::new(config);

        let write_count = Arc::new(AtomicU32::new(0));
        let write_count_clone = Arc::clone(&write_count);

        engine.start(move |_path, _content| {
            let count = Arc::clone(&write_count_clone);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }).await;

        engine.queue_write("/a.txt".to_string(), b"aaa".to_vec()).await.unwrap();
        engine.queue_write("/b.txt".to_string(), b"bbb".to_vec()).await.unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(write_count.load(Ordering::SeqCst), 2);

        engine.shutdown().await;
    }

    #[test]
    fn test_compute_backoff_fixed() {
        let base = Duration::from_secs(1);
        assert_eq!(compute_backoff(base, 0, BackoffStrategy::Fixed), Duration::from_secs(1));
        assert_eq!(compute_backoff(base, 1, BackoffStrategy::Fixed), Duration::from_secs(1));
    }

    #[test]
    fn test_compute_backoff_exponential() {
        let base = Duration::from_secs(1);
        assert_eq!(compute_backoff(base, 0, BackoffStrategy::Exponential), Duration::from_secs(1));
        assert_eq!(compute_backoff(base, 1, BackoffStrategy::Exponential), Duration::from_secs(2));
        assert_eq!(compute_backoff(base, 2, BackoffStrategy::Exponential), Duration::from_secs(4));
    }
}

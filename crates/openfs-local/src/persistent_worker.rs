//! Persistent index worker backed by SQLite work queue.
//!
//! Drop-in replacement for `IndexWorker` that survives process crashes,
//! provides retry with exponential backoff, and has a dead letter queue.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::incremental::IncrementalIndexer;
use crate::pipeline::PipelineConfig;
use crate::work_queue::{QueueEventType, QueueItem, WorkQueue, WorkQueueConfig};
use openfs_core::{Backend, VfsError};

/// Events that can be sent to the persistent worker.
#[derive(Debug)]
pub enum PersistentEvent {
    FileChanged { path: String },
    FileDeleted { path: String },
    Shutdown,
}

/// Thread-safe wrapper around WorkQueue (rusqlite::Connection is !Send).
struct SyncQueue {
    inner: Mutex<WorkQueue>,
}

impl SyncQueue {
    fn new(queue: WorkQueue) -> Self {
        SyncQueue {
            inner: Mutex::new(queue),
        }
    }

    fn enqueue(&self, path: &str, event_type: QueueEventType) -> Result<(), String> {
        self.inner.lock().unwrap().enqueue(path, event_type)
    }

    fn fetch_ready(&self, batch_size: usize) -> Result<Vec<QueueItem>, String> {
        self.inner.lock().unwrap().fetch_ready(batch_size)
    }

    fn complete(&self, id: i64) -> Result<(), String> {
        self.inner.lock().unwrap().complete(id)
    }

    fn fail(&self, id: i64, error: &str) -> Result<(), String> {
        self.inner.lock().unwrap().fail(id, error)
    }

    fn recover_stuck(&self) -> Result<usize, String> {
        self.inner.lock().unwrap().recover_stuck()
    }
}

// SAFETY: We protect all access to the non-Send Connection via a Mutex,
// ensuring only one thread accesses it at a time.
unsafe impl Send for SyncQueue {}
unsafe impl Sync for SyncQueue {}

/// A persistent background worker for indexing, backed by SQLite.
pub struct PersistentIndexWorker {
    sender: mpsc::Sender<PersistentEvent>,
}

impl PersistentIndexWorker {
    /// Spawn a new persistent index worker.
    ///
    /// The work queue DB is stored at `queue_path`. On restart, any pending or
    /// stuck items from a previous run will be automatically recovered.
    pub fn spawn<B>(
        backend: Arc<B>,
        config: PipelineConfig,
        state_path: PathBuf,
        queue_path: PathBuf,
        queue_config: WorkQueueConfig,
        buffer_size: usize,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), VfsError>
    where
        B: Backend + 'static,
    {
        let queue = WorkQueue::open(&queue_path, queue_config)
            .map_err(|e| VfsError::Indexing(format!("Failed to open work queue: {}", e)))?;
        let queue = Arc::new(SyncQueue::new(queue));

        // Recover any items stuck in "processing" from a previous crash
        match queue.recover_stuck() {
            Ok(n) if n > 0 => info!("Recovered {} stuck queue items from previous run", n),
            Ok(_) => {}
            Err(e) => warn!("Failed to recover stuck items: {}", e),
        }

        let (tx, rx) = mpsc::channel(buffer_size);
        let indexer = IncrementalIndexer::new(config, &state_path)?;

        let handle = tokio::spawn(Self::run(rx, indexer, backend, queue));

        Ok((PersistentIndexWorker { sender: tx }, handle))
    }

    /// Send an event to the worker.
    pub async fn send(&self, event: PersistentEvent) -> Result<(), VfsError> {
        self.sender
            .send(event)
            .await
            .map_err(|e| VfsError::Indexing(format!("Failed to send event: {}", e)))
    }

    /// Request shutdown.
    pub async fn shutdown(&self) -> Result<(), VfsError> {
        self.send(PersistentEvent::Shutdown).await
    }

    async fn run<B>(
        mut rx: mpsc::Receiver<PersistentEvent>,
        mut indexer: IncrementalIndexer,
        backend: Arc<B>,
        queue: Arc<SyncQueue>,
    ) where
        B: Backend + 'static,
    {
        info!("Persistent index worker started");

        let mut process_interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    match event {
                        PersistentEvent::FileChanged { path } => {
                            if let Err(e) = queue.enqueue(&path, QueueEventType::Changed) {
                                error!("Failed to enqueue change for {}: {}", path, e);
                            }
                        }
                        PersistentEvent::FileDeleted { path } => {
                            if let Err(e) = queue.enqueue(&path, QueueEventType::Deleted) {
                                error!("Failed to enqueue delete for {}: {}", path, e);
                            }
                        }
                        PersistentEvent::Shutdown => {
                            info!("Persistent worker shutting down");
                            Self::process_batch(&queue, &mut indexer, &backend).await;
                            if let Err(e) = indexer.persist_state() {
                                error!("Failed to persist index state: {}", e);
                            }
                            break;
                        }
                    }
                }
                _ = process_interval.tick() => {
                    Self::process_batch(&queue, &mut indexer, &backend).await;
                }
            }
        }

        if let Err(e) = indexer.persist_state() {
            error!("Failed to persist index state: {}", e);
        }

        info!("Persistent index worker stopped");
    }

    async fn process_batch<B>(
        queue: &Arc<SyncQueue>,
        indexer: &mut IncrementalIndexer,
        backend: &Arc<B>,
    ) where
        B: Backend + 'static,
    {
        let items = match queue.fetch_ready(32) {
            Ok(items) => items,
            Err(e) => {
                error!("Failed to fetch ready items: {}", e);
                return;
            }
        };

        for item in items {
            let deleted = item.event_type == QueueEventType::Deleted;
            debug!(
                "Processing: {} ({})",
                item.path,
                if deleted { "delete" } else { "index" }
            );

            match indexer
                .handle_change(backend.as_ref(), &item.path, deleted)
                .await
            {
                Ok(()) => {
                    if let Err(e) = queue.complete(item.id) {
                        error!("Failed to complete item {}: {}", item.id, e);
                    }
                }
                Err(e) => {
                    warn!("Failed to process {}: {}", item.path, e);
                    if let Err(qe) = queue.fail(item.id, &e.to_string()) {
                        error!("Failed to mark item {} as failed: {}", item.id, qe);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfs_remote::MemoryBackend;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_persistent_worker_basic() {
        let tmp = TempDir::new().unwrap();
        let state_path = tmp.path().join(".openfs-index-state.json");
        let queue_path = tmp.path().join("queue.db");

        let backend = Arc::new(MemoryBackend::new());
        backend.write("/test.txt", b"Hello world").await.unwrap();

        let (worker, handle) = PersistentIndexWorker::spawn(
            backend,
            PipelineConfig::default(),
            state_path,
            queue_path,
            WorkQueueConfig {
                debounce_secs: 0,
                ..Default::default()
            },
            16,
        )
        .unwrap();

        worker
            .send(PersistentEvent::FileChanged {
                path: "/test.txt".to_string(),
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(1500)).await;

        worker.shutdown().await.unwrap();
        handle.await.unwrap();
    }
}

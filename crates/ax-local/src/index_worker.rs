use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use ax_core::{Backend, VfsError};
use crate::incremental::IncrementalIndexer;
use crate::pipeline::PipelineConfig;

/// Events that can be sent to the index worker.
#[derive(Debug)]
pub enum IndexEvent {
    /// A file was created or modified and needs (re-)indexing.
    FileChanged { path: String },
    /// A file was deleted and should be removed from the index.
    FileDeleted { path: String },
    /// Request the worker to persist its state and shut down.
    Shutdown,
}

/// A background worker that processes indexing events.
///
/// Receives `IndexEvent`s via an mpsc channel and processes them through
/// an `IncrementalIndexer`.
///
/// **Deprecated**: Use [`PersistentIndexWorker`](crate::PersistentIndexWorker) instead,
/// which uses a SQLite-backed persistent queue that survives crashes and
/// supports retry with exponential backoff.
#[deprecated(note = "Use PersistentIndexWorker for crash-resilient indexing with retry")]
pub struct IndexWorker {
    sender: mpsc::Sender<IndexEvent>,
}

#[allow(deprecated)]
impl IndexWorker {
    /// Spawn a new index worker.
    ///
    /// Returns the worker handle (for sending events) and a join handle for the background task.
    pub fn spawn<B>(
        backend: Arc<B>,
        config: PipelineConfig,
        state_path: PathBuf,
        buffer_size: usize,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), VfsError>
    where
        B: Backend + 'static,
    {
        let (tx, rx) = mpsc::channel(buffer_size);

        let indexer = IncrementalIndexer::new(config, &state_path)?;

        let handle = tokio::spawn(Self::run(rx, indexer, backend));

        Ok((IndexWorker { sender: tx }, handle))
    }

    /// Send an event to the worker.
    pub async fn send(&self, event: IndexEvent) -> Result<(), VfsError> {
        self.sender.send(event).await.map_err(|e| {
            VfsError::Indexing(format!("Failed to send index event: {}", e))
        })
    }

    /// Request the worker to shut down gracefully.
    pub async fn shutdown(&self) -> Result<(), VfsError> {
        self.send(IndexEvent::Shutdown).await
    }

    /// The main worker loop.
    async fn run<B>(
        mut rx: mpsc::Receiver<IndexEvent>,
        mut indexer: IncrementalIndexer,
        backend: Arc<B>,
    ) where
        B: Backend + 'static,
    {
        info!("Index worker started");

        while let Some(event) = rx.recv().await {
            match event {
                IndexEvent::FileChanged { path } => {
                    debug!("Index worker: file changed: {}", path);
                    match indexer.handle_change(backend.as_ref(), &path, false).await {
                        Ok(()) => {
                            debug!("Indexed: {}", path);
                        }
                        Err(e) => {
                            warn!("Failed to index {}: {}", path, e);
                        }
                    }
                }
                IndexEvent::FileDeleted { path } => {
                    debug!("Index worker: file deleted: {}", path);
                    match indexer.handle_change(backend.as_ref(), &path, true).await {
                        Ok(()) => {
                            debug!("Removed from index: {}", path);
                        }
                        Err(e) => {
                            warn!("Failed to remove {} from index: {}", path, e);
                        }
                    }
                }
                IndexEvent::Shutdown => {
                    info!("Index worker shutting down");
                    if let Err(e) = indexer.persist_state() {
                        error!("Failed to persist index state on shutdown: {}", e);
                    }
                    break;
                }
            }
        }

        // Final persist
        if let Err(e) = indexer.persist_state() {
            error!("Failed to persist index state: {}", e);
        }

        info!("Index worker stopped");
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use ax_remote::MemoryBackend;
    use crate::index_state::IndexState;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_worker_index_file() {
        let tmp = TempDir::new().unwrap();
        let state_path = tmp.path().join(".ax-index-state.json");

        let backend = Arc::new(MemoryBackend::new());
        backend
            .write("/test.txt", b"Hello world")
            .await
            .unwrap();

        let (worker, handle) =
            IndexWorker::spawn(backend, PipelineConfig::default(), state_path, 16).unwrap();

        worker
            .send(IndexEvent::FileChanged {
                path: "/test.txt".to_string(),
            })
            .await
            .unwrap();

        // Give worker time to process
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        worker.shutdown().await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_worker_shutdown_persists_state() {
        let tmp = TempDir::new().unwrap();
        let state_path = tmp.path().join(".ax-index-state.json");

        let backend = Arc::new(MemoryBackend::new());
        backend
            .write("/test.txt", b"Hello world")
            .await
            .unwrap();

        let (worker, handle) =
            IndexWorker::spawn(backend, PipelineConfig::default(), state_path.clone(), 16)
                .unwrap();

        worker
            .send(IndexEvent::FileChanged {
                path: "/test.txt".to_string(),
            })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        worker.shutdown().await.unwrap();
        handle.await.unwrap();

        // State file should exist
        assert!(state_path.exists());
        let state = IndexState::load(&state_path).unwrap();
        assert_eq!(state.file_count(), 1);
    }
}

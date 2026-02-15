pub mod chunkers;
pub mod content_hash;
pub mod embedders;
pub mod extractors;
pub mod sparse;
pub mod types;
pub mod pipeline;
pub mod search;
pub mod incremental;
pub mod index_state;
pub mod watcher;
pub mod work_queue;
pub mod persistent_worker;

// Re-exports
pub use chunkers::{Chunker, ChunkerConfig};
pub use embedders::{Embedder, EmbedderConfig};
pub use extractors::{create_extractors, TextExtractor};
pub use content_hash::{content_hash, content_hash_streaming};
pub use sparse::SparseEncoder;
pub use types::*;
pub use pipeline::{IndexingPipeline, PipelineConfig};
pub use search::{SearchConfig, SearchEngine, SearchMode};
pub use incremental::{IncrementalIndexer, IncrementalResult};
pub use index_state::{IndexState, FileInfo, ReconcileAction, ReconcileResult};
pub use watcher::{ChangeKind, FileChange, WatchEngine};
pub use work_queue::{WorkQueue, WorkQueueConfig, QueueEventType, QueueItemStatus, QueueItem};
pub use persistent_worker::{PersistentIndexWorker, PersistentEvent};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexingError {
    #[error("Chunking error: {0}")]
    ChunkingError(String),

    #[error("Embedding error: {0}")]
    EmbeddingError(String),

    #[error("Extraction error: {0}")]
    ExtractionError(String),

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Unsupported file type: {0}")]
    UnsupportedFileType(String),
}

#[cfg(feature = "embedder-ollama")]
impl From<reqwest::Error> for IndexingError {
    fn from(e: reqwest::Error) -> Self {
        IndexingError::HttpError(e.to_string())
    }
}

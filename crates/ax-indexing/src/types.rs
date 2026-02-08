use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A chunk of text extracted from a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier for this chunk.
    pub id: String,
    /// The source file path.
    pub source_path: String,
    /// The text content of the chunk.
    pub content: String,
    /// Start byte offset in the original file.
    pub start_offset: usize,
    /// End byte offset in the original file.
    pub end_offset: usize,
    /// Start line number (1-indexed).
    pub start_line: usize,
    /// End line number (1-indexed).
    pub end_line: usize,
    /// Chunk index within the file (0-indexed).
    pub chunk_index: usize,
    /// Total number of chunks from this file.
    pub total_chunks: usize,
    /// Additional metadata.
    pub metadata: HashMap<String, String>,
}

impl Chunk {
    pub fn new(
        source_path: String,
        content: String,
        start_offset: usize,
        end_offset: usize,
        start_line: usize,
        end_line: usize,
        chunk_index: usize,
        total_chunks: usize,
    ) -> Self {
        Chunk {
            id: uuid::Uuid::new_v4().to_string(),
            source_path,
            content,
            start_offset,
            end_offset,
            start_line,
            end_line,
            chunk_index,
            total_chunks,
            metadata: HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// An embedded chunk with its vector representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedChunk {
    /// The original chunk.
    pub chunk: Chunk,
    /// Dense embedding vector.
    pub embedding: Vec<f32>,
}

/// A sparse vector representation (for BM25/sparse search).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparseVector {
    /// Token indices.
    pub indices: Vec<u32>,
    /// Token weights/scores.
    pub values: Vec<f32>,
}

/// Result of embedding a batch of texts.
#[derive(Debug, Clone)]
pub struct EmbeddingResult {
    /// The embeddings, one per input text.
    pub embeddings: Vec<Vec<f32>>,
    /// Number of tokens processed (if available).
    pub token_count: Option<usize>,
}

/// Search result from the index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The chunk that matched.
    pub chunk: Chunk,
    /// Similarity score (higher is better).
    pub score: f32,
    /// Optional dense score component.
    pub dense_score: Option<f32>,
    /// Optional sparse score component.
    pub sparse_score: Option<f32>,
}

/// Pipeline event for indexing.
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    /// File was created.
    Created { path: String },
    /// File was modified.
    Modified { path: String },
    /// File was deleted.
    Deleted { path: String },
}

impl PipelineEvent {
    pub fn path(&self) -> &str {
        match self {
            PipelineEvent::Created { path } => path,
            PipelineEvent::Modified { path } => path,
            PipelineEvent::Deleted { path } => path,
        }
    }
}

/// Result of processing a pipeline event.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// Path that was processed.
    pub path: String,
    /// Number of chunks created/updated.
    pub chunks_created: usize,
    /// Number of chunks deleted.
    pub chunks_deleted: usize,
    /// Processing time in milliseconds.
    pub duration_ms: u64,
}

/// Result of bulk indexing.
#[derive(Debug, Clone)]
pub struct BulkIndexResult {
    /// Number of files processed.
    pub files_processed: usize,
    /// Number of files skipped (errors or filtered).
    pub files_skipped: usize,
    /// Total chunks created.
    pub total_chunks: usize,
    /// Total processing time in milliseconds.
    pub duration_ms: u64,
    /// Errors encountered (path -> error message).
    pub errors: Vec<(String, String)>,
}

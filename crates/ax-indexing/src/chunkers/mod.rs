mod fixed;
mod recursive;
mod semantic;

pub use fixed::FixedChunker;
pub use recursive::RecursiveChunker;
pub use semantic::SemanticChunker;

#[cfg(feature = "chunker-ast")]
mod ast;
#[cfg(feature = "chunker-ast")]
pub use ast::AstChunker;

use crate::{Chunk, IndexingError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Configuration for a chunker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkerConfig {
    /// Target chunk size in characters.
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Overlap between chunks in characters.
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    /// Minimum chunk size (chunks smaller than this are merged).
    #[serde(default = "default_min_chunk_size")]
    pub min_chunk_size: usize,
}

fn default_chunk_size() -> usize {
    512
}

fn default_chunk_overlap() -> usize {
    64
}

fn default_min_chunk_size() -> usize {
    50
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        ChunkerConfig {
            chunk_size: default_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
            min_chunk_size: default_min_chunk_size(),
        }
    }
}

/// Trait for text chunking implementations.
#[async_trait]
pub trait Chunker: Send + Sync {
    /// Split text into chunks.
    async fn chunk(&self, text: &str, source_path: &str) -> Result<Vec<Chunk>, IndexingError>;

    /// Get the name of this chunker.
    fn name(&self) -> &'static str;
}

/// Create a chunker based on strategy name.
pub fn create_chunker(
    strategy: &str,
    config: ChunkerConfig,
) -> Result<Box<dyn Chunker>, IndexingError> {
    match strategy.to_lowercase().as_str() {
        "fixed" => Ok(Box::new(FixedChunker::new(config))),
        "recursive" => Ok(Box::new(RecursiveChunker::new(config))),
        "semantic" => Ok(Box::new(SemanticChunker::new(config))),
        #[cfg(feature = "chunker-ast")]
        "ast" => Ok(Box::new(AstChunker::new(config))),
        _ => Err(IndexingError::ChunkingError(format!(
            "Unknown chunking strategy: {}",
            strategy
        ))),
    }
}

/// Helper to count lines up to a byte offset.
pub fn count_lines_to_offset(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}

mod stub;

pub use stub::StubEmbedder;

#[cfg(feature = "embedder-ollama")]
mod ollama;
#[cfg(feature = "embedder-ollama")]
pub use ollama::OllamaEmbedder;

#[cfg(feature = "embedder-openai")]
mod openai;
#[cfg(feature = "embedder-openai")]
pub use openai::OpenAiEmbedder;

use std::sync::Arc;

use crate::{EmbeddingResult, IndexingError};
use async_trait::async_trait;
use openfs_core::{BackendError, TextEmbedder};
use serde::{Deserialize, Serialize};

/// Configuration for an embedder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedderConfig {
    /// The model name to use.
    pub model: String,
    /// The expected embedding dimensions.
    pub dimensions: usize,
    /// API endpoint (for HTTP-based embedders).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// API key (for authenticated APIs).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Maximum batch size for embedding requests.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    32
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        EmbedderConfig {
            model: "nomic-embed-text".to_string(),
            dimensions: 768,
            endpoint: None,
            api_key: None,
            batch_size: default_batch_size(),
        }
    }
}

/// Trait for text embedding implementations.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a batch of texts.
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingResult, IndexingError>;

    /// Embed a single text.
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>, IndexingError> {
        let result = self.embed(&[text]).await?;
        result
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| IndexingError::EmbeddingError("No embedding returned".to_string()))
    }

    /// Get the embedding dimensions.
    fn dimensions(&self) -> usize;

    /// Get the model name.
    fn model(&self) -> &str;

    /// Get the embedder name.
    fn name(&self) -> &'static str;
}

/// Adapter that wraps an [`Embedder`] to implement [`TextEmbedder`] from openfs-core.
///
/// This bridges the full indexing embedder (which supports batching, dimensions,
/// model info) to the minimal single-text interface used by write-through paths.
pub struct EmbedderAdapter {
    inner: Arc<dyn Embedder>,
}

impl EmbedderAdapter {
    /// Wrap an embedder as a [`TextEmbedder`].
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        EmbedderAdapter { inner: embedder }
    }
}

#[async_trait]
impl TextEmbedder for EmbedderAdapter {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, BackendError> {
        self.inner
            .embed_one(text)
            .await
            .map_err(|e| BackendError::Other(format!("Embedding failed: {}", e)))
    }
}

/// Create an embedder based on provider name.
pub fn create_embedder(
    provider: &str,
    config: EmbedderConfig,
) -> Result<Box<dyn Embedder>, IndexingError> {
    match provider.to_lowercase().as_str() {
        "stub" | "none" => Ok(Box::new(StubEmbedder::new(config.dimensions))),
        #[cfg(feature = "embedder-ollama")]
        "ollama" => Ok(Box::new(OllamaEmbedder::new(config))),
        #[cfg(feature = "embedder-openai")]
        "openai" | "openai-compatible" => Ok(Box::new(OpenAiEmbedder::new(config))),
        _ => Err(IndexingError::EmbeddingError(format!(
            "Unknown embedding provider: {}",
            provider
        ))),
    }
}

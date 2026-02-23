use super::Embedder;
use crate::{EmbeddingResult, IndexingError};
use async_trait::async_trait;

/// Stub embedder that returns zero vectors.
/// Useful for testing and when embeddings are not needed.
pub struct StubEmbedder {
    dimensions: usize,
}

impl StubEmbedder {
    pub fn new(dimensions: usize) -> Self {
        StubEmbedder { dimensions }
    }
}

#[async_trait]
impl Embedder for StubEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingResult, IndexingError> {
        let embeddings = texts
            .iter()
            .map(|_| vec![0.0f32; self.dimensions])
            .collect();

        Ok(EmbeddingResult {
            embeddings,
            token_count: None,
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model(&self) -> &str {
        "stub"
    }

    fn name(&self) -> &'static str {
        "stub"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_embedder() {
        let embedder = StubEmbedder::new(384);

        let result = embedder.embed(&["hello", "world"]).await.unwrap();

        assert_eq!(result.embeddings.len(), 2);
        assert_eq!(result.embeddings[0].len(), 384);
        assert!(result.embeddings[0].iter().all(|&x| x == 0.0));
    }
}

#![cfg(feature = "embedder-ollama")]

use super::{Embedder, EmbedderConfig};
use crate::{EmbeddingResult, IndexingError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Ollama embedding client.
pub struct OllamaEmbedder {
    config: EmbedderConfig,
    client: reqwest::Client,
    endpoint: String,
}

#[derive(Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbedder {
    pub fn new(config: EmbedderConfig) -> Self {
        let endpoint = config
            .endpoint
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        OllamaEmbedder {
            config,
            client: reqwest::Client::new(),
            endpoint,
        }
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingResult, IndexingError> {
        if texts.is_empty() {
            return Ok(EmbeddingResult {
                embeddings: vec![],
                token_count: None,
            });
        }

        let mut all_embeddings = Vec::new();

        // Process in batches
        for batch in texts.chunks(self.config.batch_size) {
            let request = OllamaEmbedRequest {
                model: self.config.model.clone(),
                input: batch.iter().map(|s| s.to_string()).collect(),
            };

            let response = self
                .client
                .post(format!("{}/api/embed", self.endpoint))
                .json(&request)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(IndexingError::EmbeddingError(format!(
                    "Ollama API error: {} - {}",
                    status, body
                )));
            }

            let result: OllamaEmbedResponse = response.json().await?;
            all_embeddings.extend(result.embeddings);
        }

        Ok(EmbeddingResult {
            embeddings: all_embeddings,
            token_count: None,
        })
    }

    fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn name(&self) -> &'static str {
        "ollama"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires running Ollama instance
    async fn test_ollama_embedder() {
        let config = EmbedderConfig {
            model: "nomic-embed-text".to_string(),
            dimensions: 768,
            endpoint: Some("http://localhost:11434".to_string()),
            ..Default::default()
        };

        let embedder = OllamaEmbedder::new(config);
        let result = embedder.embed(&["hello world"]).await.unwrap();

        assert_eq!(result.embeddings.len(), 1);
        assert!(!result.embeddings[0].is_empty());
    }
}

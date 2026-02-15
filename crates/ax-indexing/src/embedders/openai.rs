#![cfg(feature = "embedder-openai")]

use super::{Embedder, EmbedderConfig};
use crate::{EmbeddingResult, IndexingError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// OpenAI-compatible embedding client.
/// Works with OpenAI and other compatible APIs.
pub struct OpenAiEmbedder {
    config: EmbedderConfig,
    client: reqwest::Client,
    endpoint: String,
}

#[derive(Serialize)]
struct OpenAiEmbedRequest {
    model: String,
    input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Deserialize)]
struct OpenAiEmbedResponse {
    data: Vec<EmbeddingData>,
    usage: Option<UsageInfo>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Deserialize)]
struct UsageInfo {
    total_tokens: Option<usize>,
}

impl OpenAiEmbedder {
    pub fn new(config: EmbedderConfig) -> Self {
        let endpoint = config
            .endpoint
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(ref api_key) = config.api_key {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", api_key).parse().unwrap(),
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");

        OpenAiEmbedder {
            config,
            client,
            endpoint,
        }
    }

    /// Create with API key from environment variable.
    pub fn from_env(config: EmbedderConfig) -> Self {
        let mut config = config;
        if config.api_key.is_none() {
            config.api_key = std::env::var("OPENAI_API_KEY").ok();
        }
        Self::new(config)
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingResult, IndexingError> {
        if texts.is_empty() {
            return Ok(EmbeddingResult {
                embeddings: vec![],
                token_count: None,
            });
        }

        let mut all_embeddings = Vec::with_capacity(texts.len());
        let mut total_tokens = 0usize;

        // Process in batches
        for batch in texts.chunks(self.config.batch_size) {
            let request = OpenAiEmbedRequest {
                model: self.config.model.clone(),
                input: batch.iter().map(|s| s.to_string()).collect(),
                dimensions: Some(self.config.dimensions),
            };

            let response = self
                .client
                .post(format!("{}/embeddings", self.endpoint))
                .json(&request)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(IndexingError::EmbeddingError(format!(
                    "OpenAI API error: {} - {}",
                    status, body
                )));
            }

            let mut result: OpenAiEmbedResponse = response.json().await?;

            // Sort by index to ensure correct order
            result.data.sort_by_key(|d| d.index);

            for data in result.data {
                all_embeddings.push(data.embedding);
            }

            if let Some(usage) = result.usage {
                if let Some(tokens) = usage.total_tokens {
                    total_tokens += tokens;
                }
            }
        }

        Ok(EmbeddingResult {
            embeddings: all_embeddings,
            token_count: if total_tokens > 0 {
                Some(total_tokens)
            } else {
                None
            },
        })
    }

    fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn name(&self) -> &'static str {
        "openai"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires OpenAI API key
    async fn test_openai_embedder() {
        let config = EmbedderConfig {
            model: "text-embedding-3-small".to_string(),
            dimensions: 1536,
            ..Default::default()
        };

        let embedder = OpenAiEmbedder::from_env(config);
        let result = embedder.embed(&["hello world"]).await.unwrap();

        assert_eq!(result.embeddings.len(), 1);
        assert!(!result.embeddings[0].is_empty());
    }
}

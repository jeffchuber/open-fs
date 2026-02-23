use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::{
    chunkers, embedders, extractors, BulkIndexResult, Chunker, ChunkerConfig, EmbeddedChunk,
    Embedder, EmbedderConfig, PipelineResult, SparseEncoder, SparseVector, TextExtractor,
};
use openfs_core::{Backend, ChromaStore, SparseEmbedding, VfsError};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for the indexing pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Chunker strategy (fixed, recursive, semantic).
    pub chunker_strategy: String,
    /// Chunker configuration.
    pub chunker: ChunkerConfig,
    /// Embedder provider (stub, ollama, openai).
    pub embedder_provider: String,
    /// Embedder configuration.
    pub embedder: EmbedderConfig,
    /// Whether to compute sparse (BM25) vectors.
    pub enable_sparse: bool,
    /// Batch size for embedding operations.
    pub batch_size: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig {
            chunker_strategy: "fixed".to_string(),
            chunker: ChunkerConfig::default(),
            embedder_provider: "stub".to_string(),
            embedder: EmbedderConfig::default(),
            enable_sparse: true,
            batch_size: 32,
        }
    }
}

/// An indexing pipeline that coordinates text extraction, chunking, and embedding.
pub struct IndexingPipeline {
    config: PipelineConfig,
    chunker: Box<dyn Chunker>,
    embedder: Box<dyn Embedder>,
    extractor: extractors::PlainTextExtractor,
    sparse_encoder: Arc<RwLock<SparseEncoder>>,
    chroma: Option<Arc<dyn ChromaStore>>,
}

impl IndexingPipeline {
    /// Create a new indexing pipeline with the given configuration.
    pub fn new(config: PipelineConfig) -> Result<Self, VfsError> {
        let chunker = chunkers::create_chunker(&config.chunker_strategy, config.chunker.clone())
            .map_err(|e| VfsError::Config(format!("Failed to create chunker: {}", e)))?;
        let embedder =
            embedders::create_embedder(&config.embedder_provider, config.embedder.clone())
                .map_err(|e| VfsError::Config(format!("Failed to create embedder: {}", e)))?;
        let extractor = extractors::PlainTextExtractor::new();
        let sparse_encoder = Arc::new(RwLock::new(SparseEncoder::new()));

        Ok(IndexingPipeline {
            config,
            chunker,
            embedder,
            extractor,
            sparse_encoder,
            chroma: None,
        })
    }

    /// Connect a Chroma backend for vector storage.
    pub fn with_chroma(mut self, chroma: Arc<dyn ChromaStore>) -> Self {
        self.chroma = Some(chroma);
        self
    }

    /// Index a single file.
    pub async fn index_file(&self, path: &str, content: &[u8]) -> Result<PipelineResult, VfsError> {
        let start = Instant::now();

        // Extract text
        let text = self
            .extractor
            .extract(content, path)
            .await
            .map_err(|e| VfsError::Backend(Box::new(PipelineError(e.to_string()))))?;

        // Chunk the text
        let chunks = self
            .chunker
            .chunk(&text, path)
            .await
            .map_err(|e| VfsError::Backend(Box::new(PipelineError(e.to_string()))))?;

        debug!("Created {} chunks for {}", chunks.len(), path);

        // Embed chunks in batches
        let mut embedded_chunks = Vec::new();
        for chunk_batch in chunks.chunks(self.config.batch_size) {
            let texts: Vec<&str> = chunk_batch.iter().map(|c| c.content.as_str()).collect();
            let embeddings = self
                .embedder
                .embed(&texts)
                .await
                .map_err(|e| VfsError::Backend(Box::new(PipelineError(e.to_string()))))?;

            for (chunk, embedding) in chunk_batch.iter().zip(embeddings.embeddings) {
                embedded_chunks.push(EmbeddedChunk {
                    chunk: chunk.clone(),
                    embedding,
                });
            }
        }

        // Update sparse encoder and compute sparse vectors if enabled
        let mut sparse_vectors: Vec<Option<SparseVector>> = Vec::new();
        if self.config.enable_sparse {
            let mut encoder = self.sparse_encoder.write().await;
            for chunk in &chunks {
                encoder.update_idf(&chunk.content);
            }
            for chunk in &chunks {
                match encoder.encode(&chunk.content) {
                    Ok(sv) => sparse_vectors.push(Some(sv)),
                    Err(e) => {
                        warn!("Failed to encode sparse vector for chunk: {}", e);
                        sparse_vectors.push(None);
                    }
                }
            }
        }

        // Store in Chroma if configured
        if let Some(chroma) = &self.chroma {
            for (idx, embedded) in embedded_chunks.iter().enumerate() {
                let chunk = &embedded.chunk;
                let mut metadata: HashMap<String, serde_json::Value> = HashMap::new();
                metadata.insert(
                    "source_path".to_string(),
                    serde_json::json!(chunk.source_path),
                );
                metadata.insert(
                    "start_line".to_string(),
                    serde_json::json!(chunk.start_line),
                );
                metadata.insert("end_line".to_string(), serde_json::json!(chunk.end_line));
                metadata.insert(
                    "chunk_index".to_string(),
                    serde_json::json!(chunk.chunk_index),
                );
                metadata.insert(
                    "total_chunks".to_string(),
                    serde_json::json!(chunk.total_chunks),
                );

                // Create a unique ID for this chunk
                let chunk_path = format!("{}#chunk_{}", chunk.source_path, chunk.chunk_index);

                // Convert sparse vector to SparseEmbedding for Chroma
                let sparse_embedding =
                    sparse_vectors
                        .get(idx)
                        .and_then(|sv| sv.as_ref())
                        .map(|sv| SparseEmbedding {
                            indices: sv.indices.clone(),
                            values: sv.values.clone(),
                        });

                chroma
                    .upsert(
                        &chunk_path,
                        &chunk.content,
                        Some(embedded.embedding.clone()),
                        sparse_embedding,
                        Some(metadata),
                    )
                    .await
                    .map_err(|e| VfsError::Backend(Box::new(e)))?;
            }
            debug!(
                "Stored {} chunks in Chroma for {}",
                embedded_chunks.len(),
                path
            );
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(PipelineResult {
            path: path.to_string(),
            chunks_created: embedded_chunks.len(),
            chunks_deleted: 0,
            duration_ms,
        })
    }

    /// Index multiple files from a backend.
    pub async fn index_directory<B: Backend>(
        &self,
        backend: &B,
        dir_path: &str,
        recursive: bool,
    ) -> Result<BulkIndexResult, VfsError> {
        let start = Instant::now();
        let mut files_processed = 0;
        let mut files_skipped = 0;
        let mut total_chunks = 0;
        let mut errors = Vec::new();

        // Collect files to index
        let mut paths_to_index = Vec::new();
        self.collect_files(backend, dir_path, recursive, &mut paths_to_index)
            .await?;

        info!(
            "Found {} files to index in {}",
            paths_to_index.len(),
            dir_path
        );

        for path in paths_to_index {
            // Check if extractor supports this file type
            if !self.extractor.supports(&path) {
                debug!("Skipping unsupported file: {}", path);
                files_skipped += 1;
                continue;
            }

            match backend.read(&path).await {
                Ok(content) => match self.index_file(&path, &content).await {
                    Ok(result) => {
                        files_processed += 1;
                        total_chunks += result.chunks_created;
                    }
                    Err(e) => {
                        warn!("Failed to index {}: {}", path, e);
                        errors.push((path, e.to_string()));
                        files_skipped += 1;
                    }
                },
                Err(e) => {
                    warn!("Failed to read {}: {}", path, e);
                    errors.push((path, e.to_string()));
                    files_skipped += 1;
                }
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(BulkIndexResult {
            files_processed,
            files_skipped,
            total_chunks,
            duration_ms,
            errors,
        })
    }

    /// Recursively collect file paths from a directory.
    async fn collect_files<B: Backend>(
        &self,
        backend: &B,
        dir_path: &str,
        recursive: bool,
        paths: &mut Vec<String>,
    ) -> Result<(), VfsError> {
        let entries = backend
            .list(dir_path)
            .await
            .map_err(|e| VfsError::Backend(Box::new(e)))?;

        for entry in entries {
            if entry.is_dir {
                if recursive {
                    Box::pin(self.collect_files(backend, &entry.path, recursive, paths)).await?;
                }
            } else {
                paths.push(entry.path);
            }
        }

        Ok(())
    }

    /// Delete indexed content for a file.
    pub async fn delete_file(&self, path: &str) -> Result<(), VfsError> {
        if let Some(chroma) = &self.chroma {
            // Delete all chunks for this source_path using metadata filter
            let filter = serde_json::json!({"source_path": path});
            chroma
                .delete_by_metadata(filter)
                .await
                .map_err(|e| VfsError::Backend(Box::new(e)))?;
        }
        Ok(())
    }

    /// Get the sparse encoder for query encoding.
    pub fn sparse_encoder(&self) -> Arc<RwLock<SparseEncoder>> {
        Arc::clone(&self.sparse_encoder)
    }

    /// Get embedding dimension from the embedder.
    pub fn embedding_dimensions(&self) -> usize {
        self.embedder.dimensions()
    }

    /// Embed a query string.
    pub async fn embed_query(&self, query: &str) -> Result<Vec<f32>, VfsError> {
        self.embedder
            .embed_one(query)
            .await
            .map_err(|e| VfsError::Backend(Box::new(PipelineError(e.to_string()))))
    }

    /// Encode a query for sparse search.
    pub async fn encode_sparse_query(&self, query: &str) -> Result<SparseVector, VfsError> {
        let encoder = self.sparse_encoder.read().await;
        encoder
            .encode_query(query)
            .map_err(|e| VfsError::Backend(Box::new(PipelineError(e.to_string()))))
    }

    /// Persist SparseEncoder state to Chroma collection metadata.
    pub async fn persist_sparse_encoder(&self) -> Result<(), VfsError> {
        let chroma = match &self.chroma {
            Some(c) => c,
            None => return Ok(()),
        };

        let encoder = self.sparse_encoder.read().await;
        let json = encoder
            .to_json()
            .map_err(|e| VfsError::Backend(Box::new(PipelineError(e.to_string()))))?;

        let mut metadata = HashMap::new();
        metadata.insert("sparse_encoder_state".to_string(), serde_json::json!(json));

        chroma
            .set_collection_metadata(metadata)
            .await
            .map_err(|e| VfsError::Backend(Box::new(e)))?;

        debug!("Persisted SparseEncoder state to Chroma collection metadata");
        Ok(())
    }

    /// Load SparseEncoder state from Chroma collection metadata.
    pub async fn load_sparse_encoder(&self) -> Result<bool, VfsError> {
        let chroma = match &self.chroma {
            Some(c) => c,
            None => return Ok(false),
        };

        let metadata = chroma
            .get_collection_metadata()
            .await
            .map_err(|e| VfsError::Backend(Box::new(e)))?;

        if let Some(meta) = metadata {
            if let Some(serde_json::Value::String(json)) = meta.get("sparse_encoder_state") {
                match SparseEncoder::from_json(json) {
                    Ok(restored) => {
                        let mut encoder = self.sparse_encoder.write().await;
                        *encoder = restored;
                        info!("Restored SparseEncoder state from Chroma collection metadata");
                        return Ok(true);
                    }
                    Err(e) => {
                        warn!("Failed to restore SparseEncoder state: {}", e);
                    }
                }
            }
        }

        Ok(false)
    }
}

/// Wrapper error type for pipeline errors.
#[derive(Debug)]
struct PipelineError(String);

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PipelineError {}

#[cfg(test)]
mod tests {
    use super::*;
    use openfs_remote::MemoryBackend;

    #[tokio::test]
    async fn test_pipeline_index_file() {
        let config = PipelineConfig::default();
        let pipeline = IndexingPipeline::new(config).unwrap();

        let content = b"Hello, world! This is a test file with some content.";
        let result = pipeline.index_file("/test.txt", content).await.unwrap();

        assert_eq!(result.path, "/test.txt");
        assert!(result.chunks_created > 0);
    }

    #[tokio::test]
    async fn test_pipeline_index_directory() {
        let config = PipelineConfig::default();
        let pipeline = IndexingPipeline::new(config).unwrap();

        let backend = MemoryBackend::new();
        backend
            .write("/dir/file1.txt", b"Content of file 1")
            .await
            .unwrap();
        backend
            .write("/dir/file2.txt", b"Content of file 2")
            .await
            .unwrap();
        backend
            .write("/dir/sub/file3.txt", b"Content of file 3")
            .await
            .unwrap();

        let result = pipeline
            .index_directory(&backend, "/dir", true)
            .await
            .unwrap();

        assert_eq!(result.files_processed, 3);
        assert!(result.total_chunks >= 3);
    }

    #[tokio::test]
    async fn test_pipeline_embed_query() {
        let config = PipelineConfig::default();
        let pipeline = IndexingPipeline::new(config).unwrap();

        let embedding = pipeline.embed_query("test query").await.unwrap();
        assert!(!embedding.is_empty());
    }
}

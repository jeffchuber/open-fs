use std::collections::HashMap;
use std::sync::Arc;

use ax_core::{ChromaStore, QueryResult as ChromaQueryResult, SparseEmbedding, VfsError};
use crate::types::{SearchResult, Chunk};

use crate::pipeline::IndexingPipeline;

/// Search mode configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum SearchMode {
    /// Dense-only search using vector embeddings.
    Dense,
    /// Sparse-only search using BM25.
    Sparse,
    /// Hybrid search combining dense and sparse scores.
    #[default]
    Hybrid,
}


/// Configuration for search queries.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Search mode to use.
    pub mode: SearchMode,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Minimum score threshold (0.0 to 1.0).
    pub min_score: f32,
    /// Weight for dense scores in hybrid mode (0.0 to 1.0).
    pub dense_weight: f32,
    /// Weight for sparse scores in hybrid mode (0.0 to 1.0).
    pub sparse_weight: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            mode: SearchMode::Hybrid,
            limit: 10,
            min_score: 0.0,
            dense_weight: 0.7,
            sparse_weight: 0.3,
        }
    }
}

/// Search engine that queries Chroma for both dense and sparse search.
pub struct SearchEngine {
    pipeline: Arc<IndexingPipeline>,
    chroma: Option<Arc<dyn ChromaStore>>,
}

impl SearchEngine {
    /// Create a new search engine with the given pipeline.
    pub fn new(pipeline: Arc<IndexingPipeline>) -> Self {
        SearchEngine {
            pipeline,
            chroma: None,
        }
    }

    /// Set the Chroma backend for search.
    pub fn with_chroma(mut self, chroma: Arc<dyn ChromaStore>) -> Self {
        self.chroma = Some(chroma);
        self
    }

    /// Search for documents matching the query.
    pub async fn search(
        &self,
        query: &str,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, VfsError> {
        match config.mode {
            SearchMode::Dense => self.search_dense(query, config).await,
            SearchMode::Sparse => self.search_sparse(query, config).await,
            SearchMode::Hybrid => self.search_hybrid(query, config).await,
        }
    }

    /// Perform dense (embedding-based) search.
    async fn search_dense(
        &self,
        query: &str,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, VfsError> {
        let chroma = self.chroma.as_ref().ok_or_else(|| {
            VfsError::Config("Chroma backend required for dense search".to_string())
        })?;

        let query_embedding = self.pipeline.embed_query(query).await?;

        let results = chroma
            .query_by_embedding(query_embedding, config.limit)
            .await
            .map_err(|e| VfsError::Backend(Box::new(e)))?;

        let search_results = self.chroma_to_search_results(results, config);
        Ok(search_results)
    }

    /// Perform sparse (BM25) search via Chroma.
    async fn search_sparse(
        &self,
        query: &str,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, VfsError> {
        let chroma = self.chroma.as_ref().ok_or_else(|| {
            VfsError::Config("Chroma backend required for sparse search".to_string())
        })?;

        let query_vector = self.pipeline.encode_sparse_query(query).await?;
        let query_sparse = SparseEmbedding {
            indices: query_vector.indices,
            values: query_vector.values,
        };

        let results = chroma
            .query_by_sparse_embedding(&query_sparse, config.limit)
            .await
            .map_err(|e| VfsError::Backend(Box::new(e)))?;

        let search_results: Vec<SearchResult> = results
            .into_iter()
            .filter(|r| r.score > config.min_score)
            .map(|r| {
                let chunk = self.result_to_chunk(&r);
                SearchResult {
                    chunk,
                    score: r.score,
                    dense_score: None,
                    sparse_score: Some(r.score),
                }
            })
            .collect();

        Ok(search_results)
    }

    /// Perform hybrid search combining dense and sparse results from Chroma.
    async fn search_hybrid(
        &self,
        query: &str,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, VfsError> {
        let chroma = self.chroma.as_ref().ok_or_else(|| {
            VfsError::Config("Chroma backend required for hybrid search".to_string())
        })?;

        // Get dense results from Chroma
        let query_embedding = self.pipeline.embed_query(query).await?;
        let dense_results = chroma
            .query_by_embedding(query_embedding, config.limit * 2)
            .await
            .map_err(|e| VfsError::Backend(Box::new(e)))?;

        // Get sparse results from Chroma
        let query_vector = self.pipeline.encode_sparse_query(query).await?;
        let query_sparse = SparseEmbedding {
            indices: query_vector.indices,
            values: query_vector.values,
        };
        let sparse_results = chroma
            .query_by_sparse_embedding(&query_sparse, config.limit * 2)
            .await
            .map_err(|e| VfsError::Backend(Box::new(e)))?;

        // Build score maps
        let mut combined_scores: HashMap<String, (Option<Chunk>, f32, f32)> = HashMap::new();

        // Add dense scores
        for result in &dense_results {
            let chunk = self.result_to_chunk(result);
            let chunk_id = chunk.id.clone();
            combined_scores.insert(chunk_id, (Some(chunk), result.score, 0.0));
        }

        // Add sparse scores
        for result in &sparse_results {
            let chunk = self.result_to_chunk(result);
            let chunk_id = chunk.id.clone();
            combined_scores
                .entry(chunk_id)
                .and_modify(|(_, _, s)| *s = result.score)
                .or_insert((Some(chunk), 0.0, result.score));
        }

        // Calculate hybrid scores
        let mut results: Vec<SearchResult> = combined_scores
            .into_iter()
            .filter_map(|(_, (chunk_opt, dense_score, sparse_score))| {
                chunk_opt.map(|chunk| {
                    let score =
                        config.dense_weight * dense_score + config.sparse_weight * sparse_score;
                    SearchResult {
                        chunk,
                        score,
                        dense_score: if dense_score > 0.0 {
                            Some(dense_score)
                        } else {
                            None
                        },
                        sparse_score: if sparse_score > 0.0 {
                            Some(sparse_score)
                        } else {
                            None
                        },
                    }
                })
            })
            .filter(|r| r.score > config.min_score)
            .collect();

        // Sort by combined score
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Take top N
        results.truncate(config.limit);

        Ok(results)
    }

    /// Convert Chroma query results to search results.
    fn chroma_to_search_results(
        &self,
        results: Vec<ChromaQueryResult>,
        config: &SearchConfig,
    ) -> Vec<SearchResult> {
        results
            .into_iter()
            .filter(|r| r.score > config.min_score)
            .map(|r| {
                let chunk = self.result_to_chunk(&r);
                SearchResult {
                    chunk,
                    score: r.score,
                    dense_score: Some(r.score),
                    sparse_score: None,
                }
            })
            .collect()
    }

    /// Convert a Chroma query result to a Chunk.
    fn result_to_chunk(&self, result: &ChromaQueryResult) -> Chunk {
        let metadata = result.metadata.as_ref();

        let source_path = metadata
            .and_then(|m| m.get("source_path"))
            .and_then(|v| v.as_str())
            .unwrap_or(&result.id)
            .to_string();

        let start_line = metadata
            .and_then(|m| m.get("start_line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;

        let end_line = metadata
            .and_then(|m| m.get("end_line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;

        let chunk_index = metadata
            .and_then(|m| m.get("chunk_index"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let total_chunks = metadata
            .and_then(|m| m.get("total_chunks"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;

        Chunk {
            id: result.id.clone(),
            source_path,
            content: result.document.clone().unwrap_or_default(),
            start_offset: 0,
            end_offset: 0,
            start_line,
            end_line,
            chunk_index,
            total_chunks,
            metadata: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::PipelineConfig;

    #[tokio::test]
    async fn test_search_engine_requires_chroma() {
        let config = PipelineConfig::default();
        let pipeline = Arc::new(IndexingPipeline::new(config).unwrap());
        let engine = SearchEngine::new(pipeline.clone());

        // Without Chroma, dense/sparse/hybrid search should return an error
        let search_config = SearchConfig {
            mode: SearchMode::Dense,
            limit: 10,
            min_score: 0.0,
            ..Default::default()
        };
        assert!(engine.search("hello", &search_config).await.is_err());

        let search_config = SearchConfig {
            mode: SearchMode::Sparse,
            limit: 10,
            min_score: 0.0,
            ..Default::default()
        };
        assert!(engine.search("hello", &search_config).await.is_err());
    }

    #[tokio::test]
    async fn test_search_config_default() {
        let config = SearchConfig::default();
        assert_eq!(config.mode, SearchMode::Hybrid);
        assert_eq!(config.limit, 10);
        assert_eq!(config.dense_weight, 0.7);
        assert_eq!(config.sparse_weight, 0.3);
    }
}

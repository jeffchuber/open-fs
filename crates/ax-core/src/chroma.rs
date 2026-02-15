use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::BackendError;

/// Sparse vector representation for BM25/keyword search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparseEmbedding {
    pub indices: Vec<u32>,
    pub values: Vec<f32>,
}

/// Result from a Chroma query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub id: String,
    pub document: Option<String>,
    pub distance: f32,
    pub score: f32,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Trait for Chroma vector store operations.
///
/// Abstracts over different Chroma implementations:
/// - HTTP client (for remote Chroma server)
/// - Local (for in-process PersistentClient)
/// - Mock (for testing)
#[async_trait]
pub trait ChromaStore: Send + Sync + 'static {
    /// Store a document with optional dense and sparse embeddings.
    async fn upsert(
        &self,
        path: &str,
        content: &str,
        embedding: Option<Vec<f32>>,
        sparse_embedding: Option<SparseEmbedding>,
        metadata: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), BackendError>;

    /// Query by embedding vector.
    async fn query_by_embedding(
        &self,
        embedding: Vec<f32>,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError>;

    /// Query by sparse embedding (BM25/keyword search).
    async fn query_by_sparse_embedding(
        &self,
        query_sparse: &SparseEmbedding,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError>;

    /// Delete all documents matching a metadata filter.
    async fn delete_by_metadata(
        &self,
        filter: serde_json::Value,
    ) -> Result<usize, BackendError>;

    /// Set collection metadata (used for persisting SparseEncoder state).
    async fn set_collection_metadata(
        &self,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<(), BackendError>;

    /// Get collection metadata (used for loading SparseEncoder state).
    async fn get_collection_metadata(
        &self,
    ) -> Result<Option<HashMap<String, serde_json::Value>>, BackendError>;

    /// Get the collection name.
    fn collection_name(&self) -> &str;
}

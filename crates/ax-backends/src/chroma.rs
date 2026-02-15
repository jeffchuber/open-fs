use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::BackendError;
use crate::traits::{Backend, Entry};

/// Chroma vector database backend.
/// Uses Chroma's HTTP API for storing files and their embeddings.
pub struct ChromaBackend {
    client: Client,
    endpoint: String,
    collection_id: String,
    collection_name: String,
}

#[derive(Serialize)]
struct CreateCollectionRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<HashMap<String, String>>,
    get_or_create: bool,
}

#[derive(Deserialize)]
struct CollectionResponse {
    id: String,
    name: String,
}

/// Sparse vector representation for BM25/keyword search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparseEmbedding {
    pub indices: Vec<u32>,
    pub values: Vec<f32>,
}

#[derive(Serialize)]
struct AddDocumentsRequest {
    ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embeddings: Option<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadatas: Option<Vec<HashMap<String, serde_json::Value>>>,
}

#[derive(Serialize)]
struct QueryRequest {
    query_embeddings: Option<Vec<Vec<f32>>>,
    query_texts: Option<Vec<String>>,
    n_results: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    r#where: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct QueryResponse {
    ids: Vec<Vec<String>>,
    #[serde(default)]
    documents: Option<Vec<Vec<Option<String>>>>,
    #[serde(default)]
    metadatas: Option<Vec<Vec<Option<HashMap<String, serde_json::Value>>>>>,
    #[serde(default)]
    distances: Option<Vec<Vec<f32>>>,
}

#[derive(Serialize)]
struct GetDocumentsRequest {
    ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r#where: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct GetDocumentsResponse {
    ids: Vec<String>,
    #[serde(default)]
    documents: Option<Vec<Option<String>>>,
    #[serde(default)]
    metadatas: Option<Vec<Option<HashMap<String, serde_json::Value>>>>,
}

impl ChromaBackend {
    /// Create a new Chroma backend.
    pub async fn new(endpoint: &str, collection_name: &str) -> Result<Self, BackendError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| BackendError::Other(format!("Failed to build HTTP client: {}", e)))?;
        let endpoint = endpoint.trim_end_matches('/').to_string();

        // Create or get collection
        let request = CreateCollectionRequest {
            name: collection_name.to_string(),
            metadata: None,
            get_or_create: true,
        };

        let response = client
            .post(format!("{}/api/v1/collections", endpoint))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to connect to Chroma: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::Other(format!(
                "Failed to create collection: {} - {}",
                status, body
            )));
        }

        let collection: CollectionResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        Ok(ChromaBackend {
            client,
            endpoint,
            collection_id: collection.id,
            collection_name: collection.name,
        })
    }

    /// Generate document ID from path.
    fn path_to_id(path: &str) -> String {
        // Use a sanitized version of the path as the ID
        path.replace('/', "_").trim_start_matches('_').to_string()
    }

    /// Store a document with optional dense and sparse embeddings.
    pub async fn upsert(
        &self,
        path: &str,
        content: &str,
        embedding: Option<Vec<f32>>,
        sparse_embedding: Option<SparseEmbedding>,
        metadata: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), BackendError> {
        let id = Self::path_to_id(path);

        let mut meta = metadata.unwrap_or_default();
        meta.insert("path".to_string(), serde_json::json!(path));
        meta.insert(
            "updated_at".to_string(),
            serde_json::json!(Utc::now().to_rfc3339()),
        );

        // Store sparse embedding in metadata as JSON (Chroma doesn't natively support sparse vectors)
        if let Some(ref sparse) = sparse_embedding {
            meta.insert(
                "_sparse_indices".to_string(),
                serde_json::json!(sparse.indices),
            );
            meta.insert(
                "_sparse_values".to_string(),
                serde_json::json!(sparse.values),
            );
        }

        let request = AddDocumentsRequest {
            ids: vec![id],
            embeddings: embedding.map(|e| vec![e]),
            documents: Some(vec![content.to_string()]),
            metadatas: Some(vec![meta]),
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/upsert",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::Other(format!(
                "Failed to upsert document: {} - {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Query by embedding vector.
    pub async fn query_by_embedding(
        &self,
        embedding: Vec<f32>,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError> {
        let request = QueryRequest {
            query_embeddings: Some(vec![embedding]),
            query_texts: None,
            n_results,
            r#where: None,
            include: Some(vec![
                "documents".to_string(),
                "metadatas".to_string(),
                "distances".to_string(),
            ]),
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/query",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma query failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::Other(format!(
                "Query failed: {} - {}",
                status, body
            )));
        }

        let result: QueryResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse query response: {}", e)))?;

        let mut results = Vec::new();
        if let (Some(ids), Some(documents), Some(distances)) =
            (result.ids.first(), result.documents.as_ref().and_then(|d| d.first()), result.distances.as_ref().and_then(|d| d.first()))
        {
            for (i, id) in ids.iter().enumerate() {
                let doc = documents.get(i).and_then(|d| d.clone());
                let dist = distances.get(i).copied().unwrap_or(0.0);
                let metadata = result
                    .metadatas
                    .as_ref()
                    .and_then(|m| m.first())
                    .and_then(|m| m.get(i))
                    .and_then(|m| m.clone());

                results.push(QueryResult {
                    id: id.clone(),
                    document: doc,
                    distance: dist,
                    score: 1.0 - dist, // Convert distance to similarity
                    metadata,
                });
            }
        }

        Ok(results)
    }

    /// Get collection name.
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Query by sparse embedding — fetches all documents with sparse vectors and scores them client-side.
    ///
    /// Chroma doesn't natively support sparse vector search, so we retrieve documents
    /// that have sparse metadata and compute dot-product scores locally.
    pub async fn query_by_sparse_embedding(
        &self,
        query_sparse: &SparseEmbedding,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError> {
        // Fetch all documents with sparse metadata
        let request = GetDocumentsRequest {
            ids: None,
            r#where: Some(serde_json::json!({"_sparse_indices": {"$ne": ""}})),
            include: Some(vec![
                "documents".to_string(),
                "metadatas".to_string(),
            ]),
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/get",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            // If the filter doesn't work (some Chroma versions), fall back to getting all docs
            return self.query_sparse_fallback(query_sparse, n_results).await;
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        self.score_sparse_results(&result, query_sparse, n_results)
    }

    /// Fallback: get all documents and filter for ones with sparse vectors.
    async fn query_sparse_fallback(
        &self,
        query_sparse: &SparseEmbedding,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError> {
        let request = GetDocumentsRequest {
            ids: None,
            r#where: None,
            include: Some(vec![
                "documents".to_string(),
                "metadatas".to_string(),
            ]),
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/get",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        self.score_sparse_results(&result, query_sparse, n_results)
    }

    /// Score documents by sparse dot product, returning top N.
    fn score_sparse_results(
        &self,
        result: &GetDocumentsResponse,
        query_sparse: &SparseEmbedding,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError> {
        let mut scored: Vec<QueryResult> = Vec::new();

        for (i, id) in result.ids.iter().enumerate() {
            let metadata = result
                .metadatas
                .as_ref()
                .and_then(|m| m.get(i))
                .and_then(|m| m.clone());

            // Extract sparse vector from metadata
            let sparse = metadata.as_ref().and_then(|m| {
                let indices = m.get("_sparse_indices")?.as_array()?;
                let values = m.get("_sparse_values")?.as_array()?;
                let indices: Vec<u32> = indices.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect();
                let values: Vec<f32> = values.iter().filter_map(|v| v.as_f64().map(|n| n as f32)).collect();
                if indices.is_empty() { return None; }
                Some(SparseEmbedding { indices, values })
            });

            if let Some(doc_sparse) = sparse {
                let score = sparse_dot_product(query_sparse, &doc_sparse);
                if score > 0.0 {
                    let doc = result
                        .documents
                        .as_ref()
                        .and_then(|d| d.get(i))
                        .and_then(|d| d.clone());

                    scored.push(QueryResult {
                        id: id.clone(),
                        document: doc,
                        distance: 1.0 - score, // Convert to distance
                        score,
                        metadata,
                    });
                }
            }
        }

        // Sort by score descending
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(n_results);
        Ok(scored)
    }

    /// Delete all documents matching a metadata filter (e.g., all chunks for a source_path).
    pub async fn delete_by_metadata(
        &self,
        filter: serde_json::Value,
    ) -> Result<usize, BackendError> {
        // First, find matching IDs
        let request = GetDocumentsRequest {
            ids: None,
            r#where: Some(filter.clone()),
            include: None,
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/get",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(0);
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        if result.ids.is_empty() {
            return Ok(0);
        }

        let count = result.ids.len();

        // Delete by IDs
        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/delete",
                self.endpoint, self.collection_id
            ))
            .json(&serde_json::json!({ "ids": result.ids }))
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma delete failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::Other(format!(
                "Failed to delete by metadata: {} - {}",
                status, body
            )));
        }

        Ok(count)
    }

    /// Set collection metadata (used for persisting SparseEncoder state).
    pub async fn set_collection_metadata(
        &self,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<(), BackendError> {
        let response = self
            .client
            .put(format!(
                "{}/api/v1/collections/{}",
                self.endpoint, self.collection_id
            ))
            .json(&serde_json::json!({ "metadata": metadata }))
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::Other(format!(
                "Failed to set collection metadata: {} - {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Get collection metadata (used for loading SparseEncoder state).
    pub async fn get_collection_metadata(
        &self,
    ) -> Result<Option<HashMap<String, serde_json::Value>>, BackendError> {
        let response = self
            .client
            .get(format!(
                "{}/api/v1/collections/{}",
                self.endpoint, self.collection_id
            ))
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(None);
        }

        #[derive(Deserialize)]
        struct CollectionDetail {
            #[serde(default)]
            metadata: Option<HashMap<String, serde_json::Value>>,
        }

        let detail: CollectionDetail = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        Ok(detail.metadata)
    }
}

/// Compute sparse dot product between two sparse vectors.
fn sparse_dot_product(a: &SparseEmbedding, b: &SparseEmbedding) -> f32 {
    let b_map: HashMap<u32, f32> = b.indices.iter().copied().zip(b.values.iter().copied()).collect();
    let mut score = 0.0f32;
    for (idx, val) in a.indices.iter().zip(a.values.iter()) {
        if let Some(&b_val) = b_map.get(idx) {
            score += val * b_val;
        }
    }
    score
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

#[async_trait]
impl Backend for ChromaBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let id = Self::path_to_id(path);

        let request = GetDocumentsRequest {
            ids: Some(vec![id]),
            r#where: None,
            include: Some(vec!["documents".to_string()]),
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/get",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(BackendError::NotFound(path.to_string()));
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        if result.ids.is_empty() {
            return Err(BackendError::NotFound(path.to_string()));
        }

        let doc = result
            .documents
            .and_then(|d| d.into_iter().next())
            .flatten()
            .ok_or_else(|| BackendError::NotFound(path.to_string()))?;

        Ok(doc.into_bytes())
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let text = String::from_utf8_lossy(content).to_string();
        self.upsert(path, &text, None, None, None).await
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let existing = match self.read(path).await {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(BackendError::NotFound(_)) => String::new(),
            Err(e) => return Err(e),
        };

        let new_content = format!("{}{}", existing, String::from_utf8_lossy(content));
        self.upsert(path, &new_content, None, None, None).await
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        let id = Self::path_to_id(path);

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/delete",
                self.endpoint, self.collection_id
            ))
            .json(&serde_json::json!({ "ids": [id] }))
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::Other(format!(
                "Failed to delete: {} - {}",
                status, body
            )));
        }

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        // List all documents with path prefix
        let prefix = if path.is_empty() || path == "/" {
            String::new()
        } else {
            format!("{}/", path.trim_matches('/'))
        };

        let request = GetDocumentsRequest {
            ids: None,
            r#where: None,
            include: Some(vec!["metadatas".to_string()]),
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/get",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        let mut entries = Vec::new();
        let mut seen_dirs = std::collections::HashSet::new();

        for (i, _id) in result.ids.iter().enumerate() {
            let metadata = result
                .metadatas
                .as_ref()
                .and_then(|m| m.get(i))
                .and_then(|m| m.clone());

            if let Some(meta) = metadata {
                if let Some(serde_json::Value::String(file_path)) = meta.get("path") {
                    // Check if path matches prefix
                    let relative = if prefix.is_empty() {
                        file_path.trim_start_matches('/').to_string()
                    } else if file_path.starts_with(&prefix) {
                        file_path[prefix.len()..].to_string()
                    } else {
                        continue;
                    };

                    if relative.is_empty() {
                        continue;
                    }

                    // Check if immediate child or in subdirectory
                    if let Some(slash_pos) = relative.find('/') {
                        // It's in a subdirectory
                        let dir_name = &relative[..slash_pos];
                        if seen_dirs.insert(dir_name.to_string()) {
                            entries.push(Entry::dir(
                                format!("{}{}", prefix, dir_name),
                                dir_name.to_string(),
                                None,
                            ));
                        }
                    } else {
                        // It's an immediate child
                        let modified = meta
                            .get("updated_at")
                            .and_then(|v| v.as_str())
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.with_timezone(&Utc));

                        entries.push(Entry::file(
                            file_path.clone(),
                            relative.clone(),
                            0, // Size not tracked
                            modified,
                        ));
                    }
                }
            }
        }

        // Sort: directories first, then alphabetically
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        match self.read(path).await {
            Ok(_) => Ok(true),
            Err(BackendError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let id = Self::path_to_id(path);

        let request = GetDocumentsRequest {
            ids: Some(vec![id]),
            r#where: None,
            include: Some(vec!["documents".to_string(), "metadatas".to_string()]),
        };

        let response = self
            .client
            .post(format!(
                "{}/api/v1/collections/{}/get",
                self.endpoint, self.collection_id
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(BackendError::NotFound(path.to_string()));
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        if result.ids.is_empty() {
            return Err(BackendError::NotFound(path.to_string()));
        }

        let size = result
            .documents
            .as_ref()
            .and_then(|d| d.first())
            .and_then(|d| d.as_ref())
            .map(|d| d.len() as u64)
            .unwrap_or(0);

        let modified = result
            .metadatas
            .as_ref()
            .and_then(|m| m.first())
            .and_then(|m| m.as_ref())
            .and_then(|m| m.get("updated_at"))
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let name = path.rsplit('/').next().unwrap_or(path).to_string();

        Ok(Entry::file(path.to_string(), name, size, modified))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_id() {
        assert_eq!(ChromaBackend::path_to_id("/workspace/test.txt"), "workspace_test.txt");
        assert_eq!(ChromaBackend::path_to_id("test.txt"), "test.txt");
    }

    #[tokio::test]
    #[ignore] // Requires running Chroma instance
    async fn test_chroma_backend() {
        let backend = ChromaBackend::new("http://localhost:8000", "test_collection")
            .await
            .unwrap();

        backend.write("/test.txt", b"hello world").await.unwrap();
        let content = backend.read("/test.txt").await.unwrap();
        assert_eq!(content, b"hello world");

        backend.delete("/test.txt").await.unwrap();
        assert!(!backend.exists("/test.txt").await.unwrap());
    }

    #[test]
    fn test_sparse_dot_product() {
        let a = SparseEmbedding {
            indices: vec![0, 1, 2],
            values: vec![1.0, 2.0, 3.0],
        };
        let b = SparseEmbedding {
            indices: vec![1, 2, 3],
            values: vec![1.0, 1.0, 1.0],
        };
        let score = sparse_dot_product(&a, &b);
        assert!((score - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_sparse_dot_product_no_overlap() {
        let a = SparseEmbedding {
            indices: vec![0, 1],
            values: vec![1.0, 2.0],
        };
        let b = SparseEmbedding {
            indices: vec![10, 11],
            values: vec![1.0, 1.0],
        };
        let score = sparse_dot_product(&a, &b);
        assert_eq!(score, 0.0);
    }
}

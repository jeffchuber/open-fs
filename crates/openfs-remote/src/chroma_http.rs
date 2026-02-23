use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use openfs_core::{Backend, BackendError, ChromaStore, Entry, QueryResult, SparseEmbedding, TextEmbedder};

const DEFAULT_TENANT: &str = "default_tenant";
const DEFAULT_DATABASE: &str = "default_database";

#[derive(Debug, Clone, Copy)]
enum ChromaApiVersion {
    V1,
    V2,
}

/// Chroma vector database backend (HTTP client).
/// Uses Chroma's HTTP API for storing files and their embeddings.
pub struct ChromaHttpBackend {
    client: Client,
    endpoint: String,
    collection_id: String,
    collection_name: String,
    api_version: ChromaApiVersion,
    tenant: String,
    database: String,
    /// Optional embedder for automatic embedding on write.
    embedder: Option<Arc<dyn TextEmbedder>>,
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

#[derive(Serialize)]
struct AddDocumentsRequest {
    ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embeddings: Option<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadatas: Option<Vec<HashMap<String, serde_json::Value>>>,
    /// Per-record expected versions for CAS. Each entry corresponds to the
    /// record at the same index. `None` entries skip version checking.
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_versions: Option<Vec<Option<i64>>>,
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
    /// Per-record versions (log offsets). Populated when include=["versions"].
    #[serde(default)]
    versions: Option<Vec<Option<i64>>>,
}

impl ChromaHttpBackend {
    /// Create a new Chroma HTTP backend.
    pub async fn new(
        endpoint: &str,
        collection_name: &str,
        api_key: Option<&str>,
        tenant: Option<&str>,
        database: Option<&str>,
    ) -> Result<Self, BackendError> {
        let mut client_builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5));

        // Add default auth header for Chroma Cloud
        if let Some(key) = api_key {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "X-Chroma-Token",
                reqwest::header::HeaderValue::from_str(key)
                    .map_err(|e| BackendError::Other(format!("Invalid API key: {}", e)))?,
            );
            // Also set Authorization header as Bearer token (Chroma Cloud accepts both)
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key))
                    .map_err(|e| BackendError::Other(format!("Invalid API key: {}", e)))?,
            );
            client_builder = client_builder.default_headers(headers);
        }

        let client = client_builder
            .build()
            .map_err(|e| BackendError::Other(format!("Failed to build HTTP client: {}", e)))?;
        let endpoint = endpoint.trim_end_matches('/').to_string();

        let tenant = tenant.unwrap_or(DEFAULT_TENANT).to_string();
        let database = database.unwrap_or(DEFAULT_DATABASE).to_string();

        // Prefer v2, fall back to v1 for older servers.
        match Self::create_collection_v2(&client, &endpoint, collection_name, &tenant, &database)
            .await
        {
            Ok(collection) => Ok(ChromaHttpBackend {
                client,
                endpoint,
                collection_id: collection.id,
                collection_name: collection.name,
                api_version: ChromaApiVersion::V2,
                tenant,
                database,
                embedder: None,
            }),
            Err((status, body))
                if matches!(status.as_u16(), 404 | 405 | 501)
                    || body.contains("Not Found")
                    || body.contains("not found") =>
            {
                let collection =
                    Self::create_collection_v1(&client, &endpoint, collection_name).await?;
                Ok(ChromaHttpBackend {
                    client,
                    endpoint,
                    collection_id: collection.id,
                    collection_name: collection.name,
                    api_version: ChromaApiVersion::V1,
                    tenant,
                    database,
                    embedder: None,
                })
            }
            Err((status, body)) => Err(BackendError::Other(format!(
                "Failed to create collection: {} - {}",
                status, body
            ))),
        }
    }

    async fn create_collection_v1(
        client: &Client,
        endpoint: &str,
        collection_name: &str,
    ) -> Result<CollectionResponse, BackendError> {
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

        response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))
    }

    async fn create_collection_v2(
        client: &Client,
        endpoint: &str,
        collection_name: &str,
        tenant: &str,
        database: &str,
    ) -> Result<CollectionResponse, (reqwest::StatusCode, String)> {
        let request = CreateCollectionRequest {
            name: collection_name.to_string(),
            metadata: None,
            get_or_create: true,
        };

        let response = client
            .post(format!(
                "{}/api/v2/tenants/{}/databases/{}/collections",
                endpoint, tenant, database
            ))
            .json(&request)
            .send()
            .await
            .map_err(|err| {
                (
                    reqwest::StatusCode::SERVICE_UNAVAILABLE,
                    format!("Failed to connect to Chroma: {}", err),
                )
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err((status, body));
        }

        response
            .json()
            .await
            .map_err(|err| (reqwest::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
    }

    fn collection_url(&self) -> String {
        match self.api_version {
            ChromaApiVersion::V1 => {
                format!(
                    "{}/api/v1/collections/{}",
                    self.endpoint, self.collection_id
                )
            }
            ChromaApiVersion::V2 => format!(
                "{}/api/v2/tenants/{}/databases/{}/collections/{}",
                self.endpoint, self.tenant, self.database, self.collection_id
            ),
        }
    }

    fn collection_op_url(&self, op: &str) -> String {
        format!("{}/{}", self.collection_url(), op)
    }

    /// Fetch the per-record version for a given path.
    /// Uses `get` with `include=["versions"]` on V1; not available on Chroma Cloud (V2).
    async fn record_version(&self, path: &str) -> Result<Option<String>, BackendError> {
        // Chroma Cloud doesn't support "versions" — return None (no CAS support).
        if matches!(self.api_version, ChromaApiVersion::V2) {
            return Ok(None);
        }

        let id = Self::path_to_id(path);

        let request = GetDocumentsRequest {
            ids: Some(vec![id]),
            r#where: None,
            include: Some(vec!["versions".to_string()]),
        };

        let response = self
            .client
            .post(self.collection_op_url("get"))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        if result.ids.is_empty() {
            return Ok(None);
        }

        Ok(result
            .versions
            .and_then(|v| v.into_iter().next())
            .flatten()
            .map(|v| v.to_string()))
    }

    async fn upsert_document(
        &self,
        path: &str,
        content: &str,
        embedding: Option<Vec<f32>>,
        sparse_embedding: Option<SparseEmbedding>,
        metadata: Option<HashMap<String, serde_json::Value>>,
        expected_version: Option<i64>,
    ) -> Result<(), BackendError> {
        let id = Self::path_to_id(path);

        let mut meta = metadata.unwrap_or_default();
        meta.insert("path".to_string(), serde_json::json!(path));
        meta.insert(
            "updated_at".to_string(),
            serde_json::json!(Utc::now().to_rfc3339()),
        );

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

        let embeddings = match (embedding, self.api_version) {
            (Some(v), _) => Some(vec![v]),
            (None, ChromaApiVersion::V2) => Some(vec![vec![0.0]]),
            (None, ChromaApiVersion::V1) => None,
        };

        let expected_versions =
            expected_version.map(|v| vec![Some(v)]);

        let request = AddDocumentsRequest {
            ids: vec![id],
            embeddings,
            documents: Some(vec![content.to_string()]),
            metadatas: Some(vec![meta]),
            expected_versions,
        };

        let response = self
            .client
            .post(self.collection_op_url("upsert"))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::PRECONDITION_FAILED
            || status == reqwest::StatusCode::CONFLICT
        {
            return Err(BackendError::PreconditionFailed {
                path: path.to_string(),
                expected: expected_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unspecified".to_string()),
                actual: body,
            });
        }

        Err(BackendError::Other(format!(
            "Failed to upsert document: {} - {}",
            status, body
        )))
    }

    /// Set an embedder for automatic embedding on writes.
    pub fn set_embedder(&mut self, embedder: Arc<dyn TextEmbedder>) {
        self.embedder = Some(embedder);
    }

    /// Generate document ID from path.
    fn path_to_id(path: &str) -> String {
        path.replace('/', "_").trim_start_matches('_').to_string()
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
            include: Some(vec!["documents".to_string(), "metadatas".to_string()]),
        };

        let response = self
            .client
            .post(self.collection_op_url("get"))
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
                let indices: Vec<u32> = indices
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect();
                let values: Vec<f32> = values
                    .iter()
                    .filter_map(|v| v.as_f64().map(|n| n as f32))
                    .collect();
                if indices.is_empty() {
                    return None;
                }
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
                        distance: 1.0 - score,
                        score,
                        metadata,
                    });
                }
            }
        }

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(n_results);
        Ok(scored)
    }
}

/// Compute sparse dot product between two sparse vectors.
fn sparse_dot_product(a: &SparseEmbedding, b: &SparseEmbedding) -> f32 {
    let b_map: HashMap<u32, f32> = b
        .indices
        .iter()
        .copied()
        .zip(b.values.iter().copied())
        .collect();
    let mut score = 0.0f32;
    for (idx, val) in a.indices.iter().zip(a.values.iter()) {
        if let Some(&b_val) = b_map.get(idx) {
            score += val * b_val;
        }
    }
    score
}

#[async_trait]
impl ChromaStore for ChromaHttpBackend {
    async fn upsert(
        &self,
        path: &str,
        content: &str,
        embedding: Option<Vec<f32>>,
        sparse_embedding: Option<SparseEmbedding>,
        metadata: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), BackendError> {
        self.upsert_document(path, content, embedding, sparse_embedding, metadata, None)
            .await
    }

    async fn query_by_embedding(
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
            .post(self.collection_op_url("query"))
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
        if let (Some(ids), Some(documents), Some(distances)) = (
            result.ids.first(),
            result.documents.as_ref().and_then(|d| d.first()),
            result.distances.as_ref().and_then(|d| d.first()),
        ) {
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
                    score: 1.0 - dist,
                    metadata,
                });
            }
        }

        Ok(results)
    }

    async fn query_by_sparse_embedding(
        &self,
        query_sparse: &SparseEmbedding,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError> {
        let request = GetDocumentsRequest {
            ids: None,
            r#where: Some(serde_json::json!({"_sparse_indices": {"$ne": ""}})),
            include: Some(vec!["documents".to_string(), "metadatas".to_string()]),
        };

        let response = self
            .client
            .post(self.collection_op_url("get"))
            .json(&request)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("Chroma request failed: {}", e)))?;

        if !response.status().is_success() {
            return self.query_sparse_fallback(query_sparse, n_results).await;
        }

        let result: GetDocumentsResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("Failed to parse response: {}", e)))?;

        self.score_sparse_results(&result, query_sparse, n_results)
    }

    async fn delete_by_metadata(&self, filter: serde_json::Value) -> Result<usize, BackendError> {
        let request = GetDocumentsRequest {
            ids: None,
            r#where: Some(filter.clone()),
            include: None,
        };

        let response = self
            .client
            .post(self.collection_op_url("get"))
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

        let response = self
            .client
            .post(self.collection_op_url("delete"))
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

    async fn set_collection_metadata(
        &self,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<(), BackendError> {
        let response = self
            .client
            .put(self.collection_url())
            .json(&serde_json::json!({ "new_metadata": metadata }))
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

    async fn get_collection_metadata(
        &self,
    ) -> Result<Option<HashMap<String, serde_json::Value>>, BackendError> {
        let response = self
            .client
            .get(self.collection_url())
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

    fn collection_name(&self) -> &str {
        &self.collection_name
    }
}

#[async_trait]
impl Backend for ChromaHttpBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let id = Self::path_to_id(path);

        let request = GetDocumentsRequest {
            ids: Some(vec![id]),
            r#where: None,
            include: Some(vec!["documents".to_string()]),
        };

        let response = self
            .client
            .post(self.collection_op_url("get"))
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

    async fn read_with_cas_token(
        &self,
        path: &str,
    ) -> Result<(Vec<u8>, Option<String>), BackendError> {
        let id = Self::path_to_id(path);

        // Chroma Cloud (V2) doesn't support "versions" in include — only request
        // it for self-hosted V1 instances.
        let include = match self.api_version {
            ChromaApiVersion::V1 => vec!["documents".to_string(), "versions".to_string()],
            ChromaApiVersion::V2 => vec!["documents".to_string()],
        };

        let request = GetDocumentsRequest {
            ids: Some(vec![id]),
            r#where: None,
            include: Some(include),
        };

        let response = self
            .client
            .post(self.collection_op_url("get"))
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

        let version = result
            .versions
            .and_then(|v| v.into_iter().next())
            .flatten()
            .map(|v| v.to_string());

        Ok((doc.into_bytes(), version))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let text = String::from_utf8_lossy(content).to_string();
        let embedding = match &self.embedder {
            Some(embedder) => Some(embedder.embed_text(&text).await?),
            None => None,
        };
        ChromaStore::upsert(self, path, &text, embedding, None, None).await
    }

    async fn compare_and_swap(
        &self,
        path: &str,
        expected: Option<&str>,
        content: &[u8],
    ) -> Result<Option<String>, BackendError> {
        let expected_token = expected.map(ToString::to_string);
        let expected_version = match expected_token.as_deref() {
            Some(token) => Some(token.parse::<i64>().map_err(|_| {
                BackendError::Other(format!(
                    "Invalid CAS token '{}': expected a numeric record version",
                    token
                ))
            })?),
            None => None,
        };

        let text = String::from_utf8_lossy(content).to_string();
        let embedding = match &self.embedder {
            Some(embedder) => Some(embedder.embed_text(&text).await?),
            None => None,
        };
        match self
            .upsert_document(path, &text, embedding, None, None, expected_version)
            .await
        {
            Ok(()) => {
                // Fetch the new record version after successful write
                self.record_version(path).await
            }
            Err(BackendError::PreconditionFailed { .. }) => {
                let actual = self
                    .record_version(path)
                    .await?
                    .unwrap_or_else(|| "unknown".to_string());
                Err(BackendError::PreconditionFailed {
                    path: path.to_string(),
                    expected: expected_token.unwrap_or_else(|| "unspecified".to_string()),
                    actual,
                })
            }
            Err(err) => Err(err),
        }
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let existing = match self.read(path).await {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(BackendError::NotFound(_)) => String::new(),
            Err(e) => return Err(e),
        };

        let new_content = format!("{}{}", existing, String::from_utf8_lossy(content));
        let embedding = match &self.embedder {
            Some(embedder) => Some(embedder.embed_text(&new_content).await?),
            None => None,
        };
        ChromaStore::upsert(self, path, &new_content, embedding, None, None).await
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        let id = Self::path_to_id(path);

        let response = self
            .client
            .post(self.collection_op_url("delete"))
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
            .post(self.collection_op_url("get"))
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

                    if let Some(slash_pos) = relative.find('/') {
                        let dir_name = &relative[..slash_pos];
                        if seen_dirs.insert(dir_name.to_string()) {
                            entries.push(Entry::dir(
                                format!("{}{}", prefix, dir_name),
                                dir_name.to_string(),
                                None,
                            ));
                        }
                    } else {
                        let modified = meta
                            .get("updated_at")
                            .and_then(|v| v.as_str())
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.with_timezone(&Utc));

                        entries.push(Entry::file(
                            file_path.clone(),
                            relative.clone(),
                            0,
                            modified,
                        ));
                    }
                }
            }
        }

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
            .post(self.collection_op_url("get"))
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
        assert_eq!(
            ChromaHttpBackend::path_to_id("/workspace/test.txt"),
            "workspace_test.txt"
        );
        assert_eq!(ChromaHttpBackend::path_to_id("test.txt"), "test.txt");
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

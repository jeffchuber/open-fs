use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use openfs_core::{BackendError, ChromaStore, QueryResult, SparseEmbedding};

/// A single document stored in the mock Chroma store.
#[derive(Debug, Clone)]
pub struct MockDoc {
    pub path: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub sparse_embedding: Option<SparseEmbedding>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// In-memory implementation of `ChromaStore` for deterministic testing.
///
/// Both agents' `IndexingPipeline` instances reference the same `Arc<MockChromaStore>`,
/// so agent 0's indexed files are immediately searchable by agent 1.
pub struct MockChromaStore {
    collection_name: String,
    docs: RwLock<HashMap<String, MockDoc>>,
    collection_metadata: RwLock<Option<HashMap<String, serde_json::Value>>>,
}

impl MockChromaStore {
    pub fn new(collection_name: &str) -> Self {
        MockChromaStore {
            collection_name: collection_name.to_string(),
            docs: RwLock::new(HashMap::new()),
            collection_metadata: RwLock::new(None),
        }
    }

    /// Return a snapshot of all stored documents for oracle inspection.
    pub fn snapshot(&self) -> HashMap<String, MockDoc> {
        self.docs.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Check if a document exists by path prefix (e.g. "file.txt" matches "file.txt#chunk_0").
    pub fn has_docs_for_path(&self, source_path: &str) -> bool {
        let docs = self.docs.read().unwrap_or_else(|e| e.into_inner());
        docs.values().any(|doc| {
            doc.metadata
                .as_ref()
                .and_then(|m| m.get("source_path"))
                .and_then(|v| v.as_str())
                .map(|sp| sp == source_path)
                .unwrap_or(false)
        })
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

fn sparse_dot_product(a: &SparseEmbedding, b: &SparseEmbedding) -> f32 {
    let mut ai = 0;
    let mut bi = 0;
    let mut dot = 0.0f32;
    while ai < a.indices.len() && bi < b.indices.len() {
        match a.indices[ai].cmp(&b.indices[bi]) {
            std::cmp::Ordering::Equal => {
                dot += a.values[ai] * b.values[bi];
                ai += 1;
                bi += 1;
            }
            std::cmp::Ordering::Less => ai += 1,
            std::cmp::Ordering::Greater => bi += 1,
        }
    }
    dot
}

#[async_trait]
impl ChromaStore for MockChromaStore {
    async fn upsert(
        &self,
        path: &str,
        content: &str,
        embedding: Option<Vec<f32>>,
        sparse_embedding: Option<SparseEmbedding>,
        metadata: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), BackendError> {
        let mut docs = self.docs.write().unwrap_or_else(|e| e.into_inner());
        docs.insert(
            path.to_string(),
            MockDoc {
                path: path.to_string(),
                content: content.to_string(),
                embedding,
                sparse_embedding,
                metadata,
            },
        );
        Ok(())
    }

    async fn query_by_embedding(
        &self,
        embedding: Vec<f32>,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError> {
        let docs = self.docs.read().unwrap_or_else(|e| e.into_inner());
        let mut scored: Vec<(String, f32, &MockDoc)> = docs
            .iter()
            .filter_map(|(id, doc)| {
                doc.embedding.as_ref().map(|emb| {
                    let sim = cosine_similarity(&embedding, emb);
                    (id.clone(), sim, doc)
                })
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(n_results);

        Ok(scored
            .into_iter()
            .map(|(id, score, doc)| QueryResult {
                id,
                document: Some(doc.content.clone()),
                distance: 1.0 - score,
                score,
                metadata: doc.metadata.clone(),
            })
            .collect())
    }

    async fn query_by_sparse_embedding(
        &self,
        query_sparse: &SparseEmbedding,
        n_results: usize,
    ) -> Result<Vec<QueryResult>, BackendError> {
        let docs = self.docs.read().unwrap_or_else(|e| e.into_inner());
        let mut scored: Vec<(String, f32, &MockDoc)> = docs
            .iter()
            .filter_map(|(id, doc)| {
                doc.sparse_embedding.as_ref().map(|se| {
                    let dot = sparse_dot_product(query_sparse, se);
                    (id.clone(), dot, doc)
                })
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(n_results);

        Ok(scored
            .into_iter()
            .map(|(id, score, doc)| QueryResult {
                id,
                document: Some(doc.content.clone()),
                distance: 1.0 - score,
                score,
                metadata: doc.metadata.clone(),
            })
            .collect())
    }

    async fn delete_by_metadata(&self, filter: serde_json::Value) -> Result<usize, BackendError> {
        let mut docs = self.docs.write().unwrap_or_else(|e| e.into_inner());
        let before = docs.len();

        docs.retain(|_id, doc| {
            if let Some(metadata) = &doc.metadata {
                if let Some(filter_map) = filter.as_object() {
                    for (key, expected_val) in filter_map {
                        if let Some(actual_val) = metadata.get(key) {
                            if actual_val == expected_val {
                                return false; // remove this doc
                            }
                        }
                    }
                }
            }
            true // keep
        });

        Ok(before - docs.len())
    }

    async fn set_collection_metadata(
        &self,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<(), BackendError> {
        let mut cm = self
            .collection_metadata
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *cm = Some(metadata);
        Ok(())
    }

    async fn get_collection_metadata(
        &self,
    ) -> Result<Option<HashMap<String, serde_json::Value>>, BackendError> {
        let cm = self
            .collection_metadata
            .read()
            .unwrap_or_else(|e| e.into_inner());
        Ok(cm.clone())
    }

    fn collection_name(&self) -> &str {
        &self.collection_name
    }
}

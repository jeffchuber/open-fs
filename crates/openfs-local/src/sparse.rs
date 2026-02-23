use crate::{IndexingError, SparseVector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// BM25 sparse encoder for keyword search.
#[derive(Serialize, Deserialize)]
pub struct SparseEncoder {
    /// Term to index mapping.
    vocab: HashMap<String, u32>,
    /// Inverse document frequencies.
    idf: HashMap<u32, f32>,
    /// Next vocabulary index.
    next_index: u32,
    /// BM25 k1 parameter.
    k1: f32,
    /// BM25 b parameter.
    b: f32,
    /// Average document length.
    avg_doc_len: f32,
    /// Number of documents indexed.
    doc_count: usize,
}

impl Default for SparseEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseEncoder {
    pub fn new() -> Self {
        SparseEncoder {
            vocab: HashMap::new(),
            idf: HashMap::new(),
            next_index: 0,
            k1: 1.5,
            b: 0.75,
            avg_doc_len: 100.0,
            doc_count: 0,
        }
    }

    pub fn with_params(k1: f32, b: f32) -> Self {
        SparseEncoder {
            k1,
            b,
            ..Self::new()
        }
    }

    /// Tokenize text into terms.
    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty() && s.len() >= 2)
            .map(String::from)
            .collect()
    }

    /// Get term frequencies for a document.
    fn term_frequencies(tokens: &[String]) -> HashMap<String, usize> {
        let mut freqs = HashMap::new();
        for token in tokens {
            *freqs.entry(token.clone()).or_insert(0) += 1;
        }
        freqs
    }

    /// Get or create vocabulary index for a term.
    fn get_or_create_index(&mut self, term: &str) -> u32 {
        if let Some(&idx) = self.vocab.get(term) {
            idx
        } else {
            let idx = self.next_index;
            self.vocab.insert(term.to_string(), idx);
            self.next_index += 1;
            idx
        }
    }

    /// Update IDF values with a new document.
    pub fn update_idf(&mut self, text: &str) {
        let tokens = Self::tokenize(text);
        let unique_terms: std::collections::HashSet<_> = tokens.iter().collect();

        for term in unique_terms {
            let idx = self.get_or_create_index(term);
            let df = self.idf.entry(idx).or_insert(0.0);
            *df += 1.0;
        }

        self.doc_count += 1;

        // Update average document length
        let total_len = self.avg_doc_len * (self.doc_count - 1) as f32 + tokens.len() as f32;
        self.avg_doc_len = total_len / self.doc_count as f32;
    }

    /// Encode text to sparse vector using BM25 scoring.
    pub fn encode(&mut self, text: &str) -> Result<SparseVector, IndexingError> {
        let tokens = Self::tokenize(text);
        let doc_len = tokens.len() as f32;
        let term_freqs = Self::term_frequencies(&tokens);

        let mut indices = Vec::new();
        let mut values = Vec::new();

        for (term, tf) in term_freqs {
            let idx = self.get_or_create_index(&term);

            // Calculate BM25 score
            let tf_f = tf as f32;
            let df = self.idf.get(&idx).copied().unwrap_or(1.0);
            let idf = ((self.doc_count as f32 - df + 0.5) / (df + 0.5) + 1.0).ln();

            let numerator = tf_f * (self.k1 + 1.0);
            let denominator = tf_f + self.k1 * (1.0 - self.b + self.b * doc_len / self.avg_doc_len);

            let score = idf * numerator / denominator;

            if score > 0.0 {
                indices.push(idx);
                values.push(score);
            }
        }

        Ok(SparseVector { indices, values })
    }

    /// Encode text for query (slightly different scoring for queries).
    pub fn encode_query(&self, text: &str) -> Result<SparseVector, IndexingError> {
        let tokens = Self::tokenize(text);
        let term_freqs = Self::term_frequencies(&tokens);

        let mut indices = Vec::new();
        let mut values = Vec::new();

        for (term, tf) in term_freqs {
            if let Some(&idx) = self.vocab.get(&term) {
                // For queries, we use a simplified scoring
                let tf_f = tf as f32;
                let df = self.idf.get(&idx).copied().unwrap_or(1.0);
                let idf = ((self.doc_count as f32 - df + 0.5) / (df + 0.5) + 1.0).ln();

                let score = idf * (tf_f * (self.k1 + 1.0)) / (tf_f + self.k1);

                if score > 0.0 {
                    indices.push(idx);
                    values.push(score);
                }
            }
        }

        Ok(SparseVector { indices, values })
    }

    /// Calculate sparse dot product (for scoring).
    pub fn dot_product(a: &SparseVector, b: &SparseVector) -> f32 {
        let mut score = 0.0;
        let b_map: HashMap<u32, f32> = b
            .indices
            .iter()
            .copied()
            .zip(b.values.iter().copied())
            .collect();

        for (idx, val) in a.indices.iter().zip(a.values.iter()) {
            if let Some(&b_val) = b_map.get(idx) {
                score += val * b_val;
            }
        }

        score
    }

    /// Get vocabulary size.
    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    /// Serialize encoder state to JSON.
    pub fn to_json(&self) -> Result<String, IndexingError> {
        serde_json::to_string(self).map_err(IndexingError::JsonError)
    }

    /// Deserialize encoder state from JSON.
    pub fn from_json(s: &str) -> Result<Self, IndexingError> {
        serde_json::from_str(s).map_err(IndexingError::JsonError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens = SparseEncoder::tokenize("Hello, world! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Single letter words should be filtered
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_encode() {
        let mut encoder = SparseEncoder::new();

        // Add some documents to build IDF
        encoder.update_idf("the quick brown fox");
        encoder.update_idf("the lazy dog");
        encoder.update_idf("the quick dog jumps");

        let vector = encoder.encode("quick brown").unwrap();

        assert!(!vector.indices.is_empty());
        assert!(!vector.values.is_empty());
        assert_eq!(vector.indices.len(), vector.values.len());
    }

    #[test]
    fn test_dot_product() {
        let a = SparseVector {
            indices: vec![0, 1, 2],
            values: vec![1.0, 2.0, 3.0],
        };
        let b = SparseVector {
            indices: vec![1, 2, 3],
            values: vec![1.0, 1.0, 1.0],
        };

        let score = SparseEncoder::dot_product(&a, &b);
        // Should be 2.0 * 1.0 + 3.0 * 1.0 = 5.0
        assert!((score - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_default() {
        let encoder = SparseEncoder::default();
        assert_eq!(encoder.k1, 1.5);
        assert_eq!(encoder.b, 0.75);
    }

    #[test]
    fn test_with_params() {
        let encoder = SparseEncoder::with_params(2.0, 0.5);
        assert_eq!(encoder.k1, 2.0);
        assert_eq!(encoder.b, 0.5);
    }

    #[test]
    fn test_encode_empty_string() {
        let mut encoder = SparseEncoder::new();
        let vector = encoder.encode("").unwrap();
        assert!(vector.indices.is_empty());
        assert!(vector.values.is_empty());
    }

    #[test]
    fn test_encode_query_unknown_terms() {
        let encoder = SparseEncoder::new();
        // Query with terms not in vocabulary
        let vector = encoder.encode_query("unknown terms here").unwrap();
        // Should return empty since terms aren't in vocab
        assert!(vector.indices.is_empty());
    }

    #[test]
    fn test_to_json_from_json_roundtrip() {
        let mut encoder = SparseEncoder::new();
        encoder.update_idf("the quick brown fox");
        encoder.update_idf("the lazy dog");
        encoder.update_idf("the quick dog jumps");

        let json = encoder.to_json().unwrap();
        let restored = SparseEncoder::from_json(&json).unwrap();

        assert_eq!(restored.vocab_size(), encoder.vocab_size());
        assert_eq!(restored.doc_count, encoder.doc_count);
    }
}

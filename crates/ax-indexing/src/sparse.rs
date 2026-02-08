use crate::{IndexingError, SparseVector};
use std::collections::HashMap;

/// BM25 sparse encoder for keyword search.
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
        let b_map: HashMap<u32, f32> = b.indices.iter().copied().zip(b.values.iter().copied()).collect();

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
    fn test_tokenize_empty_string() {
        let tokens = SparseEncoder::tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_only_punctuation() {
        let tokens = SparseEncoder::tokenize("!@#$%^&*()");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_single_char_words_filtered() {
        let tokens = SparseEncoder::tokenize("a b c d e f");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_preserves_underscores() {
        let tokens = SparseEncoder::tokenize("variable_name another_var");
        assert!(tokens.contains(&"variable_name".to_string()));
        assert!(tokens.contains(&"another_var".to_string()));
    }

    #[test]
    fn test_tokenize_case_insensitive() {
        let tokens = SparseEncoder::tokenize("Hello WORLD hElLo");
        // All should be lowercase
        for token in &tokens {
            assert_eq!(token, &token.to_lowercase());
        }
    }

    #[test]
    fn test_tokenize_numbers() {
        let tokens = SparseEncoder::tokenize("test123 456test");
        assert!(tokens.contains(&"test123".to_string()));
        assert!(tokens.contains(&"456test".to_string()));
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
    fn test_encode_query_known_terms() {
        let mut encoder = SparseEncoder::new();
        encoder.update_idf("the quick brown fox");
        encoder.update_idf("the lazy dog");

        let vector = encoder.encode_query("quick fox").unwrap();
        assert!(!vector.indices.is_empty());
    }

    #[test]
    fn test_update_idf_increases_doc_count() {
        let mut encoder = SparseEncoder::new();
        assert_eq!(encoder.doc_count, 0);

        encoder.update_idf("document one");
        assert_eq!(encoder.doc_count, 1);

        encoder.update_idf("document two");
        assert_eq!(encoder.doc_count, 2);
    }

    #[test]
    fn test_vocab_size_grows() {
        let mut encoder = SparseEncoder::new();
        assert_eq!(encoder.vocab_size(), 0);

        encoder.update_idf("hello world");
        let size1 = encoder.vocab_size();
        assert!(size1 > 0);

        encoder.update_idf("new unique terms");
        let size2 = encoder.vocab_size();
        assert!(size2 > size1);
    }

    #[test]
    fn test_with_params() {
        let encoder = SparseEncoder::with_params(2.0, 0.5);
        assert_eq!(encoder.k1, 2.0);
        assert_eq!(encoder.b, 0.5);
    }

    #[test]
    fn test_default() {
        let encoder = SparseEncoder::default();
        assert_eq!(encoder.k1, 1.5);
        assert_eq!(encoder.b, 0.75);
    }

    #[test]
    fn test_dot_product_no_overlap() {
        let a = SparseVector {
            indices: vec![0, 1, 2],
            values: vec![1.0, 2.0, 3.0],
        };
        let b = SparseVector {
            indices: vec![10, 11, 12],
            values: vec![1.0, 1.0, 1.0],
        };

        let score = SparseEncoder::dot_product(&a, &b);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_dot_product_full_overlap() {
        let a = SparseVector {
            indices: vec![0, 1, 2],
            values: vec![1.0, 2.0, 3.0],
        };
        let b = SparseVector {
            indices: vec![0, 1, 2],
            values: vec![2.0, 2.0, 2.0],
        };

        let score = SparseEncoder::dot_product(&a, &b);
        // 1*2 + 2*2 + 3*2 = 12
        assert!((score - 12.0).abs() < 0.001);
    }

    #[test]
    fn test_dot_product_empty_vectors() {
        let a = SparseVector {
            indices: vec![],
            values: vec![],
        };
        let b = SparseVector {
            indices: vec![],
            values: vec![],
        };

        let score = SparseEncoder::dot_product(&a, &b);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_term_frequencies() {
        let tokens = vec![
            "hello".to_string(),
            "world".to_string(),
            "hello".to_string(),
            "hello".to_string(),
        ];
        let freqs = SparseEncoder::term_frequencies(&tokens);
        assert_eq!(freqs.get("hello"), Some(&3));
        assert_eq!(freqs.get("world"), Some(&1));
    }

    #[test]
    fn test_bm25_rare_term_higher_score() {
        let mut encoder = SparseEncoder::new();
        // Common term appears in all documents
        encoder.update_idf("common word here");
        encoder.update_idf("common word there");
        encoder.update_idf("common word everywhere");
        // Rare term appears in only one
        encoder.update_idf("rare unique special");

        let common_vec = encoder.encode("common").unwrap();
        let rare_vec = encoder.encode("rare").unwrap();

        // Rare term should have higher IDF weight
        let common_max = common_vec.values.iter().cloned().fold(0.0f32, f32::max);
        let rare_max = rare_vec.values.iter().cloned().fold(0.0f32, f32::max);
        assert!(rare_max > common_max);
    }

    #[test]
    fn test_avg_doc_len_updates() {
        let mut encoder = SparseEncoder::new();
        encoder.update_idf("short");
        let avg1 = encoder.avg_doc_len;

        encoder.update_idf("this is a much longer document with many words");
        let avg2 = encoder.avg_doc_len;

        assert!(avg2 > avg1);
    }
}

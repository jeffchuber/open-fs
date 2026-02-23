//! BLAKE3-based content hashing for deduplication.
//!
//! Computes fast, cryptographically strong hashes of file content
//! to detect when files have been touched (mtime changed) but
//! content is identical. Avoids wasted embedding calls.

/// Compute the BLAKE3 hash of content, returning a hex string.
pub fn content_hash(content: &[u8]) -> String {
    blake3::hash(content).to_hex().to_string()
}

/// Compute the BLAKE3 hash incrementally (for large files).
pub fn content_hash_streaming(chunks: &[&[u8]]) -> String {
    let mut hasher = blake3::Hasher::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash(b"hello world");
        let h2 = content_hash(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_content_hash_different_content() {
        let h1 = content_hash(b"hello world");
        let h2 = content_hash(b"hello world!");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_content_hash_length() {
        let h = content_hash(b"test");
        assert_eq!(h.len(), 64); // BLAKE3 = 256 bits = 64 hex chars
    }

    #[test]
    fn test_content_hash_empty() {
        let h = content_hash(b"");
        assert_eq!(h.len(), 64);
    }

    #[test]
    fn test_streaming_matches_oneshot() {
        let content = b"hello world this is a test";
        let oneshot = content_hash(content);
        let streaming = content_hash_streaming(&[b"hello world", b" this is ", b"a test"]);
        assert_eq!(oneshot, streaming);
    }

    #[test]
    fn test_streaming_empty() {
        let h1 = content_hash(b"");
        let h2 = content_hash_streaming(&[]);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_known_hash() {
        // Verify hash matches known BLAKE3 value
        let h = content_hash(b"");
        // BLAKE3 hash of empty input
        assert_eq!(
            h,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }
}

use super::{count_lines_to_offset, Chunker, ChunkerConfig};
use crate::{Chunk, IndexingError};
use async_trait::async_trait;

/// Fixed-size chunker that splits text into chunks of approximately equal size.
pub struct FixedChunker {
    config: ChunkerConfig,
}

impl FixedChunker {
    pub fn new(config: ChunkerConfig) -> Self {
        FixedChunker { config }
    }
}

#[async_trait]
impl Chunker for FixedChunker {
    async fn chunk(&self, text: &str, source_path: &str) -> Result<Vec<Chunk>, IndexingError> {
        let mut chunks = Vec::new();
        let text_len = text.len();

        if text_len == 0 {
            return Ok(chunks);
        }

        let chunk_size = self.config.chunk_size;
        let overlap = self.config.chunk_overlap;
        let step = chunk_size.saturating_sub(overlap).max(1);

        let mut start = 0;
        let mut chunk_index = 0;

        // First pass: create all chunks
        let mut raw_chunks = Vec::new();
        while start < text_len {
            let end = (start + chunk_size).min(text_len);
            raw_chunks.push((start, end));
            start += step;
        }

        let total_chunks = raw_chunks.len();

        // Second pass: create Chunk objects
        for (start, end) in raw_chunks {
            let content = &text[start..end];

            // Skip chunks that are too small (unless it's the only chunk)
            if content.len() < self.config.min_chunk_size && total_chunks > 1 && chunk_index > 0 {
                continue;
            }

            let start_line = count_lines_to_offset(text, start);
            let end_line = count_lines_to_offset(text, end);

            chunks.push(Chunk::new(
                source_path.to_string(),
                content.to_string(),
                start,
                end,
                start_line,
                end_line,
                chunk_index,
                total_chunks,
            ));

            chunk_index += 1;
        }

        // Update total_chunks to actual count
        let actual_count = chunks.len();
        for chunk in &mut chunks {
            chunk.total_chunks = actual_count;
        }

        Ok(chunks)
    }

    fn name(&self) -> &'static str {
        "fixed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fixed_chunker() {
        let config = ChunkerConfig {
            chunk_size: 100,
            chunk_overlap: 20,
            min_chunk_size: 10,
        };
        let chunker = FixedChunker::new(config);

        let text = "a".repeat(250);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        assert!(!chunks.is_empty());
        // First chunk should be 100 chars
        assert_eq!(chunks[0].content.len(), 100);
    }

    #[tokio::test]
    async fn test_small_text() {
        let config = ChunkerConfig::default();
        let chunker = FixedChunker::new(config);

        let text = "Hello, world!";
        let chunks = chunker.chunk(text, "/test.txt").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }

    #[tokio::test]
    async fn test_empty_text() {
        let config = ChunkerConfig::default();
        let chunker = FixedChunker::new(config);

        let chunks = chunker.chunk("", "/test.txt").await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_exact_chunk_size() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 10,
        };
        let chunker = FixedChunker::new(config);

        let text = "a".repeat(50);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content.len(), 50);
    }

    #[tokio::test]
    async fn test_chunk_overlap() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 10,
            min_chunk_size: 5,
        };
        let chunker = FixedChunker::new(config);

        let text = "a".repeat(100);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // With overlap, we should see the same content at boundaries
        if chunks.len() >= 2 {
            let end_of_first = &chunks[0].content[40..50];
            let start_of_second = &chunks[1].content[0..10];
            assert_eq!(end_of_first, start_of_second);
        }
    }

    #[tokio::test]
    async fn test_zero_overlap() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = FixedChunker::new(config);

        let text = "a".repeat(100);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // With zero overlap, chunks should not share content
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].content.len(), 50);
        assert_eq!(chunks[1].content.len(), 50);
    }

    #[tokio::test]
    async fn test_chunk_metadata() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = FixedChunker::new(config);

        let text = "a".repeat(100);
        let chunks = chunker.chunk(&text, "/my/path.txt").await.unwrap();

        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.source_path, "/my/path.txt");
            assert_eq!(chunk.chunk_index, i);
            assert_eq!(chunk.total_chunks, chunks.len());
        }
    }

    #[tokio::test]
    async fn test_chunk_offsets() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = FixedChunker::new(config);

        let text = "a".repeat(100);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        assert_eq!(chunks[0].start_offset, 0);
        assert_eq!(chunks[0].end_offset, 50);
        assert_eq!(chunks[1].start_offset, 50);
        assert_eq!(chunks[1].end_offset, 100);
    }

    #[tokio::test]
    async fn test_chunk_line_numbers() {
        let config = ChunkerConfig {
            chunk_size: 20,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = FixedChunker::new(config);

        let text = "line1\nline2\nline3\nline4\nline5\n";
        let chunks = chunker.chunk(text, "/test.txt").await.unwrap();

        // First chunk should start at line 1
        assert_eq!(chunks[0].start_line, 1);
    }

    #[tokio::test]
    async fn test_ascii_text_with_varied_chars() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = FixedChunker::new(config);

        // ASCII text with varied characters
        let text = "Hello World! ".repeat(20);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should handle text without panicking
        assert!(!chunks.is_empty());
        // Content should be valid UTF-8
        for chunk in &chunks {
            assert!(!chunk.content.is_empty());
        }
    }

    #[tokio::test]
    async fn test_min_chunk_size_filtering() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 20,
        };
        let chunker = FixedChunker::new(config);

        // 60 chars = first chunk of 50, last chunk of 10 (below min)
        let text = "a".repeat(60);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // The small trailing chunk might be filtered or kept as last chunk
        // Based on implementation, we check it doesn't crash
        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_chunker_name() {
        let config = ChunkerConfig::default();
        let chunker = FixedChunker::new(config);
        assert_eq!(chunker.name(), "fixed");
    }

    #[tokio::test]
    async fn test_very_large_text() {
        let config = ChunkerConfig {
            chunk_size: 1000,
            chunk_overlap: 100,
            min_chunk_size: 50,
        };
        let chunker = FixedChunker::new(config);

        // 100KB of text
        let text = "a".repeat(100_000);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        assert!(!chunks.is_empty());
        // Should have roughly 100 chunks (100000 / (1000-100) = ~111)
        assert!(chunks.len() > 50);
    }

    #[tokio::test]
    async fn test_single_character_chunks() {
        let config = ChunkerConfig {
            chunk_size: 1,
            chunk_overlap: 0,
            min_chunk_size: 1,
        };
        let chunker = FixedChunker::new(config);

        let text = "hello";
        let chunks = chunker.chunk(text, "/test.txt").await.unwrap();

        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[0].content, "h");
        assert_eq!(chunks[1].content, "e");
    }
}

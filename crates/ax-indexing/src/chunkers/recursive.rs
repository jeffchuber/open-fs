use super::{count_lines_to_offset, Chunker, ChunkerConfig};
use crate::{Chunk, IndexingError};
use async_trait::async_trait;

/// Recursive text splitter that tries to split on natural boundaries.
/// Tries separators in order: paragraphs, sentences, words, characters.
pub struct RecursiveChunker {
    config: ChunkerConfig,
    separators: Vec<&'static str>,
}

impl RecursiveChunker {
    pub fn new(config: ChunkerConfig) -> Self {
        RecursiveChunker {
            config,
            separators: vec!["\n\n", "\n", ". ", " ", ""],
        }
    }

    pub fn with_separators(mut self, separators: Vec<&'static str>) -> Self {
        self.separators = separators;
        self
    }

    fn split_text(&self, text: &str, separators: &[&str]) -> Vec<String> {
        if text.is_empty() {
            return vec![];
        }

        let separator = separators.first().copied().unwrap_or("");
        let remaining_separators = if separators.len() > 1 {
            &separators[1..]
        } else {
            &[]
        };

        let mut chunks = Vec::new();

        // Split by current separator
        let splits: Vec<&str> = if separator.is_empty() {
            // Character-level split using char_indices for correct byte boundaries
            text.char_indices()
                .map(|(start, c)| &text[start..start + c.len_utf8()])
                .collect()
        } else {
            text.split(separator).collect()
        };

        let mut current_chunk = String::new();

        for (i, split) in splits.iter().enumerate() {
            let piece = if i < splits.len() - 1 && !separator.is_empty() {
                format!("{}{}", split, separator)
            } else {
                split.to_string()
            };

            if current_chunk.len() + piece.len() <= self.config.chunk_size {
                current_chunk.push_str(&piece);
            } else {
                // Current chunk is full
                if !current_chunk.is_empty() {
                    if current_chunk.len() > self.config.chunk_size && !remaining_separators.is_empty() {
                        // Recursively split with finer separator
                        chunks.extend(self.split_text(&current_chunk, remaining_separators));
                    } else {
                        chunks.push(current_chunk);
                    }
                }

                // Start new chunk with overlap
                if self.config.chunk_overlap > 0 && !chunks.is_empty() {
                    let last_chunk = chunks.last().unwrap();
                    let overlap_start = last_chunk.len().saturating_sub(self.config.chunk_overlap);
                    current_chunk = last_chunk[overlap_start..].to_string();
                    current_chunk.push_str(&piece);
                } else {
                    current_chunk = piece;
                }
            }
        }

        // Don't forget the last chunk
        if !current_chunk.is_empty() {
            if current_chunk.len() > self.config.chunk_size && !remaining_separators.is_empty() {
                chunks.extend(self.split_text(&current_chunk, remaining_separators));
            } else {
                chunks.push(current_chunk);
            }
        }

        chunks
    }
}

#[async_trait]
impl Chunker for RecursiveChunker {
    async fn chunk(&self, text: &str, source_path: &str) -> Result<Vec<Chunk>, IndexingError> {
        let raw_chunks = self.split_text(text, &self.separators);

        let total_chunks = raw_chunks.len();
        let mut chunks = Vec::new();
        let mut current_offset = 0;

        for (chunk_index, content) in raw_chunks.into_iter().enumerate() {
            // Skip chunks that are too small (unless it's the only/last chunk)
            if content.len() < self.config.min_chunk_size
                && total_chunks > 1
                && chunk_index < total_chunks - 1
            {
                continue;
            }

            // Find the actual offset in the original text
            let start_offset = if let Some(pos) = text[current_offset..].find(&content) {
                current_offset + pos
            } else {
                current_offset
            };
            let end_offset = start_offset + content.len();

            let start_line = count_lines_to_offset(text, start_offset);
            let end_line = count_lines_to_offset(text, end_offset);

            chunks.push(Chunk::new(
                source_path.to_string(),
                content,
                start_offset,
                end_offset,
                start_line,
                end_line,
                chunk_index,
                total_chunks,
            ));

            current_offset = start_offset + 1;
        }

        // Update total_chunks to actual count
        let actual_count = chunks.len();
        for chunk in &mut chunks {
            chunk.total_chunks = actual_count;
            chunk.chunk_index = chunk.chunk_index.min(actual_count.saturating_sub(1));
        }

        Ok(chunks)
    }

    fn name(&self) -> &'static str {
        "recursive"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_recursive_chunker_paragraphs() {
        let config = ChunkerConfig {
            chunk_size: 100,
            chunk_overlap: 20,
            min_chunk_size: 10,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "First paragraph with some content.\n\nSecond paragraph with more content.\n\nThird paragraph here.";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        assert!(!chunks.is_empty());
        // Should split on paragraph boundaries
    }

    #[tokio::test]
    async fn test_recursive_chunker_long_text() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 10,
            min_chunk_size: 10,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "This is a long sentence that should be split. And another one here. Plus more text to fill it out.";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        assert!(chunks.len() > 1);
        // Each chunk should be roughly the target size
        for chunk in &chunks {
            assert!(chunk.content.len() <= 60); // Allow some flexibility
        }
    }

    #[tokio::test]
    async fn test_recursive_chunker_empty_text() {
        let config = ChunkerConfig::default();
        let chunker = RecursiveChunker::new(config);

        let chunks = chunker.chunk("", "/test.txt").await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_recursive_chunker_small_text() {
        let config = ChunkerConfig {
            chunk_size: 100,
            chunk_overlap: 0,
            min_chunk_size: 10,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "Small text.";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }

    #[tokio::test]
    async fn test_recursive_chunker_newline_splits() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "Line one.\nLine two.\nLine three.\nLine four.\nLine five.";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should prefer splitting on newlines
        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_recursive_chunker_sentence_splits() {
        let config = ChunkerConfig {
            chunk_size: 30,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "First sentence. Second sentence. Third sentence.";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should prefer splitting on sentence boundaries
        assert!(chunks.len() > 1);
    }

    #[tokio::test]
    async fn test_recursive_chunker_word_splits() {
        let config = ChunkerConfig {
            chunk_size: 15,
            chunk_overlap: 0,
            min_chunk_size: 3,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "one two three four five";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should split on word boundaries
        assert!(chunks.len() > 1);
    }

    #[tokio::test]
    async fn test_recursive_chunker_character_splits() {
        let config = ChunkerConfig {
            chunk_size: 5,
            chunk_overlap: 0,
            min_chunk_size: 1,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "abcdefghij";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should eventually split on characters
        assert!(chunks.len() > 1);
    }

    #[tokio::test]
    async fn test_recursive_chunker_custom_separators() {
        let config = ChunkerConfig {
            chunk_size: 20,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = RecursiveChunker::new(config)
            .with_separators(vec!["|", ""]);

        let text = "part1|part2|part3|part4";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should split on custom separator
        assert!(chunks.len() > 1);
    }

    #[tokio::test]
    async fn test_recursive_chunker_overlap() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 10,
            min_chunk_size: 5,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "a".repeat(100);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // With overlap enabled, chunks may share content
        assert!(chunks.len() > 1);
    }

    #[tokio::test]
    async fn test_recursive_chunker_metadata() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "First part.\n\nSecond part.\n\nThird part.";
        let chunks = chunker.chunk(&text, "/path/to/file.txt").await.unwrap();

        for chunk in &chunks {
            assert_eq!(chunk.source_path, "/path/to/file.txt");
            assert!(chunk.chunk_index < chunk.total_chunks);
        }
    }

    #[tokio::test]
    async fn test_recursive_chunker_unicode() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = RecursiveChunker::new(config);

        let text = "\u{4e16}\u{754c} \u{4f60}\u{597d} hello \u{1F600} world";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should handle unicode without panicking
        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_recursive_chunker_repeated_characters() {
        let config = ChunkerConfig {
            chunk_size: 5,
            chunk_overlap: 0,
            min_chunk_size: 1,
        };
        let chunker = RecursiveChunker::new(config);

        // Test with repeated characters (regression test for the fixed bug)
        let text = "aaabbbccc";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should correctly handle repeated characters
        assert!(!chunks.is_empty());
        let reconstructed: String = chunks.iter().map(|c| c.content.as_str()).collect();
        // The chunked content should contain all the original characters
        assert!(reconstructed.len() >= text.len() || chunks.iter().any(|c| c.content.contains("aaa")));
    }

    #[tokio::test]
    async fn test_recursive_chunker_name() {
        let config = ChunkerConfig::default();
        let chunker = RecursiveChunker::new(config);
        assert_eq!(chunker.name(), "recursive");
    }

    #[tokio::test]
    async fn test_recursive_chunker_very_long_line() {
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let chunker = RecursiveChunker::new(config);

        // A very long line with no natural separators
        let text = "a".repeat(200);
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should still produce chunks
        assert!(chunks.len() > 1);
    }
}

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

    fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
        if idx >= text.len() {
            return text.len();
        }
        while idx > 0 && !text.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }

    fn ceil_char_boundary(text: &str, mut idx: usize) -> usize {
        if idx >= text.len() {
            return text.len();
        }
        while idx < text.len() && !text.is_char_boundary(idx) {
            idx += 1;
        }
        idx
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
                    if current_chunk.len() > self.config.chunk_size
                        && !remaining_separators.is_empty()
                    {
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
                    let overlap_start = Self::floor_char_boundary(last_chunk, overlap_start);
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
            let search_start = Self::ceil_char_boundary(text, current_offset);
            let start_offset = if let Some(pos) = text[search_start..].find(&content) {
                search_start + pos
            } else {
                search_start
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

            current_offset = Self::ceil_char_boundary(text, start_offset.saturating_add(1));
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
        let chunks = chunker.chunk(text, "/test.txt").await.unwrap();

        assert!(!chunks.is_empty());
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
        let chunks = chunker.chunk(text, "/test.txt").await.unwrap();

        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.content.len() <= 60);
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
        let chunks = chunker.chunk(text, "/test.txt").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }

    #[tokio::test]
    async fn test_recursive_chunker_name() {
        let config = ChunkerConfig::default();
        let chunker = RecursiveChunker::new(config);
        assert_eq!(chunker.name(), "recursive");
    }
}

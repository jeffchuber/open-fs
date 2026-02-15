use super::{count_lines_to_offset, Chunker, ChunkerConfig};
use crate::{Chunk, IndexingError};
use async_trait::async_trait;

/// Semantic chunker that splits on natural content boundaries.
/// Preserves paragraphs, headers, and logical sections.
pub struct SemanticChunker {
    config: ChunkerConfig,
}

impl SemanticChunker {
    pub fn new(config: ChunkerConfig) -> Self {
        SemanticChunker { config }
    }

    /// Detect if a line is a header (markdown or text).
    fn is_header(line: &str) -> bool {
        let trimmed = line.trim();
        // Markdown headers
        if trimmed.starts_with('#') {
            return true;
        }
        // Underline-style headers (===== or -----)
        if trimmed.len() >= 3 && (trimmed.chars().all(|c| c == '=') || trimmed.chars().all(|c| c == '-')) {
            return true;
        }
        // All caps lines (likely headers)
        if trimmed.len() > 3 && trimmed.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()) {
            return true;
        }
        false
    }

    /// Split text into semantic units (paragraphs, sections).
    fn split_into_sections(&self, text: &str) -> Vec<(usize, usize, String)> {
        let mut sections = Vec::new();
        let mut current_section = String::new();
        let mut section_start = 0;
        let mut in_code_block = false;
        let mut current_offset = 0;

        for line in text.lines() {
            let line_with_newline = format!("{}\n", line);

            // Track code blocks (don't split inside them)
            if line.trim().starts_with("```") {
                in_code_block = !in_code_block;
            }

            // Check for section boundaries (only outside code blocks)
            let is_boundary = !in_code_block && (
                line.is_empty() ||  // Empty line (paragraph break)
                Self::is_header(line) ||  // Header
                line.trim().starts_with("---") ||  // Horizontal rule
                line.trim().starts_with("***")
            );

            if is_boundary && !current_section.trim().is_empty() {
                // Check if current section is getting too large
                if current_section.len() >= self.config.chunk_size {
                    // Save current section
                    sections.push((section_start, current_offset, current_section.trim().to_string()));
                    current_section = String::new();
                    section_start = current_offset;
                }
            }

            current_section.push_str(&line_with_newline);
            current_offset += line_with_newline.len();

            // Force split if section is way too large
            if current_section.len() > self.config.chunk_size * 2 {
                sections.push((section_start, current_offset, current_section.trim().to_string()));
                current_section = String::new();
                section_start = current_offset;
            }
        }

        // Don't forget the last section
        if !current_section.trim().is_empty() {
            sections.push((section_start, current_offset, current_section.trim().to_string()));
        }

        sections
    }

    /// Merge small sections together.
    fn merge_small_sections(&self, sections: Vec<(usize, usize, String)>) -> Vec<(usize, usize, String)> {
        let mut merged = Vec::new();
        let mut current: Option<(usize, usize, String)> = None;

        for (start, end, content) in sections {
            match current.take() {
                Some((curr_start, _curr_end, curr_content)) => {
                    let combined_len = curr_content.len() + content.len() + 2; // +2 for potential separator

                    if combined_len <= self.config.chunk_size {
                        // Merge sections
                        current = Some((curr_start, end, format!("{}\n\n{}", curr_content, content)));
                    } else {
                        // Save current and start new
                        if curr_content.len() >= self.config.min_chunk_size {
                            merged.push((curr_start, _curr_end, curr_content));
                        }
                        current = Some((start, end, content));
                    }
                }
                None => {
                    current = Some((start, end, content));
                }
            }
        }

        if let Some((start, end, content)) = current {
            if content.len() >= self.config.min_chunk_size || merged.is_empty() {
                merged.push((start, end, content));
            } else if let Some(last) = merged.last_mut() {
                // Merge into last chunk
                last.1 = end;
                last.2 = format!("{}\n\n{}", last.2, content);
            }
        }

        merged
    }
}

#[async_trait]
impl Chunker for SemanticChunker {
    async fn chunk(&self, text: &str, source_path: &str) -> Result<Vec<Chunk>, IndexingError> {
        let sections = self.split_into_sections(text);
        let merged = self.merge_small_sections(sections);

        let total_chunks = merged.len();
        let chunks: Vec<Chunk> = merged
            .into_iter()
            .enumerate()
            .map(|(chunk_index, (start_offset, end_offset, content))| {
                let start_line = count_lines_to_offset(text, start_offset);
                let end_line = count_lines_to_offset(text, end_offset);

                Chunk::new(
                    source_path.to_string(),
                    content,
                    start_offset,
                    end_offset,
                    start_line,
                    end_line,
                    chunk_index,
                    total_chunks,
                )
            })
            .collect();

        Ok(chunks)
    }

    fn name(&self) -> &'static str {
        "semantic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_semantic_chunker_paragraphs() {
        let config = ChunkerConfig {
            chunk_size: 200,
            chunk_overlap: 0,
            min_chunk_size: 20,
        };
        let chunker = SemanticChunker::new(config);

        let text = r#"# Introduction

This is the first paragraph with some content.

This is the second paragraph.

## Section Two

More content here in section two.
"#;
        let chunks = chunker.chunk(text, "/test.md").await.unwrap();

        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_semantic_chunker_empty_text() {
        let config = ChunkerConfig::default();
        let chunker = SemanticChunker::new(config);

        let chunks = chunker.chunk("", "/test.md").await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_semantic_chunker_name() {
        let config = ChunkerConfig::default();
        let chunker = SemanticChunker::new(config);
        assert_eq!(chunker.name(), "semantic");
    }

    #[test]
    fn test_is_header_markdown() {
        assert!(SemanticChunker::is_header("# Header"));
        assert!(SemanticChunker::is_header("## Header"));
        assert!(SemanticChunker::is_header("### Header"));
        assert!(!SemanticChunker::is_header("Not a header"));
    }

    #[test]
    fn test_is_header_underline() {
        assert!(SemanticChunker::is_header("====="));
        assert!(SemanticChunker::is_header("-----"));
        assert!(!SemanticChunker::is_header("=="));  // Too short
        assert!(!SemanticChunker::is_header("--"));  // Too short
    }

    #[test]
    fn test_is_header_all_caps() {
        assert!(SemanticChunker::is_header("INTRODUCTION"));
        assert!(SemanticChunker::is_header("METHODS"));
        assert!(!SemanticChunker::is_header("ABC"));  // Too short
        assert!(!SemanticChunker::is_header("Introduction"));  // Mixed case
    }
}

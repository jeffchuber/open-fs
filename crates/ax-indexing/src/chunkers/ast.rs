#![cfg(feature = "chunker-ast")]

use super::{count_lines_to_offset, Chunker, ChunkerConfig};
use crate::{Chunk, IndexingError};
use async_trait::async_trait;

/// AST-based chunker using tree-sitter.
/// Extracts functions, classes, and other semantic units.
pub struct AstChunker {
    config: ChunkerConfig,
}

impl AstChunker {
    pub fn new(config: ChunkerConfig) -> Self {
        AstChunker { config }
    }

    fn detect_language(path: &str) -> Option<Language> {
        let ext = path.rsplit('.').next()?.to_lowercase();
        match ext.as_str() {
            "rs" => Some(Language::Rust),
            "py" => Some(Language::Python),
            "js" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "mts" | "cts" => Some(Language::TypeScript),
            "tsx" => Some(Language::TypeScript),
            "jsx" => Some(Language::JavaScript),
            "go" => Some(Language::Go),
            _ => None,
        }
    }

    fn get_parser(lang: Language) -> Result<tree_sitter::Parser, IndexingError> {
        let mut parser = tree_sitter::Parser::new();
        let language = match lang {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
        };
        parser
            .set_language(&language)
            .map_err(|e| IndexingError::ChunkingError(format!("Failed to set language: {}", e)))?;
        Ok(parser)
    }

    fn get_chunk_node_types(lang: Language) -> Vec<&'static str> {
        match lang {
            Language::Rust => vec![
                "function_item",
                "impl_item",
                "struct_item",
                "enum_item",
                "trait_item",
                "mod_item",
                "const_item",
                "static_item",
                "type_item",
            ],
            Language::Python => vec![
                "function_definition",
                "class_definition",
                "decorated_definition",
            ],
            Language::JavaScript | Language::TypeScript => vec![
                "function_declaration",
                "class_declaration",
                "method_definition",
                "arrow_function",
                "export_statement",
            ],
            Language::Go => vec![
                "function_declaration",
                "method_declaration",
                "type_declaration",
                "const_declaration",
                "var_declaration",
            ],
        }
    }

    fn extract_chunks_from_tree(
        &self,
        text: &str,
        tree: &tree_sitter::Tree,
        lang: Language,
        source_path: &str,
    ) -> Vec<Chunk> {
        let chunk_types = Self::get_chunk_node_types(lang);
        let mut chunks = Vec::new();
        let mut cursor = tree.walk();

        self.visit_node(&mut cursor, text, &chunk_types, source_path, &mut chunks);

        // If no AST chunks found, fall back to the whole file
        if chunks.is_empty() {
            chunks.push(Chunk::new(
                source_path.to_string(),
                text.to_string(),
                0,
                text.len(),
                1,
                count_lines_to_offset(text, text.len()),
                0,
                1,
            ));
        }

        // Update total_chunks
        let total = chunks.len();
        for (i, chunk) in chunks.iter_mut().enumerate() {
            chunk.chunk_index = i;
            chunk.total_chunks = total;
        }

        chunks
    }

    fn visit_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        text: &str,
        chunk_types: &[&str],
        source_path: &str,
        chunks: &mut Vec<Chunk>,
    ) {
        let node = cursor.node();
        let node_type = node.kind();

        if chunk_types.contains(&node_type) {
            let start_byte = node.start_byte();
            let end_byte = node.end_byte();
            let content = &text[start_byte..end_byte];

            // Only create chunk if it's within size limits
            if content.len() <= self.config.chunk_size * 2 {
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;

                let mut chunk = Chunk::new(
                    source_path.to_string(),
                    content.to_string(),
                    start_byte,
                    end_byte,
                    start_line,
                    end_line,
                    chunks.len(),
                    0, // Will be updated later
                );

                // Add metadata about the AST node
                chunk.metadata.insert("node_type".to_string(), node_type.to_string());

                // Try to extract name
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = &text[name_node.start_byte()..name_node.end_byte()];
                    chunk.metadata.insert("name".to_string(), name.to_string());
                }

                chunks.push(chunk);
                return; // Don't descend into children
            }
        }

        // Visit children
        if cursor.goto_first_child() {
            loop {
                self.visit_node(cursor, text, chunk_types, source_path, chunks);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
}

#[async_trait]
impl Chunker for AstChunker {
    async fn chunk(&self, text: &str, source_path: &str) -> Result<Vec<Chunk>, IndexingError> {
        let lang = match Self::detect_language(source_path) {
            Some(l) => l,
            None => {
                // Fall back to treating as plain text
                return Ok(vec![Chunk::new(
                    source_path.to_string(),
                    text.to_string(),
                    0,
                    text.len(),
                    1,
                    count_lines_to_offset(text, text.len()),
                    0,
                    1,
                )]);
            }
        };

        let mut parser = Self::get_parser(lang)?;
        let tree = parser
            .parse(text, None)
            .ok_or_else(|| IndexingError::ChunkingError("Failed to parse source code".to_string()))?;

        Ok(self.extract_chunks_from_tree(text, &tree, lang, source_path))
    }

    fn name(&self) -> &'static str {
        "ast"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ast_chunker_rust() {
        let config = ChunkerConfig::default();
        let chunker = AstChunker::new(config);

        let text = r#"
fn hello() {
    println!("Hello");
}

fn world() {
    println!("World");
}

struct Foo {
    bar: i32,
}
"#;
        let chunks = chunker.chunk(&text, "/test.rs").await.unwrap();

        assert!(chunks.len() >= 3); // hello, world, Foo
        assert!(chunks.iter().any(|c| c.content.contains("fn hello")));
        assert!(chunks.iter().any(|c| c.content.contains("fn world")));
        assert!(chunks.iter().any(|c| c.content.contains("struct Foo")));
    }

    #[tokio::test]
    async fn test_ast_chunker_python() {
        let config = ChunkerConfig::default();
        let chunker = AstChunker::new(config);

        let text = r#"
def hello():
    print("Hello")

def world():
    print("World")

class Foo:
    def __init__(self):
        pass
"#;
        let chunks = chunker.chunk(&text, "/test.py").await.unwrap();

        assert!(chunks.len() >= 3);
        assert!(chunks.iter().any(|c| c.content.contains("def hello")));
        assert!(chunks.iter().any(|c| c.content.contains("class Foo")));
    }

    #[tokio::test]
    async fn test_ast_chunker_unknown_extension() {
        let config = ChunkerConfig::default();
        let chunker = AstChunker::new(config);

        let text = "Some plain text content";
        let chunks = chunker.chunk(&text, "/test.txt").await.unwrap();

        // Should fall back to treating as plain text
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }
}

use super::TextExtractor;
use crate::IndexingError;
use async_trait::async_trait;

/// Plain text extractor for UTF-8 text files.
pub struct PlainTextExtractor {
    /// File extensions to support (empty = all text files).
    extensions: Vec<String>,
}

impl PlainTextExtractor {
    pub fn new() -> Self {
        PlainTextExtractor {
            extensions: vec![
                // Source code
                "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp",
                "cs", "rb", "php", "swift", "kt", "scala", "clj", "ex", "exs", "erl", "hs",
                "lua", "r", "jl", "pl", "pm", "sh", "bash", "zsh", "fish", "ps1", "bat",
                // Web
                "html", "htm", "css", "scss", "sass", "less", "vue", "svelte",
                // Config
                "json", "yaml", "yml", "toml", "ini", "cfg", "conf", "env",
                // Documentation
                "md", "markdown", "txt", "rst", "adoc", "org",
                // Data
                "csv", "tsv", "xml", "sql",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }

    pub fn with_extensions(mut self, extensions: Vec<String>) -> Self {
        self.extensions = extensions;
        self
    }

    fn get_extension(path: &str) -> Option<&str> {
        path.rsplit('.').next()
    }

    fn is_likely_binary(content: &[u8]) -> bool {
        // Check first 8KB for null bytes (common in binary files)
        let check_len = content.len().min(8192);
        content[..check_len].iter().any(|&b| b == 0)
    }
}

impl Default for PlainTextExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TextExtractor for PlainTextExtractor {
    async fn extract(&self, content: &[u8], path: &str) -> Result<String, IndexingError> {
        // Check for binary content
        if Self::is_likely_binary(content) {
            return Err(IndexingError::UnsupportedFileType(format!(
                "Binary file detected: {}",
                path
            )));
        }

        // Try UTF-8 first
        match std::str::from_utf8(content) {
            Ok(text) => Ok(text.to_string()),
            Err(_) => {
                // Fall back to lossy conversion
                Ok(String::from_utf8_lossy(content).to_string())
            }
        }
    }

    fn supports(&self, path: &str) -> bool {
        if self.extensions.is_empty() {
            return true;
        }

        if let Some(ext) = Self::get_extension(path) {
            self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
        } else {
            false
        }
    }

    fn name(&self) -> &'static str {
        "plaintext"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_extract_utf8() {
        let extractor = PlainTextExtractor::new();
        let content = "Hello, world! 你好世界";
        let result = extractor.extract(content.as_bytes(), "/test.txt").await.unwrap();
        assert_eq!(result, content);
    }

    #[tokio::test]
    async fn test_supports_extensions() {
        let extractor = PlainTextExtractor::new();

        assert!(extractor.supports("/path/to/file.rs"));
        assert!(extractor.supports("/path/to/file.py"));
        assert!(extractor.supports("/path/to/file.md"));
        assert!(extractor.supports("/path/to/FILE.RS")); // Case insensitive
    }

    #[tokio::test]
    async fn test_binary_detection() {
        let extractor = PlainTextExtractor::new();

        let binary_content = vec![0x00, 0x01, 0x02, 0x03];
        let result = extractor.extract(&binary_content, "/test.bin").await;
        assert!(result.is_err());
    }
}

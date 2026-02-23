use async_trait::async_trait;
use pdf_extract::extract_text_from_mem;

use super::TextExtractor;
use crate::IndexingError;

/// PDF text extractor using pdf-extract.
pub struct PdfExtractor;

impl PdfExtractor {
    /// Create a new PDF extractor.
    pub fn new() -> Self {
        PdfExtractor
    }
}

impl Default for PdfExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TextExtractor for PdfExtractor {
    async fn extract(&self, content: &[u8], path: &str) -> Result<String, IndexingError> {
        // pdf-extract is synchronous, run it on a blocking thread
        let content = content.to_vec();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || {
            extract_text_from_mem(&content).map_err(|e| {
                IndexingError::ExtractionError(format!("PDF extraction failed for {}: {}", path, e))
            })
        })
        .await
        .map_err(|e| IndexingError::ExtractionError(format!("Task join error: {}", e)))?
    }

    fn supports(&self, path: &str) -> bool {
        let path_lower = path.to_lowercase();
        path_lower.ends_with(".pdf")
    }

    fn name(&self) -> &'static str {
        "pdf"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports() {
        let extractor = PdfExtractor::new();
        assert!(extractor.supports("document.pdf"));
        assert!(extractor.supports("path/to/file.PDF"));
        assert!(!extractor.supports("document.txt"));
        assert!(!extractor.supports("document.doc"));
    }
}

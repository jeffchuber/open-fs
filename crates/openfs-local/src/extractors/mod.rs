mod plaintext;

#[cfg(feature = "extractor-pdf")]
mod pdf;

pub use plaintext::PlainTextExtractor;

#[cfg(feature = "extractor-pdf")]
pub use pdf::PdfExtractor;

use crate::IndexingError;
use async_trait::async_trait;

/// Trait for extracting text from files.
#[async_trait]
pub trait TextExtractor: Send + Sync {
    /// Extract text from raw bytes.
    async fn extract(&self, content: &[u8], path: &str) -> Result<String, IndexingError>;

    /// Check if this extractor supports the given file.
    fn supports(&self, path: &str) -> bool;

    /// Get the extractor name.
    fn name(&self) -> &'static str;
}

/// Default text extractor that handles common text files.
pub fn default_extractor() -> Box<dyn TextExtractor> {
    Box::new(PlainTextExtractor::new())
}

/// Create a composite extractor that tries multiple extractors.
pub fn create_extractors() -> Vec<Box<dyn TextExtractor>> {
    let mut extractors: Vec<Box<dyn TextExtractor>> = Vec::new();

    #[cfg(feature = "extractor-pdf")]
    extractors.push(Box::new(PdfExtractor::new()));

    extractors.push(Box::new(PlainTextExtractor::new()));

    extractors
}

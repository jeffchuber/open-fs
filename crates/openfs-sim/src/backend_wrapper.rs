use std::sync::Arc;

use async_trait::async_trait;
use openfs_core::{Backend, BackendError, Entry};

/// Wrapper to hold `Arc<dyn Backend>` as a concrete type for `CachedBackend<B>`.
///
/// Same pattern as the private `DynBackend` in `openfs-remote/src/vfs.rs`.
/// Needed so we can share a single `Arc<MemoryBackend>` between agents
/// while still passing it as a concrete `B: Backend` to `CachedBackend::new`.
#[derive(Clone)]
pub struct DynBackend(pub Arc<dyn Backend>);

#[async_trait]
impl Backend for DynBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        self.0.read(path).await
    }
    async fn read_with_cas_token(
        &self,
        path: &str,
    ) -> Result<(Vec<u8>, Option<String>), BackendError> {
        self.0.read_with_cas_token(path).await
    }
    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        self.0.write(path, content).await
    }
    async fn compare_and_swap(
        &self,
        path: &str,
        expected: Option<&str>,
        content: &[u8],
    ) -> Result<Option<String>, BackendError> {
        self.0.compare_and_swap(path, expected, content).await
    }
    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        self.0.append(path, content).await
    }
    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        self.0.delete(path).await
    }
    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        self.0.list(path).await
    }
    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        self.0.exists(path).await
    }
    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        self.0.stat(path).await
    }
    async fn rename(&self, from: &str, to: &str) -> Result<(), BackendError> {
        self.0.rename(from, to).await
    }
}

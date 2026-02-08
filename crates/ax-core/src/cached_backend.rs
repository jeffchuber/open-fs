use std::sync::Arc;

use async_trait::async_trait;
use ax_backends::{Backend, BackendError, Entry};

use crate::cache::{CacheConfig, LruCache, CacheStats};
use crate::sync::{SyncConfig, SyncEngine, SyncMode, SyncStats};

/// A backend wrapper that adds caching and sync capabilities.
pub struct CachedBackend<B: Backend> {
    /// The underlying backend.
    inner: Arc<B>,
    /// The cache layer.
    cache: Arc<LruCache>,
    /// The sync engine.
    sync: Arc<SyncEngine>,
    /// Whether this is a read-only mount.
    read_only: bool,
}

impl<B: Backend> CachedBackend<B> {
    /// Create a new cached backend.
    pub fn new(
        inner: B,
        cache_config: CacheConfig,
        sync_config: SyncConfig,
        read_only: bool,
    ) -> Self {
        CachedBackend {
            inner: Arc::new(inner),
            cache: Arc::new(LruCache::new(cache_config)),
            sync: Arc::new(SyncEngine::new(sync_config)),
            read_only,
        }
    }

    /// Create with just caching (no sync).
    pub fn with_cache(inner: B, cache_config: CacheConfig) -> Self {
        Self::new(inner, cache_config, SyncConfig::default(), false)
    }

    /// Create with write-through sync.
    pub fn write_through(inner: B, cache_config: CacheConfig) -> Self {
        let sync_config = SyncConfig {
            mode: SyncMode::WriteThrough,
            ..Default::default()
        };
        Self::new(inner, cache_config, sync_config, false)
    }

    /// Create with write-back sync.
    pub fn write_back(inner: B, cache_config: CacheConfig, flush_interval_secs: u64) -> Self {
        let sync_config = SyncConfig {
            mode: SyncMode::WriteBack,
            flush_interval: std::time::Duration::from_secs(flush_interval_secs),
            ..Default::default()
        };
        Self::new(inner, cache_config, sync_config, false)
    }

    /// Create as read-only pull mirror.
    pub fn pull_mirror(inner: B, cache_config: CacheConfig) -> Self {
        let sync_config = SyncConfig {
            mode: SyncMode::PullMirror,
            ..Default::default()
        };
        Self::new(inner, cache_config, sync_config, true)
    }

    /// Get cache statistics.
    pub async fn cache_stats(&self) -> CacheStats {
        self.cache.stats().await
    }

    /// Get sync statistics.
    pub async fn sync_stats(&self) -> SyncStats {
        self.sync.stats().await
    }

    /// Get the sync mode.
    pub fn sync_mode(&self) -> SyncMode {
        self.sync.mode()
    }

    /// Clear the cache.
    pub async fn clear_cache(&self) {
        self.cache.clear().await;
    }

    /// Prune expired cache entries.
    pub async fn prune_cache(&self) -> usize {
        self.cache.prune_expired().await
    }

    /// Warm the cache by pre-fetching paths.
    pub async fn warm(&self, paths: &[&str]) -> Result<usize, BackendError> {
        let mut warmed = 0;
        for path in paths {
            if !self.cache.contains(path).await {
                if let Ok(content) = self.inner.read(path).await {
                    self.cache.put(path, content).await;
                    warmed += 1;
                }
            }
        }
        Ok(warmed)
    }

    /// Get a reference to the inner backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }
}

#[async_trait]
impl<B: Backend + Send + Sync + 'static> Backend for CachedBackend<B> {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        // Try cache first
        if let Some(content) = self.cache.get(path).await {
            return Ok(content);
        }

        // Cache miss - read from backend
        let content = self.inner.read(path).await?;

        // Store in cache
        self.cache.put(path, content.clone()).await;

        Ok(content)
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        if self.read_only {
            return Err(BackendError::Other(format!("Mount is read-only: {}", path)));
        }

        match self.sync.mode() {
            SyncMode::WriteThrough => {
                // Write to backend immediately
                self.inner.write(path, content).await?;
                // Update cache
                self.cache.put(path, content.to_vec()).await;
            }
            SyncMode::WriteBack => {
                // Update cache immediately
                self.cache.put(path, content.to_vec()).await;
                // Queue for background sync (ignore errors for now)
                let _ = self.sync.queue_write(path.to_string(), content.to_vec()).await;
            }
            SyncMode::None | SyncMode::PullMirror => {
                // Write to backend, update cache
                self.inner.write(path, content).await?;
                self.cache.put(path, content.to_vec()).await;
            }
        }

        Ok(())
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        if self.read_only {
            return Err(BackendError::Other(format!("Mount is read-only: {}", path)));
        }

        match self.sync.mode() {
            SyncMode::WriteThrough => {
                self.inner.append(path, content).await?;
                // Invalidate cache (we don't know the full content)
                self.cache.remove(path).await;
            }
            SyncMode::WriteBack => {
                // Read current content
                let current = self.read(path).await.unwrap_or_default();
                let mut new_content = current;
                new_content.extend_from_slice(content);

                // Update cache
                self.cache.put(path, new_content.clone()).await;
                // Queue full content for sync
                let _ = self.sync.queue_write(path.to_string(), new_content).await;
            }
            SyncMode::None | SyncMode::PullMirror => {
                self.inner.append(path, content).await?;
                self.cache.remove(path).await;
            }
        }

        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        if self.read_only {
            return Err(BackendError::Other(format!("Mount is read-only: {}", path)));
        }

        // Remove from cache
        self.cache.remove(path).await;

        match self.sync.mode() {
            SyncMode::WriteBack => {
                // Queue delete for sync
                let _ = self.sync.queue_delete(path.to_string()).await;
            }
            _ => {
                // Delete from backend immediately
                self.inner.delete(path).await?;
            }
        }

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        // Directory listings are not cached
        self.inner.list(path).await
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        // Check cache first
        if self.cache.contains(path).await {
            return Ok(true);
        }

        // Check backend
        self.inner.exists(path).await
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        // Stats are not cached
        self.inner.stat(path).await
    }
}

/// Combined status for cache and sync.
#[derive(Debug, Clone)]
pub struct CachedBackendStatus {
    pub cache: CacheStats,
    pub sync: SyncStats,
    pub sync_mode: SyncMode,
    pub read_only: bool,
}

impl<B: Backend + Send + Sync + 'static> CachedBackend<B> {
    /// Get combined status.
    pub async fn status(&self) -> CachedBackendStatus {
        CachedBackendStatus {
            cache: self.cache_stats().await,
            sync: self.sync_stats().await,
            sync_mode: self.sync_mode(),
            read_only: self.read_only,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ax_backends::MemoryBackend;

    #[tokio::test]
    async fn test_cached_backend_read_cache_hit() {
        let inner = MemoryBackend::new();
        inner.write("/test.txt", b"hello").await.unwrap();

        let cached = CachedBackend::with_cache(inner, CacheConfig::default());

        // First read - cache miss
        let content = cached.read("/test.txt").await.unwrap();
        assert_eq!(content, b"hello");

        let stats = cached.cache_stats().await;
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);

        // Second read - cache hit
        let content = cached.read("/test.txt").await.unwrap();
        assert_eq!(content, b"hello");

        let stats = cached.cache_stats().await;
        assert_eq!(stats.hits, 1);
    }

    #[tokio::test]
    async fn test_cached_backend_write_through() {
        let inner = MemoryBackend::new();
        let cached = CachedBackend::write_through(inner, CacheConfig::default());

        cached.write("/test.txt", b"hello").await.unwrap();

        // Should be in cache
        let stats = cached.cache_stats().await;
        assert_eq!(stats.entries, 1);

        // Should be in backend
        let content = cached.inner().read("/test.txt").await.unwrap();
        assert_eq!(content, b"hello");
    }

    #[tokio::test]
    async fn test_cached_backend_read_only() {
        let inner = MemoryBackend::new();
        let cached = CachedBackend::pull_mirror(inner, CacheConfig::default());

        let result = cached.write("/test.txt", b"hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cached_backend_warm() {
        let inner = MemoryBackend::new();
        inner.write("/a.txt", b"aaa").await.unwrap();
        inner.write("/b.txt", b"bbb").await.unwrap();

        let cached = CachedBackend::with_cache(inner, CacheConfig::default());

        let warmed = cached.warm(&["/a.txt", "/b.txt", "/nonexistent.txt"]).await.unwrap();
        assert_eq!(warmed, 2);

        let stats = cached.cache_stats().await;
        assert_eq!(stats.entries, 2);
    }

    #[tokio::test]
    async fn test_cached_backend_delete_invalidates_cache() {
        let inner = MemoryBackend::new();
        inner.write("/test.txt", b"hello").await.unwrap();

        let cached = CachedBackend::with_cache(inner, CacheConfig::default());

        // Warm cache
        cached.read("/test.txt").await.unwrap();
        assert!(cached.cache.contains("/test.txt").await);

        // Delete should remove from cache
        cached.delete("/test.txt").await.unwrap();
        assert!(!cached.cache.contains("/test.txt").await);
    }
}

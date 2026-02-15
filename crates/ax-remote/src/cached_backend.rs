use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ax_core::{Backend, BackendError, CacheConfig, CacheStats, Entry, LruCache, VfsError};

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

    /// Start background sync for write-back mode.
    pub async fn start_sync<F, Fut>(&self, flush_fn: F)
    where
        F: Fn(String, Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), VfsError>> + Send,
    {
        self.sync.start(flush_fn).await;
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

    /// Shutdown the sync engine, flushing any pending writes.
    pub async fn shutdown_sync(&self) {
        self.sync.shutdown().await;
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
                // Queue for background sync
                if let Err(e) = self.sync.queue_write(path.to_string(), content.to_vec()).await {
                    tracing::warn!("Failed to queue write for {}: {}. Data is cached but may not sync.", path, e);
                }
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
                if self.cache.contains(path).await {
                    if let Ok(full_content) = self.inner.read(path).await {
                        self.cache.put(path, full_content).await;
                    } else {
                        self.cache.remove(path).await;
                    }
                }
            }
            SyncMode::WriteBack => {
                let current = match self.read(path).await {
                    Ok(data) => data,
                    Err(BackendError::NotFound(_)) => Vec::new(),
                    Err(e) => return Err(e),
                };
                let mut new_content = current;
                new_content.extend_from_slice(content);

                self.cache.put(path, new_content.clone()).await;
                if let Err(e) = self.sync.queue_write(path.to_string(), new_content).await {
                    tracing::warn!("Failed to queue append for {}: {}. Data is cached but may not sync.", path, e);
                }
            }
            SyncMode::None | SyncMode::PullMirror => {
                self.inner.append(path, content).await?;
                if self.cache.contains(path).await {
                    if let Ok(full_content) = self.inner.read(path).await {
                        self.cache.put(path, full_content).await;
                    } else {
                        self.cache.remove(path).await;
                    }
                }
            }
        }

        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        if self.read_only {
            return Err(BackendError::Other(format!("Mount is read-only: {}", path)));
        }

        match self.sync.mode() {
            SyncMode::WriteBack => {
                // Serialize with any in-flight flush for this path.
                self.sync.acquire_path_lock(path).await;

                let locally_present = self.cache.contains(path).await || self.sync.pending_contains(path).await;

                let delete_result = self.inner.delete(path).await;
                match delete_result {
                    Ok(()) => {}
                    Err(e @ BackendError::NotFound(_)) => {
                        if !locally_present {
                            self.sync.release_path_lock(path).await;
                            return Err(e);
                        }
                    }
                    Err(e) => {
                        self.sync.release_path_lock(path).await;
                        return Err(e);
                    }
                }

                if let Err(e) = self.sync.queue_delete(path.to_string()).await {
                    tracing::warn!("Failed to queue delete for {}: {}. File removed from cache but may not sync.", path, e);
                }

                // Remove from cache immediately for local consistency
                self.cache.remove(path).await;

                self.sync.release_path_lock(path).await;
            }
            _ => {
                self.inner.delete(path).await?;
                self.cache.remove(path).await;
            }
        }

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let mut entries = self.inner.list(path).await?;

        if self.sync.mode() != SyncMode::WriteBack {
            return Ok(entries);
        }

        let cached_entries = self.cache.entries().await;
        if cached_entries.is_empty() {
            return Ok(entries);
        }

        let normalized = path.trim_matches('/');
        let prefix = if normalized.is_empty() {
            String::new()
        } else {
            format!("{}/", normalized)
        };

        let mut by_name: HashMap<String, Entry> =
            entries.drain(..).map(|e| (e.name.clone(), e)).collect();

        for (cached_path, size) in cached_entries {
            let cached_norm = cached_path.trim_matches('/');
            let relative = if prefix.is_empty() {
                cached_norm.to_string()
            } else if cached_norm.starts_with(&prefix) {
                cached_norm[prefix.len()..].to_string()
            } else {
                continue;
            };

            if relative.is_empty() {
                continue;
            }

            let first_component = relative.split('/').next().unwrap();

            if relative.contains('/') {
                by_name.insert(
                    first_component.to_string(),
                    Entry::dir(
                        format!(
                            "{}{}",
                            if prefix.is_empty() { "" } else { &prefix },
                            first_component
                        ),
                        first_component.to_string(),
                        None,
                    ),
                );
            } else {
                by_name.insert(
                    first_component.to_string(),
                    Entry::file(
                        format!(
                            "{}{}",
                            if prefix.is_empty() { "" } else { &prefix },
                            first_component
                        ),
                        first_component.to_string(),
                        size as u64,
                        None,
                    ),
                );
            }
        }

        let mut result: Vec<_> = by_name.into_values().collect();
        result.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(result)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        if self.cache.contains(path).await {
            return Ok(true);
        }

        if self.sync.mode() == SyncMode::WriteBack {
            let normalized = path.trim_matches('/');
            if !normalized.is_empty() {
                let dir_prefix = format!("{}/", normalized);
                if self.cache.has_prefix(&dir_prefix).await {
                    return Ok(true);
                }
            }
        }

        self.inner.exists(path).await
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        if self.sync.mode() == SyncMode::WriteBack {
            if let Some(content) = self.cache.get(path).await {
                let normalized = path.trim_matches('/');
                let name = normalized.rsplit('/').next().unwrap_or(normalized);
                return Ok(Entry::file(
                    normalized.to_string(),
                    name.to_string(),
                    content.len() as u64,
                    None,
                ));
            }

            let normalized = path.trim_matches('/');
            if !normalized.is_empty() {
                let dir_prefix = format!("{}/", normalized);
                if self.cache.has_prefix(&dir_prefix).await {
                    let name = normalized.rsplit('/').next().unwrap_or(normalized);
                    return Ok(Entry::dir(
                        normalized.to_string(),
                        name.to_string(),
                        None,
                    ));
                }
            }
        }

        self.inner.stat(path).await
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), BackendError> {
        if self.read_only {
            return Err(BackendError::Other(format!("Mount is read-only: {}", from)));
        }

        if self.sync.mode() == SyncMode::WriteBack {
            let cached_content = self.cache.get(from).await;
            let mut queued_write: Option<Vec<u8>> = cached_content.clone();
            let backend_exists = self.inner.exists(from).await?;
            if backend_exists {
                self.inner.rename(from, to).await?;
                if let Some(content) = cached_content {
                    self.cache.remove(from).await;
                    self.cache.put(to, content).await;
                } else {
                    self.cache.remove(from).await;
                }
            } else if let Some(content) = cached_content {
                self.cache.remove(from).await;
                self.cache.put(to, content.clone()).await;
                queued_write = Some(content);
            } else {
                return Err(BackendError::NotFound(from.to_string()));
            }

            if let Err(e) = self.sync.queue_delete(from.to_string()).await {
                tracing::warn!(
                    "Failed to queue delete for {} during rename: {}. Data is cached but may not sync.",
                    from,
                    e
                );
            }
            if let Err(e) = self.sync.queue_delete(to.to_string()).await {
                tracing::warn!(
                    "Failed to clear pending writes for {} during rename: {}.",
                    to,
                    e
                );
            }
            if let Some(content) = queued_write {
                if let Err(e) = self.sync.queue_write(to.to_string(), content).await {
                    tracing::warn!(
                        "Failed to queue write for {} during rename: {}. Data is cached but may not sync.",
                        to,
                        e
                    );
                }
            }

            return Ok(());
        }

        self.cache.remove(from).await;
        self.inner.rename(from, to).await?;

        Ok(())
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
    use crate::backends::MemoryBackend;

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

        // Delete should remove from cache
        cached.delete("/test.txt").await.unwrap();
    }
}

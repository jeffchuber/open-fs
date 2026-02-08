use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::{debug, trace};

/// A cache entry with content and metadata.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The cached content.
    pub content: Vec<u8>,
    /// When this entry was created.
    pub created_at: Instant,
    /// When this entry was last accessed.
    pub last_accessed: Instant,
    /// Size in bytes.
    pub size: usize,
}

impl CacheEntry {
    fn new(content: Vec<u8>) -> Self {
        let size = content.len();
        let now = Instant::now();
        CacheEntry {
            content,
            created_at: now,
            last_accessed: now,
            size,
        }
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed() > ttl
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// Configuration for the cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries in the cache.
    pub max_entries: usize,
    /// Maximum total size in bytes.
    pub max_size: usize,
    /// Time-to-live for cache entries.
    pub ttl: Duration,
    /// Whether to enable the cache.
    pub enabled: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            max_entries: 1000,
            max_size: 100 * 1024 * 1024, // 100 MB
            ttl: Duration::from_secs(300), // 5 minutes
            enabled: true,
        }
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of cache hits.
    pub hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Number of entries currently in cache.
    pub entries: usize,
    /// Total size of cached data in bytes.
    pub size: usize,
    /// Number of evictions due to size/count limits.
    pub evictions: u64,
    /// Number of entries expired by TTL.
    pub expirations: u64,
}

impl CacheStats {
    /// Calculate hit rate as a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

/// LRU cache with TTL support.
pub struct LruCache {
    config: CacheConfig,
    entries: RwLock<HashMap<String, CacheEntry>>,
    /// Order of keys for LRU eviction (most recently used at end).
    lru_order: RwLock<Vec<String>>,
    stats: RwLock<CacheStats>,
}

impl LruCache {
    /// Create a new LRU cache with the given configuration.
    pub fn new(config: CacheConfig) -> Self {
        LruCache {
            config,
            entries: RwLock::new(HashMap::new()),
            lru_order: RwLock::new(Vec::new()),
            stats: RwLock::new(CacheStats::default()),
        }
    }

    /// Get an entry from the cache.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !self.config.enabled {
            trace!(key = %key, "cache disabled, skipping lookup");
            return None;
        }

        let mut entries = self.entries.write().await;
        let mut stats = self.stats.write().await;

        if let Some(entry) = entries.get_mut(key) {
            // Check if expired
            if entry.is_expired(self.config.ttl) {
                debug!(key = %key, "cache entry expired");
                entries.remove(key);
                stats.expirations += 1;
                stats.entries = entries.len();
                stats.size = entries.values().map(|e| e.size).sum();
                stats.misses += 1;

                // Remove from LRU order
                let mut lru = self.lru_order.write().await;
                lru.retain(|k| k != key);

                return None;
            }

            // Update access time and LRU order
            entry.touch();
            stats.hits += 1;
            debug!(key = %key, size = entry.size, "cache hit");

            // Move to end of LRU order (most recently used)
            let mut lru = self.lru_order.write().await;
            lru.retain(|k| k != key);
            lru.push(key.to_string());

            Some(entry.content.clone())
        } else {
            stats.misses += 1;
            trace!(key = %key, "cache miss");
            None
        }
    }

    /// Put an entry in the cache.
    pub async fn put(&self, key: &str, content: Vec<u8>) {
        if !self.config.enabled {
            trace!(key = %key, "cache disabled, skipping put");
            return;
        }

        let entry = CacheEntry::new(content);
        let entry_size = entry.size;

        let mut entries = self.entries.write().await;
        let mut lru = self.lru_order.write().await;
        let mut stats = self.stats.write().await;

        // Remove old entry if exists
        if let Some(old) = entries.remove(key) {
            stats.size = stats.size.saturating_sub(old.size);
            lru.retain(|k| k != key);
            trace!(key = %key, "replaced existing cache entry");
        }

        // Evict entries if needed (by count)
        while entries.len() >= self.config.max_entries && !lru.is_empty() {
            if let Some(oldest_key) = lru.first().cloned() {
                if let Some(removed) = entries.remove(&oldest_key) {
                    stats.size = stats.size.saturating_sub(removed.size);
                    stats.evictions += 1;
                    debug!(key = %oldest_key, reason = "count_limit", "evicted cache entry");
                }
                lru.remove(0);
            }
        }

        // Evict entries if needed (by size)
        while stats.size + entry_size > self.config.max_size && !lru.is_empty() {
            if let Some(oldest_key) = lru.first().cloned() {
                if let Some(removed) = entries.remove(&oldest_key) {
                    stats.size = stats.size.saturating_sub(removed.size);
                    stats.evictions += 1;
                    debug!(key = %oldest_key, reason = "size_limit", "evicted cache entry");
                }
                lru.remove(0);
            }
        }

        // Insert new entry
        stats.size += entry_size;
        entries.insert(key.to_string(), entry);
        lru.push(key.to_string());
        stats.entries = entries.len();
        debug!(key = %key, size = entry_size, total_entries = stats.entries, "cached entry");
    }

    /// Remove an entry from the cache.
    pub async fn remove(&self, key: &str) -> bool {
        let mut entries = self.entries.write().await;
        let mut lru = self.lru_order.write().await;
        let mut stats = self.stats.write().await;

        if let Some(removed) = entries.remove(key) {
            stats.size = stats.size.saturating_sub(removed.size);
            stats.entries = entries.len();
            lru.retain(|k| k != key);
            true
        } else {
            false
        }
    }

    /// Clear all entries from the cache.
    pub async fn clear(&self) {
        let mut entries = self.entries.write().await;
        let mut lru = self.lru_order.write().await;
        let mut stats = self.stats.write().await;

        entries.clear();
        lru.clear();
        stats.entries = 0;
        stats.size = 0;
    }

    /// Get cache statistics.
    pub async fn stats(&self) -> CacheStats {
        self.stats.read().await.clone()
    }

    /// Check if a key exists in the cache (without updating LRU order).
    pub async fn contains(&self, key: &str) -> bool {
        let entries = self.entries.read().await;
        if let Some(entry) = entries.get(key) {
            !entry.is_expired(self.config.ttl)
        } else {
            false
        }
    }

    /// Prune expired entries.
    pub async fn prune_expired(&self) -> usize {
        let mut entries = self.entries.write().await;
        let mut lru = self.lru_order.write().await;
        let mut stats = self.stats.write().await;

        let ttl = self.config.ttl;
        let expired_keys: Vec<String> = entries
            .iter()
            .filter(|(_, entry)| entry.is_expired(ttl))
            .map(|(key, _)| key.clone())
            .collect();

        let count = expired_keys.len();

        for key in &expired_keys {
            if let Some(removed) = entries.remove(key) {
                stats.size = stats.size.saturating_sub(removed.size);
                stats.expirations += 1;
            }
            lru.retain(|k| k != key);
        }

        stats.entries = entries.len();
        count
    }

    /// Get the current configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }
}

/// Thread-safe shared cache.
pub type SharedCache = Arc<LruCache>;

/// Create a new shared cache.
pub fn create_cache(config: CacheConfig) -> SharedCache {
    Arc::new(LruCache::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_put_get() {
        let cache = LruCache::new(CacheConfig::default());

        cache.put("/test.txt", b"hello world".to_vec()).await;
        let result = cache.get("/test.txt").await;

        assert_eq!(result, Some(b"hello world".to_vec()));
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = LruCache::new(CacheConfig::default());

        let result = cache.get("/nonexistent.txt").await;
        assert!(result.is_none());

        let stats = cache.stats().await;
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn test_cache_eviction_by_count() {
        let config = CacheConfig {
            max_entries: 2,
            max_size: 1024 * 1024,
            ttl: Duration::from_secs(300),
            enabled: true,
        };
        let cache = LruCache::new(config);

        cache.put("/a.txt", b"a".to_vec()).await;
        cache.put("/b.txt", b"b".to_vec()).await;
        cache.put("/c.txt", b"c".to_vec()).await;

        // /a.txt should be evicted (LRU)
        assert!(cache.get("/a.txt").await.is_none());
        assert!(cache.get("/b.txt").await.is_some());
        assert!(cache.get("/c.txt").await.is_some());
    }

    #[tokio::test]
    async fn test_cache_ttl_expiration() {
        let config = CacheConfig {
            max_entries: 100,
            max_size: 1024 * 1024,
            ttl: Duration::from_millis(50),
            enabled: true,
        };
        let cache = LruCache::new(config);

        cache.put("/test.txt", b"hello".to_vec()).await;
        assert!(cache.get("/test.txt").await.is_some());

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(cache.get("/test.txt").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_remove() {
        let cache = LruCache::new(CacheConfig::default());

        cache.put("/test.txt", b"hello".to_vec()).await;
        assert!(cache.contains("/test.txt").await);

        cache.remove("/test.txt").await;
        assert!(!cache.contains("/test.txt").await);
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let cache = LruCache::new(CacheConfig::default());

        cache.put("/a.txt", b"aaa".to_vec()).await;
        cache.put("/b.txt", b"bbb".to_vec()).await;

        cache.get("/a.txt").await; // hit
        cache.get("/c.txt").await; // miss

        let stats = cache.stats().await;
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.size, 6);
        assert_eq!(stats.hit_rate(), 50.0);
    }

    #[tokio::test]
    async fn test_cache_disabled() {
        let config = CacheConfig {
            enabled: false,
            ..Default::default()
        };
        let cache = LruCache::new(config);

        cache.put("/test.txt", b"hello".to_vec()).await;
        assert!(cache.get("/test.txt").await.is_none());
    }
}

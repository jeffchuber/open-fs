use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use moka::future::Cache;
use tracing::{debug, trace};

use crate::path_trie::PathTrie;

/// Configuration for the cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries in the cache.
    pub max_entries: usize,
    /// Maximum total size in bytes (weighted by entry size).
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
            max_size: 100 * 1024 * 1024,   // 100 MB
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

/// Internal stats tracker for atomic updates.
struct StatsTracker {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    expirations: AtomicU64,
    size: AtomicUsize,
}

impl StatsTracker {
    fn new() -> Self {
        StatsTracker {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            expirations: AtomicU64::new(0),
            size: AtomicUsize::new(0),
        }
    }

    fn hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    fn add_size(&self, size: usize) {
        self.size.fetch_add(size, Ordering::Relaxed);
    }

    fn sub_size(&self, size: usize) {
        self.size.fetch_sub(size, Ordering::Relaxed);
    }

    fn eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    fn to_stats(&self, entry_count: u64) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries: entry_count as usize,
            size: self.size.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            expirations: self.expirations.load(Ordering::Relaxed),
        }
    }
}

/// High-performance concurrent LRU cache with TTL support.
///
/// Uses `moka` for lock-free reads and automatic eviction.
/// This is a significant improvement over the previous RwLock-based
/// implementation which required write locks on every read to update
/// LRU order.
pub struct LruCache {
    config: CacheConfig,
    /// The underlying moka cache.
    /// Uses entry size as weight for size-based eviction.
    cache: Cache<String, Vec<u8>>,
    /// Statistics tracker with atomic counters.
    stats: Arc<StatsTracker>,
    /// Track cached entry sizes by key for cheap prefix checks.
    entries: Arc<RwLock<HashMap<String, usize>>>,
    /// Trie for O(k) prefix lookups (k = path depth).
    path_trie: Arc<RwLock<PathTrie>>,
}

impl LruCache {
    /// Create a new LRU cache with the given configuration.
    pub fn new(config: CacheConfig) -> Self {
        let stats = Arc::new(StatsTracker::new());
        let stats_clone = Arc::clone(&stats);
        let entries = Arc::new(RwLock::new(HashMap::new()));
        let entries_clone = Arc::clone(&entries);
        let path_trie = Arc::new(RwLock::new(PathTrie::new()));
        let trie_clone = Arc::clone(&path_trie);

        // Build the moka cache with:
        // - max_capacity for total size (weighted by entry size)
        // - time_to_live for TTL
        // - eviction_listener to track stats
        //
        // Entry count is enforced in `put` to avoid changing LRU semantics
        // for existing code that expects a max_entries limit.
        let cache = Cache::builder()
            .max_capacity(config.max_size as u64)
            .weigher(|_key: &String, value: &Vec<u8>| value.len() as u32)
            .time_to_live(config.ttl)
            .eviction_listener(move |_key: Arc<String>, value: Vec<u8>, cause| {
                if let Ok(mut entries) = entries_clone.write() {
                    entries.remove(_key.as_ref());
                }
                if let Ok(mut trie) = trie_clone.write() {
                    trie.remove(_key.as_ref());
                }
                stats_clone.sub_size(value.len());
                match cause {
                    moka::notification::RemovalCause::Expired => {
                        stats_clone.expirations.fetch_add(1, Ordering::Relaxed);
                    }
                    moka::notification::RemovalCause::Size => {
                        stats_clone.eviction();
                    }
                    _ => {}
                }
            })
            .build();

        LruCache {
            config,
            cache,
            stats,
            entries,
            path_trie,
        }
    }

    /// Get an entry from the cache.
    ///
    /// This operation is lock-free for reads. The LRU order is updated
    /// automatically by moka without blocking other readers.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !self.config.enabled {
            trace!(key = %key, "cache disabled, skipping lookup");
            return None;
        }

        match self.cache.get(key).await {
            Some(value) => {
                self.stats.hit();
                debug!(key = %key, size = value.len(), "cache hit");
                Some(value)
            }
            None => {
                self.stats.miss();
                trace!(key = %key, "cache miss");
                None
            }
        }
    }

    /// Put an entry in the cache.
    ///
    /// If the cache is at capacity, the least recently used entries
    /// will be evicted automatically.
    pub async fn put(&self, key: &str, content: Vec<u8>) {
        if !self.config.enabled {
            trace!(key = %key, "cache disabled, skipping put");
            return;
        }

        let entry_size = content.len();

        if self.cache.entry_count() >= self.config.max_entries as u64 {
            debug!(
                key = %key,
                total_entries = self.cache.entry_count(),
                max_entries = self.config.max_entries,
                "cache entry limit reached, skipping insert"
            );
            return;
        }

        // Check if this single entry exceeds max_size
        if entry_size > self.config.max_size {
            debug!(
                key = %key,
                size = entry_size,
                max_size = self.config.max_size,
                "entry too large to cache"
            );
            return;
        }

        self.cache.insert(key.to_string(), content).await;
        if self.cache.contains_key(key) {
            self.stats.add_size(entry_size);
            if let Ok(mut entries) = self.entries.write() {
                entries.insert(key.to_string(), entry_size);
            }
            if let Ok(mut trie) = self.path_trie.write() {
                trie.insert(key);
            }
        } else {
            debug!(key = %key, "cache admission rejected");
        }
        debug!(
            key = %key,
            size = entry_size,
            total_entries = self.cache.entry_count(),
            "cached entry"
        );
    }

    /// Remove an entry from the cache.
    pub async fn remove(&self, key: &str) -> bool {
        if let Some(value) = self.cache.remove(key).await {
            self.stats.sub_size(value.len());
            if let Ok(mut entries) = self.entries.write() {
                entries.remove(key);
            }
            if let Ok(mut trie) = self.path_trie.write() {
                trie.remove(key);
            }
            true
        } else {
            false
        }
    }

    /// Clear all entries from the cache.
    pub async fn clear(&self) {
        self.cache.invalidate_all();
        // Run pending tasks to ensure eviction listeners are called
        self.cache.run_pending_tasks().await;
        // Reset size counter
        self.stats.size.store(0, Ordering::Relaxed);
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
        if let Ok(mut trie) = self.path_trie.write() {
            *trie = PathTrie::new();
        }
    }

    /// Snapshot cached entries (key + size).
    pub async fn entries(&self) -> Vec<(String, usize)> {
        if !self.config.enabled {
            return Vec::new();
        }
        if let Ok(entries) = self.entries.read() {
            return entries.iter().map(|(k, v)| (k.clone(), *v)).collect();
        }
        Vec::new()
    }

    /// Check if any cached key has the given prefix. O(k) via PathTrie.
    pub async fn has_prefix(&self, prefix: &str) -> bool {
        if !self.config.enabled {
            return false;
        }
        if let Ok(trie) = self.path_trie.read() {
            return trie.has_prefix(prefix);
        }
        false
    }

    /// List direct children of a cached prefix path.
    pub async fn list_cached_children(&self, path: &str) -> Vec<String> {
        if !self.config.enabled {
            return Vec::new();
        }
        if let Ok(trie) = self.path_trie.read() {
            return trie.list_children(path);
        }
        Vec::new()
    }

    /// Get cache statistics.
    pub async fn stats(&self) -> CacheStats {
        // Run pending tasks to ensure stats are up to date
        self.cache.run_pending_tasks().await;
        self.stats.to_stats(self.cache.entry_count())
    }

    /// Check if a key exists in the cache (without updating access time).
    pub async fn contains(&self, key: &str) -> bool {
        self.cache.contains_key(key)
    }

    /// Prune expired entries.
    ///
    /// Note: With moka, expired entries are removed automatically on access
    /// or during periodic maintenance. This method forces an immediate cleanup.
    pub async fn prune_expired(&self) -> usize {
        let before = self.cache.entry_count();
        self.cache.run_pending_tasks().await;
        let after = self.cache.entry_count();
        (before - after) as usize
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
            max_entries: 3,
            max_size: 1024 * 1024,
            ttl: Duration::from_secs(300),
            enabled: true,
        };
        let cache = LruCache::new(config);

        // Add entries up to capacity
        cache.put("/a.txt", b"a".to_vec()).await;
        cache.put("/b.txt", b"b".to_vec()).await;
        cache.put("/c.txt", b"c".to_vec()).await;

        // Verify all are present
        assert!(cache.get("/a.txt").await.is_some());
        assert!(cache.get("/b.txt").await.is_some());
        assert!(cache.get("/c.txt").await.is_some());

        // Add more entries - these should be skipped due to max_entries
        cache.put("/d.txt", b"d".to_vec()).await;
        cache.put("/e.txt", b"e".to_vec()).await;
        cache.put("/f.txt", b"f".to_vec()).await;

        // With entry limit enforced, count should not exceed max_entries
        let entry_count = cache.cache.entry_count();
        assert!(
            entry_count <= 3,
            "expected at most 3 entries, got {}",
            entry_count
        );
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

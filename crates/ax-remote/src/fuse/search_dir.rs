//! Virtual .search directory for semantic search via filesystem.
//!
//! This module implements a virtual directory that exposes semantic search
//! results as filesystem entries. Claude Code (or any tool) can search by
//! listing directories like `/.search/query/how+does+auth+work/`.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::debug;

use super::inode::{InodeAttr, InodeKind, InodeTable, VIRTUAL_INO_BASE};

/// Virtual path prefix for the search directory.
pub const SEARCH_DIR_PATH: &str = "/.search";

/// Virtual path prefix for search queries.
pub const QUERY_DIR_PATH: &str = "/.search/query";

/// A search result entry in the virtual filesystem.
#[derive(Debug, Clone)]
pub struct SearchResultEntry {
    /// Name of the entry (e.g., "01_auth.py").
    pub name: String,
    /// Virtual inode number.
    pub ino: u64,
    /// Target path for symlink (relative path to actual file).
    pub target: String,
    /// Score from semantic search.
    pub score: f32,
    /// Source file path in VFS.
    pub source_path: String,
    /// Start line in source file.
    pub start_line: usize,
    /// End line in source file.
    pub end_line: usize,
}

/// A cached search query result.
#[derive(Debug, Clone)]
struct CachedQuery {
    /// Search results.
    results: Vec<SearchResultEntry>,
    /// Virtual directory inode.
    dir_ino: u64,
    /// Timestamp when cached.
    cached_at: std::time::Instant,
}

/// Manages the virtual .search directory.
pub struct SearchDir {
    /// Inode table reference for allocating virtual inodes.
    inodes: Arc<InodeTable>,
    /// Cached search queries.
    query_cache: RwLock<HashMap<String, CachedQuery>>,
    /// Cache TTL in seconds.
    cache_ttl_secs: u64,
    /// Symlink targets by inode.
    symlink_targets: RwLock<HashMap<u64, String>>,
}

impl SearchDir {
    /// Create a new search directory manager.
    pub fn new(inodes: Arc<InodeTable>) -> Self {
        SearchDir {
            inodes,
            query_cache: RwLock::new(HashMap::new()),
            cache_ttl_secs: 60, // Cache queries for 1 minute
            symlink_targets: RwLock::new(HashMap::new()),
        }
    }

    /// Check if a path is within the virtual .search directory.
    pub fn is_search_path(path: &str) -> bool {
        path == SEARCH_DIR_PATH
            || path.starts_with(&format!("{}/", SEARCH_DIR_PATH))
    }

    /// Check if a path is the .search root directory.
    pub fn is_search_root(path: &str) -> bool {
        path == SEARCH_DIR_PATH
    }

    /// Check if a path is the query directory.
    pub fn is_query_dir(path: &str) -> bool {
        path == QUERY_DIR_PATH
    }

    /// Check if a path is a specific query (e.g., /.search/query/my+query).
    pub fn is_query_path(path: &str) -> bool {
        path.starts_with(&format!("{}/", QUERY_DIR_PATH))
            && path.len() > QUERY_DIR_PATH.len() + 1
    }

    /// Extract the query string from a query path.
    pub fn extract_query(path: &str) -> Option<String> {
        if !Self::is_query_path(path) {
            return None;
        }

        let query_part = &path[QUERY_DIR_PATH.len() + 1..];
        // Remove any trailing path components (for accessing results)
        let query_encoded = query_part.split('/').next()?;

        // URL-decode the query
        urlencoding::decode(query_encoded)
            .ok()
            .map(|s| s.into_owned())
    }

    /// Get attributes for a search path.
    pub fn getattr(&self, path: &str) -> Option<InodeAttr> {
        if Self::is_search_root(path) {
            let ino = VIRTUAL_INO_BASE;
            return Some(InodeAttr::directory(ino));
        }

        if Self::is_query_dir(path) {
            let ino = VIRTUAL_INO_BASE + 1;
            return Some(InodeAttr::directory(ino));
        }

        if Self::is_query_path(path) {
            // Check if this is a query directory or a result entry
            let parts: Vec<&str> = path[QUERY_DIR_PATH.len() + 1..].split('/').collect();

            if parts.len() == 1 {
                // Query directory itself
                let query = Self::extract_query(path)?;
                let cache = self.query_cache.read();
                if let Some(cached) = cache.get(&query) {
                    return Some(InodeAttr::directory(cached.dir_ino));
                }
                // Allocate inode for new query directory
                let ino = self.inodes.alloc_virtual_ino();
                return Some(InodeAttr::directory(ino));
            } else if parts.len() == 2 {
                // Result entry (symlink)
                let query = urlencoding::decode(parts[0]).ok()?.into_owned();
                let result_name = parts[1];

                let cache = self.query_cache.read();
                if let Some(cached) = cache.get(&query) {
                    if let Some(result) = cached.results.iter().find(|r| r.name == result_name) {
                        return Some(InodeAttr::symlink(result.ino, result.target.len() as u64));
                    }
                }
            }
        }

        None
    }

    /// List entries in a search directory.
    pub fn readdir(&self, path: &str) -> Option<Vec<(u64, String, InodeKind)>> {
        if Self::is_search_root(path) {
            // List .search/ contents: just "query"
            return Some(vec![
                (VIRTUAL_INO_BASE + 1, "query".to_string(), InodeKind::Directory),
            ]);
        }

        if Self::is_query_dir(path) {
            // List cached queries as directories
            let cache = self.query_cache.read();
            let entries: Vec<_> = cache
                .iter()
                .map(|(query, cached)| {
                    let encoded = urlencoding::encode(query).into_owned();
                    (cached.dir_ino, encoded, InodeKind::Directory)
                })
                .collect();
            return Some(entries);
        }

        if Self::is_query_path(path) {
            // List results for a specific query
            let query = Self::extract_query(path)?;
            let cache = self.query_cache.read();

            if let Some(cached) = cache.get(&query) {
                let entries: Vec<_> = cached
                    .results
                    .iter()
                    .map(|r| (r.ino, r.name.clone(), InodeKind::Symlink))
                    .collect();
                return Some(entries);
            }
        }

        None
    }

    /// Read symlink target for a search result.
    pub fn readlink(&self, ino: u64) -> Option<String> {
        let targets = self.symlink_targets.read();
        targets.get(&ino).cloned()
    }

    /// Store search results for a query.
    pub fn store_results(&self, query: &str, results: Vec<SearchResultEntry>) {
        let dir_ino = self.inodes.alloc_virtual_ino();

        // Store symlink targets
        {
            let mut targets = self.symlink_targets.write();
            for result in &results {
                targets.insert(result.ino, result.target.clone());
            }
        }

        let cached = CachedQuery {
            results,
            dir_ino,
            cached_at: std::time::Instant::now(),
        };

        let mut cache = self.query_cache.write();
        cache.insert(query.to_string(), cached);
    }

    /// Clear expired cache entries.
    pub fn cleanup_cache(&self) {
        let now = std::time::Instant::now();
        let ttl = std::time::Duration::from_secs(self.cache_ttl_secs);

        let mut cache = self.query_cache.write();
        let mut targets = self.symlink_targets.write();

        cache.retain(|_, cached| {
            let keep = now.duration_since(cached.cached_at) < ttl;
            if !keep {
                // Remove symlink targets for expired results
                for result in &cached.results {
                    targets.remove(&result.ino);
                }
            }
            keep
        });
    }

    /// Lookup an entry in a search directory.
    pub fn lookup(&self, parent_path: &str, name: &str) -> Option<(u64, InodeAttr)> {
        if Self::is_search_root(parent_path) && name == "query" {
            let ino = VIRTUAL_INO_BASE + 1;
            return Some((ino, InodeAttr::directory(ino)));
        }

        if Self::is_query_dir(parent_path) {
            // Looking up a query directory
            let query = urlencoding::decode(name).ok()?.into_owned();
            let cache = self.query_cache.read();

            if let Some(cached) = cache.get(&query) {
                return Some((cached.dir_ino, InodeAttr::directory(cached.dir_ino)));
            }

            // Query doesn't exist yet - we could trigger a search here
            // For now, return None and let the caller handle it
            debug!("Query not found in cache: {}", query);
            return None;
        }

        if Self::is_query_path(parent_path) {
            // Looking up a result in a query directory
            let query = Self::extract_query(parent_path)?;
            let cache = self.query_cache.read();

            if let Some(cached) = cache.get(&query) {
                if let Some(result) = cached.results.iter().find(|r| r.name == name) {
                    let attr = InodeAttr::symlink(result.ino, result.target.len() as u64);
                    return Some((result.ino, attr));
                }
            }
        }

        None
    }

    /// Create search result entries from search results.
    pub fn create_result_entries(
        &self,
        results: &[(String, String, f32, usize, usize)], // (source_path, content, score, start_line, end_line)
    ) -> Vec<SearchResultEntry> {
        results
            .iter()
            .enumerate()
            .map(|(i, (source_path, _content, score, start_line, end_line))| {
                // Extract filename from path
                let filename = source_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(source_path);

                // Create numbered entry name
                let name = format!("{:02}_{}", i + 1, filename);

                // Calculate relative path for symlink target
                // From /.search/query/xxx/ we need to go up to root then down to file
                let target = format!("../../..{}", source_path);

                let ino = self.inodes.alloc_virtual_ino();

                SearchResultEntry {
                    name,
                    ino,
                    target,
                    score: *score,
                    source_path: source_path.clone(),
                    start_line: *start_line,
                    end_line: *end_line,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_search_dir() -> SearchDir {
        let inodes = Arc::new(InodeTable::new());
        SearchDir::new(inodes)
    }

    // ============== Path Detection Tests ==============

    #[test]
    fn test_is_search_path() {
        assert!(SearchDir::is_search_path("/.search"));
        assert!(SearchDir::is_search_path("/.search/query"));
        assert!(SearchDir::is_search_path("/.search/query/test"));
        assert!(!SearchDir::is_search_path("/workspace"));
        assert!(!SearchDir::is_search_path("/.searchwrong"));
    }

    #[test]
    fn test_is_search_path_edge_cases() {
        // Exact match
        assert!(SearchDir::is_search_path("/.search"));

        // With subpath
        assert!(SearchDir::is_search_path("/.search/"));
        assert!(SearchDir::is_search_path("/.search/anything"));

        // Not search paths
        assert!(!SearchDir::is_search_path("/"));
        assert!(!SearchDir::is_search_path("/.searchx"));
        assert!(!SearchDir::is_search_path("/search"));
        assert!(!SearchDir::is_search_path("/.search_wrong"));
        assert!(!SearchDir::is_search_path("/workspace/.search"));
    }

    #[test]
    fn test_is_search_root() {
        assert!(SearchDir::is_search_root("/.search"));
        assert!(!SearchDir::is_search_root("/.search/"));
        assert!(!SearchDir::is_search_root("/.search/query"));
        assert!(!SearchDir::is_search_root("/"));
    }

    #[test]
    fn test_is_query_dir() {
        assert!(SearchDir::is_query_dir("/.search/query"));
        assert!(!SearchDir::is_query_dir("/.search"));
        assert!(!SearchDir::is_query_dir("/.search/query/"));
        assert!(!SearchDir::is_query_dir("/.search/query/test"));
    }

    #[test]
    fn test_is_query_path() {
        assert!(SearchDir::is_query_path("/.search/query/test"));
        assert!(SearchDir::is_query_path("/.search/query/test/result"));
        assert!(!SearchDir::is_query_path("/.search/query"));
        assert!(!SearchDir::is_query_path("/.search/query/"));
        assert!(!SearchDir::is_query_path("/.search"));
    }

    // ============== Query Extraction Tests ==============

    #[test]
    fn test_extract_query() {
        // %20 is the standard URL encoding for space in path components
        assert_eq!(
            SearchDir::extract_query("/.search/query/hello%20world"),
            Some("hello world".to_string())
        );
        assert_eq!(
            SearchDir::extract_query("/.search/query/test%20query"),
            Some("test query".to_string())
        );
        assert_eq!(
            SearchDir::extract_query("/.search/query/simple"),
            Some("simple".to_string())
        );
        // + is preserved as literal + (not decoded as space in path components)
        assert_eq!(
            SearchDir::extract_query("/.search/query/hello+world"),
            Some("hello+world".to_string())
        );
        assert_eq!(SearchDir::extract_query("/.search"), None);
        assert_eq!(SearchDir::extract_query("/.search/query"), None);
    }

    #[test]
    fn test_extract_query_special_characters() {
        // URL encoded special characters
        assert_eq!(
            SearchDir::extract_query("/.search/query/hello%2Fworld"),
            Some("hello/world".to_string())
        );
        assert_eq!(
            SearchDir::extract_query("/.search/query/test%3Fquery"),
            Some("test?query".to_string())
        );
        assert_eq!(
            SearchDir::extract_query("/.search/query/test%26query"),
            Some("test&query".to_string())
        );
        assert_eq!(
            SearchDir::extract_query("/.search/query/test%3Dvalue"),
            Some("test=value".to_string())
        );
    }

    #[test]
    fn test_extract_query_unicode() {
        // URL encoded unicode
        assert_eq!(
            SearchDir::extract_query("/.search/query/%E4%B8%AD%E6%96%87"),
            Some("\u{4e2d}\u{6587}".to_string())
        );
    }

    #[test]
    fn test_extract_query_with_result_path() {
        // Should extract just the query part, not the result filename
        assert_eq!(
            SearchDir::extract_query("/.search/query/myquery/01_file.py"),
            Some("myquery".to_string())
        );
        assert_eq!(
            SearchDir::extract_query("/.search/query/test%20query/result.txt"),
            Some("test query".to_string())
        );
    }

    #[test]
    fn test_extract_query_empty() {
        // Empty query should return None (path length check)
        assert_eq!(SearchDir::extract_query("/.search/query/"), None);
    }

    // ============== Getattr Tests ==============

    #[test]
    fn test_getattr_search_root() {
        let search_dir = create_search_dir();

        let attr = search_dir.getattr("/.search").unwrap();
        assert_eq!(attr.kind, InodeKind::Directory);
        assert_eq!(attr.ino, VIRTUAL_INO_BASE);
    }

    #[test]
    fn test_getattr_query_dir() {
        let search_dir = create_search_dir();

        let attr = search_dir.getattr("/.search/query").unwrap();
        assert_eq!(attr.kind, InodeKind::Directory);
        assert_eq!(attr.ino, VIRTUAL_INO_BASE + 1);
    }

    #[test]
    fn test_getattr_query_path_new() {
        let search_dir = create_search_dir();

        // New query (not cached) should still return attr
        let attr = search_dir.getattr("/.search/query/newquery").unwrap();
        assert_eq!(attr.kind, InodeKind::Directory);
    }

    #[test]
    fn test_getattr_query_path_cached() {
        let search_dir = create_search_dir();

        // Store some results first
        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("cached", entries);

        // Should find the cached query
        let attr = search_dir.getattr("/.search/query/cached").unwrap();
        assert_eq!(attr.kind, InodeKind::Directory);
    }

    #[test]
    fn test_getattr_result_entry() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("myquery", entries);

        // Get attr for the result entry
        let attr = search_dir.getattr("/.search/query/myquery/01_test.py").unwrap();
        assert_eq!(attr.kind, InodeKind::Symlink);
    }

    #[test]
    fn test_getattr_nonexistent_result() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("myquery", entries);

        // Nonexistent result file
        let attr = search_dir.getattr("/.search/query/myquery/99_nonexistent.py");
        assert!(attr.is_none());
    }

    #[test]
    fn test_getattr_invalid_path() {
        let search_dir = create_search_dir();

        assert!(search_dir.getattr("/workspace").is_none());
        assert!(search_dir.getattr("/").is_none());
    }

    // ============== Readdir Tests ==============

    #[test]
    fn test_readdir_search_root() {
        let search_dir = create_search_dir();

        let entries = search_dir.readdir("/.search").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "query");
        assert_eq!(entries[0].2, InodeKind::Directory);
    }

    #[test]
    fn test_readdir_query_dir_empty() {
        let search_dir = create_search_dir();

        let entries = search_dir.readdir("/.search/query").unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_readdir_query_dir_with_cached_queries() {
        let search_dir = create_search_dir();

        // Store multiple queries
        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];

        let entries1 = search_dir.create_result_entries(&results);
        search_dir.store_results("query1", entries1);

        let entries2 = search_dir.create_result_entries(&results);
        search_dir.store_results("query2", entries2);

        let dir_entries = search_dir.readdir("/.search/query").unwrap();
        assert_eq!(dir_entries.len(), 2);

        let names: Vec<_> = dir_entries.iter().map(|(_, name, _)| name.as_str()).collect();
        assert!(names.contains(&"query1"));
        assert!(names.contains(&"query2"));
    }

    #[test]
    fn test_readdir_query_results() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/workspace/auth.py".to_string(), "auth code".to_string(), 0.95, 10, 20),
            ("/workspace/login.py".to_string(), "login code".to_string(), 0.85, 5, 15),
            ("/workspace/user.py".to_string(), "user code".to_string(), 0.75, 1, 5),
        ];

        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("auth", entries);

        let dir_entries = search_dir.readdir("/.search/query/auth").unwrap();
        assert_eq!(dir_entries.len(), 3);

        // All should be symlinks
        for (_, _, kind) in &dir_entries {
            assert_eq!(*kind, InodeKind::Symlink);
        }

        let names: Vec<_> = dir_entries.iter().map(|(_, name, _)| name.as_str()).collect();
        assert!(names.contains(&"01_auth.py"));
        assert!(names.contains(&"02_login.py"));
        assert!(names.contains(&"03_user.py"));
    }

    #[test]
    fn test_readdir_uncached_query() {
        let search_dir = create_search_dir();

        // Query that was never stored
        let entries = search_dir.readdir("/.search/query/nonexistent");
        assert!(entries.is_none());
    }

    #[test]
    fn test_readdir_invalid_path() {
        let search_dir = create_search_dir();

        assert!(search_dir.readdir("/workspace").is_none());
        assert!(search_dir.readdir("/").is_none());
    }

    // ============== Store and Retrieve Tests ==============

    #[test]
    fn test_store_and_retrieve_results() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/workspace/auth.py".to_string(), "auth code".to_string(), 0.95, 10, 20),
            ("/workspace/login.py".to_string(), "login code".to_string(), 0.85, 5, 15),
        ];

        let entries = search_dir.create_result_entries(&results);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "01_auth.py");
        assert_eq!(entries[1].name, "02_login.py");

        search_dir.store_results("authentication", entries);

        // Should be able to list the query results
        let dir_entries = search_dir.readdir("/.search/query/authentication").unwrap();
        assert_eq!(dir_entries.len(), 2);
    }

    #[test]
    fn test_store_results_overwrites() {
        let search_dir = create_search_dir();

        // First store
        let results1 = vec![
            ("/file1.py".to_string(), "content".to_string(), 0.9, 1, 10),
        ];
        let entries1 = search_dir.create_result_entries(&results1);
        search_dir.store_results("query", entries1);

        // Second store with same query
        let results2 = vec![
            ("/file2.py".to_string(), "content".to_string(), 0.8, 1, 10),
            ("/file3.py".to_string(), "content".to_string(), 0.7, 1, 10),
        ];
        let entries2 = search_dir.create_result_entries(&results2);
        search_dir.store_results("query", entries2);

        // Should have new results
        let dir_entries = search_dir.readdir("/.search/query/query").unwrap();
        assert_eq!(dir_entries.len(), 2);

        let names: Vec<_> = dir_entries.iter().map(|(_, name, _)| name.as_str()).collect();
        assert!(names.contains(&"01_file2.py"));
        assert!(names.contains(&"02_file3.py"));
    }

    #[test]
    fn test_store_empty_results() {
        let search_dir = create_search_dir();

        let entries: Vec<SearchResultEntry> = vec![];
        search_dir.store_results("empty", entries);

        let dir_entries = search_dir.readdir("/.search/query/empty").unwrap();
        assert_eq!(dir_entries.len(), 0);
    }

    // ============== Lookup Tests ==============

    #[test]
    fn test_lookup_result() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/workspace/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("test", entries);

        // Lookup the result
        let result = search_dir.lookup("/.search/query/test", "01_test.py");
        assert!(result.is_some());

        let (ino, attr) = result.unwrap();
        assert_eq!(attr.kind, InodeKind::Symlink);

        // Check symlink target
        let target = search_dir.readlink(ino).unwrap();
        assert!(target.contains("/workspace/test.py"));
    }

    #[test]
    fn test_lookup_query_in_search_root() {
        let search_dir = create_search_dir();

        let result = search_dir.lookup("/.search", "query");
        assert!(result.is_some());

        let (ino, attr) = result.unwrap();
        assert_eq!(ino, VIRTUAL_INO_BASE + 1);
        assert_eq!(attr.kind, InodeKind::Directory);
    }

    #[test]
    fn test_lookup_query_directory() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("myquery", entries);

        let result = search_dir.lookup("/.search/query", "myquery");
        assert!(result.is_some());

        let (_, attr) = result.unwrap();
        assert_eq!(attr.kind, InodeKind::Directory);
    }

    #[test]
    fn test_lookup_nonexistent_query() {
        let search_dir = create_search_dir();

        let result = search_dir.lookup("/.search/query", "nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_lookup_nonexistent_result() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("myquery", entries);

        let result = search_dir.lookup("/.search/query/myquery", "nonexistent.py");
        assert!(result.is_none());
    }

    #[test]
    fn test_lookup_invalid_parent() {
        let search_dir = create_search_dir();

        let result = search_dir.lookup("/workspace", "file.txt");
        assert!(result.is_none());
    }

    // ============== Readlink Tests ==============

    #[test]
    fn test_readlink() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/workspace/deep/path/file.py".to_string(), "content".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        let ino = entries[0].ino;
        search_dir.store_results("test", entries);

        let target = search_dir.readlink(ino).unwrap();
        assert!(target.contains("/workspace/deep/path/file.py"));
        assert!(target.starts_with("../../.."));
    }

    #[test]
    fn test_readlink_nonexistent() {
        let search_dir = create_search_dir();

        let target = search_dir.readlink(99999);
        assert!(target.is_none());
    }

    // ============== Create Result Entries Tests ==============

    #[test]
    fn test_create_result_entries_basic() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/workspace/auth.py".to_string(), "auth code".to_string(), 0.95, 10, 20),
            ("/workspace/login.py".to_string(), "login code".to_string(), 0.85, 5, 15),
        ];

        let entries = search_dir.create_result_entries(&results);

        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].name, "01_auth.py");
        assert_eq!(entries[0].score, 0.95);
        assert_eq!(entries[0].source_path, "/workspace/auth.py");
        assert_eq!(entries[0].start_line, 10);
        assert_eq!(entries[0].end_line, 20);

        assert_eq!(entries[1].name, "02_login.py");
        assert_eq!(entries[1].score, 0.85);
        assert_eq!(entries[1].source_path, "/workspace/login.py");
        assert_eq!(entries[1].start_line, 5);
        assert_eq!(entries[1].end_line, 15);
    }

    #[test]
    fn test_create_result_entries_unique_inodes() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/file1.py".to_string(), "content".to_string(), 0.9, 1, 10),
            ("/file2.py".to_string(), "content".to_string(), 0.8, 1, 10),
            ("/file3.py".to_string(), "content".to_string(), 0.7, 1, 10),
        ];

        let entries = search_dir.create_result_entries(&results);

        let inodes: Vec<_> = entries.iter().map(|e| e.ino).collect();
        let unique_inodes: std::collections::HashSet<_> = inodes.iter().collect();

        assert_eq!(inodes.len(), unique_inodes.len());
    }

    #[test]
    fn test_create_result_entries_target_format() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/workspace/src/main.py".to_string(), "content".to_string(), 0.9, 1, 10),
        ];

        let entries = search_dir.create_result_entries(&results);

        // Target should be relative path going up then down
        assert_eq!(entries[0].target, "../../../workspace/src/main.py");
    }

    #[test]
    fn test_create_result_entries_numbering() {
        let search_dir = create_search_dir();

        let mut results = Vec::new();
        for i in 0..15 {
            results.push((
                format!("/file{}.py", i),
                "content".to_string(),
                0.9 - (i as f32 * 0.01),
                1,
                10,
            ));
        }

        let entries = search_dir.create_result_entries(&results);

        assert_eq!(entries[0].name, "01_file0.py");
        assert_eq!(entries[9].name, "10_file9.py");
        assert_eq!(entries[14].name, "15_file14.py");
    }

    #[test]
    fn test_create_result_entries_empty() {
        let search_dir = create_search_dir();

        let results: Vec<(String, String, f32, usize, usize)> = vec![];
        let entries = search_dir.create_result_entries(&results);

        assert!(entries.is_empty());
    }

    #[test]
    fn test_create_result_entries_deep_path() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/a/b/c/d/e/f/g/file.py".to_string(), "content".to_string(), 0.9, 1, 10),
        ];

        let entries = search_dir.create_result_entries(&results);

        assert_eq!(entries[0].name, "01_file.py");
        assert!(entries[0].target.ends_with("/a/b/c/d/e/f/g/file.py"));
    }

    // ============== Cache Cleanup Tests ==============

    #[test]
    fn test_cleanup_cache_removes_old_entries() {
        let inodes = Arc::new(InodeTable::new());
        let mut search_dir = SearchDir::new(inodes);
        search_dir.cache_ttl_secs = 0; // Immediate expiry

        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        let ino = entries[0].ino;
        search_dir.store_results("test", entries);

        // Wait a tiny bit
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Cleanup should remove the entry
        search_dir.cleanup_cache();

        // Query should be gone
        assert!(search_dir.readdir("/.search/query/test").is_none());

        // Symlink target should also be gone
        assert!(search_dir.readlink(ino).is_none());
    }

    #[test]
    fn test_cleanup_cache_keeps_fresh_entries() {
        let inodes = Arc::new(InodeTable::new());
        let mut search_dir = SearchDir::new(inodes);
        search_dir.cache_ttl_secs = 3600; // Long TTL

        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("test", entries);

        // Cleanup should NOT remove the entry
        search_dir.cleanup_cache();

        // Query should still be there
        let dir_entries = search_dir.readdir("/.search/query/test").unwrap();
        assert_eq!(dir_entries.len(), 1);
    }

    // ============== Multiple Queries Tests ==============

    #[test]
    fn test_multiple_independent_queries() {
        let search_dir = create_search_dir();

        let results1 = vec![
            ("/auth.py".to_string(), "auth".to_string(), 0.9, 1, 10),
        ];
        let results2 = vec![
            ("/login.py".to_string(), "login".to_string(), 0.8, 1, 10),
        ];
        let results3 = vec![
            ("/user.py".to_string(), "user".to_string(), 0.7, 1, 10),
        ];

        search_dir.store_results("auth", search_dir.create_result_entries(&results1));
        search_dir.store_results("login", search_dir.create_result_entries(&results2));
        search_dir.store_results("user", search_dir.create_result_entries(&results3));

        // Each query should have its own results
        assert_eq!(search_dir.readdir("/.search/query/auth").unwrap().len(), 1);
        assert_eq!(search_dir.readdir("/.search/query/login").unwrap().len(), 1);
        assert_eq!(search_dir.readdir("/.search/query/user").unwrap().len(), 1);

        // Check that results are correct for each query
        let auth_entries = search_dir.readdir("/.search/query/auth").unwrap();
        assert_eq!(auth_entries[0].1, "01_auth.py");

        let login_entries = search_dir.readdir("/.search/query/login").unwrap();
        assert_eq!(login_entries[0].1, "01_login.py");
    }

    // ============== Edge Cases ==============

    #[test]
    fn test_url_encoded_query_name() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/test.py".to_string(), "test".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results("hello world", entries);

        // Query dir listing should show URL-encoded name
        let dir_entries = search_dir.readdir("/.search/query").unwrap();
        assert_eq!(dir_entries.len(), 1);
        assert_eq!(dir_entries[0].1, "hello%20world");
    }

    #[test]
    fn test_file_without_extension() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/Makefile".to_string(), "content".to_string(), 0.9, 1, 10),
            ("/README".to_string(), "content".to_string(), 0.8, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);

        assert_eq!(entries[0].name, "01_Makefile");
        assert_eq!(entries[1].name, "02_README");
    }

    #[test]
    fn test_root_level_file() {
        let search_dir = create_search_dir();

        let results = vec![
            ("/root_file.py".to_string(), "content".to_string(), 0.9, 1, 10),
        ];
        let entries = search_dir.create_result_entries(&results);

        assert_eq!(entries[0].name, "01_root_file.py");
        assert!(entries[0].target.ends_with("/root_file.py"));
    }
}

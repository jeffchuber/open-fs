//! Virtual .search directory for grep-style query results.
//!
//! Query directories expose a single `results.txt` file containing concatenated
//! matches. This keeps search behavior simple and file-oriented for FUSE clients.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::inode::{InodeAttr, InodeKind, InodeTable, VIRTUAL_INO_BASE};

/// Virtual path prefix for the search directory.
pub const SEARCH_DIR_PATH: &str = "/.search";

/// Virtual path prefix for search queries.
pub const QUERY_DIR_PATH: &str = "/.search/query";

const RESULTS_FILE_NAME: &str = "results.txt";

/// A search result entry in the virtual filesystem.
#[derive(Debug, Clone)]
pub struct SearchResultEntry {
    pub name: String,
    pub ino: u64,
    pub content: String,
    pub score: f32,
    pub source_path: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone)]
struct CachedQuery {
    results: Vec<SearchResultEntry>,
    dir_ino: u64,
    cached_at: std::time::Instant,
}

/// Manages the virtual .search directory.
pub struct SearchDir {
    inodes: Arc<InodeTable>,
    query_cache: RwLock<HashMap<String, CachedQuery>>,
    cache_ttl_secs: u64,
}

impl SearchDir {
    /// Create a new search directory manager.
    pub fn new(inodes: Arc<InodeTable>) -> Self {
        SearchDir {
            inodes,
            query_cache: RwLock::new(HashMap::new()),
            cache_ttl_secs: 60,
        }
    }

    pub fn is_search_path(path: &str) -> bool {
        path == SEARCH_DIR_PATH || path.starts_with(&format!("{}/", SEARCH_DIR_PATH))
    }

    pub fn is_search_root(path: &str) -> bool {
        path == SEARCH_DIR_PATH
    }

    pub fn is_query_dir(path: &str) -> bool {
        path == QUERY_DIR_PATH
    }

    /// Query path is `/.search/query/<url-encoded-query>`.
    pub fn is_query_path(path: &str) -> bool {
        path.starts_with(&format!("{}/", QUERY_DIR_PATH)) && path.len() > QUERY_DIR_PATH.len() + 1
    }

    /// Extract the query string from either:
    /// - `/.search/query/<query>`
    /// - `/.search/query/<query>/<file>`
    pub fn extract_query(path: &str) -> Option<String> {
        if !Self::is_query_path(path) {
            return None;
        }

        let query_part = &path[QUERY_DIR_PATH.len() + 1..];
        let query_encoded = query_part.split('/').next()?;
        urlencoding::decode(query_encoded)
            .ok()
            .map(|s| s.into_owned())
    }

    /// Whether query content is currently cached.
    pub fn has_query(&self, query: &str) -> bool {
        self.query_cache.read().contains_key(query)
    }

    fn ensure_query_dir(&self, query: &str) -> u64 {
        if let Some(cached) = self.query_cache.read().get(query) {
            return cached.dir_ino;
        }

        let dir_ino = self.inodes.alloc_virtual_ino();
        self.query_cache.write().insert(
            query.to_string(),
            CachedQuery {
                results: Vec::new(),
                dir_ino,
                cached_at: std::time::Instant::now(),
            },
        );
        dir_ino
    }

    /// Get attributes for a search path.
    pub fn getattr(&self, path: &str) -> Option<InodeAttr> {
        if Self::is_search_root(path) {
            return Some(InodeAttr::directory(VIRTUAL_INO_BASE));
        }

        if Self::is_query_dir(path) {
            return Some(InodeAttr::directory(VIRTUAL_INO_BASE + 1));
        }

        if Self::is_query_path(path) {
            let parts: Vec<&str> = path[QUERY_DIR_PATH.len() + 1..].split('/').collect();
            if parts.len() == 1 {
                let query = Self::extract_query(path)?;
                let ino = self.ensure_query_dir(&query);
                return Some(InodeAttr::directory(ino));
            }

            if parts.len() == 2 {
                let query = urlencoding::decode(parts[0]).ok()?.into_owned();
                let name = parts[1];
                let cache = self.query_cache.read();
                let cached = cache.get(&query)?;
                let result = cached.results.iter().find(|r| r.name == name)?;
                return Some(InodeAttr::file(result.ino, result.content.len() as u64));
            }
        }

        None
    }

    /// List entries in a search directory.
    pub fn readdir(&self, path: &str) -> Option<Vec<(u64, String, InodeKind)>> {
        if Self::is_search_root(path) {
            return Some(vec![(
                VIRTUAL_INO_BASE + 1,
                "query".to_string(),
                InodeKind::Directory,
            )]);
        }

        if Self::is_query_dir(path) {
            let cache = self.query_cache.read();
            let entries = cache
                .iter()
                .map(|(query, cached)| {
                    let encoded = urlencoding::encode(query).into_owned();
                    (cached.dir_ino, encoded, InodeKind::Directory)
                })
                .collect();
            return Some(entries);
        }

        if Self::is_query_path(path) {
            let query = Self::extract_query(path)?;
            let cache = self.query_cache.read();
            let cached = cache.get(&query)?;
            let entries = cached
                .results
                .iter()
                .map(|r| (r.ino, r.name.clone(), InodeKind::File))
                .collect();
            return Some(entries);
        }

        None
    }

    /// This search model does not expose symlinks.
    pub fn readlink(&self, _ino: u64) -> Option<String> {
        None
    }

    /// Read virtual file content for `/.search/query/<query>/results.txt`.
    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        if !Self::is_query_path(path) {
            return None;
        }

        let parts: Vec<&str> = path[QUERY_DIR_PATH.len() + 1..].split('/').collect();
        if parts.len() != 2 {
            return None;
        }

        let query = urlencoding::decode(parts[0]).ok()?.into_owned();
        let name = parts[1];
        let cache = self.query_cache.read();
        let cached = cache.get(&query)?;
        let result = cached.results.iter().find(|r| r.name == name)?;
        Some(result.content.as_bytes().to_vec())
    }

    /// Store query results, replacing any previous results for this query.
    pub fn store_results(&self, query: &str, results: Vec<SearchResultEntry>) {
        let dir_ino = if let Some(existing) = self.query_cache.read().get(query) {
            existing.dir_ino
        } else {
            self.inodes.alloc_virtual_ino()
        };

        self.query_cache.write().insert(
            query.to_string(),
            CachedQuery {
                results,
                dir_ino,
                cached_at: std::time::Instant::now(),
            },
        );
    }

    /// Clear expired cache entries.
    pub fn cleanup_cache(&self) {
        let now = std::time::Instant::now();
        let ttl = std::time::Duration::from_secs(self.cache_ttl_secs);

        self.query_cache
            .write()
            .retain(|_, cached| now.duration_since(cached.cached_at) < ttl);
    }

    /// Lookup an entry in a search directory.
    pub fn lookup(&self, parent_path: &str, name: &str) -> Option<(u64, InodeAttr)> {
        if Self::is_search_root(parent_path) && name == "query" {
            let ino = VIRTUAL_INO_BASE + 1;
            return Some((ino, InodeAttr::directory(ino)));
        }

        if Self::is_query_dir(parent_path) {
            let query = urlencoding::decode(name).ok()?.into_owned();
            let ino = self.ensure_query_dir(&query);
            return Some((ino, InodeAttr::directory(ino)));
        }

        if Self::is_query_path(parent_path) {
            let query = Self::extract_query(parent_path)?;
            let cache = self.query_cache.read();
            let cached = cache.get(&query)?;
            let result = cached.results.iter().find(|r| r.name == name)?;
            let attr = InodeAttr::file(result.ino, result.content.len() as u64);
            return Some((result.ino, attr));
        }

        None
    }

    /// Build one concatenated results file for a query.
    pub fn create_result_entries(
        &self,
        results: &[(String, String, f32, usize, usize)], // (source_path, content, score, start_line, end_line)
    ) -> Vec<SearchResultEntry> {
        let content = if results.is_empty() {
            "No results.\n".to_string()
        } else {
            let mut out = String::new();
            for (i, (source_path, snippet, score, start_line, end_line)) in
                results.iter().enumerate()
            {
                if i > 0 {
                    out.push_str("\n\n");
                }
                out.push_str(&format!(
                    "{}:{}-{} [score={:.4}]\n{}",
                    source_path, start_line, end_line, score, snippet
                ));
            }
            out
        };

        vec![SearchResultEntry {
            name: RESULTS_FILE_NAME.to_string(),
            ino: self.inodes.alloc_virtual_ino(),
            content,
            score: 0.0,
            source_path: String::new(),
            start_line: 0,
            end_line: 0,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_search_dir() -> SearchDir {
        let inodes = Arc::new(InodeTable::new());
        SearchDir::new(inodes)
    }

    #[test]
    fn test_query_dir_and_results_file() {
        let search_dir = create_search_dir();

        let entries = search_dir.create_result_entries(&[(
            "/workspace/a.txt".to_string(),
            "hello world".to_string(),
            1.0,
            3,
            3,
        )]);
        search_dir.store_results("hello", entries);

        let query_entries = search_dir.readdir("/.search/query/hello").unwrap();
        assert_eq!(query_entries.len(), 1);
        assert_eq!(query_entries[0].1, "results.txt");

        let bytes = search_dir
            .read_file("/.search/query/hello/results.txt")
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("/workspace/a.txt:3-3"));
        assert!(text.contains("hello world"));
    }
}

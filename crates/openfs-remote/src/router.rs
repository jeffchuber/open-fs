use std::sync::Arc;

use openfs_core::{Backend, VfsError};

/// Mount information for routing.
pub struct Mount {
    pub path: String,
    pub backend: Arc<dyn Backend>,
    pub read_only: bool,
}

/// Router that dispatches paths to the appropriate backend.
pub struct Router {
    /// Mounts sorted by path length (longest first) for longest-prefix matching.
    mounts: Vec<Mount>,
}

impl Router {
    /// Create a new router with the given mounts.
    pub fn new(mut mounts: Vec<Mount>) -> Self {
        // Sort by path length descending for longest-prefix matching
        mounts.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        Router { mounts }
    }

    /// Resolve a path to its backend and relative path.
    /// Returns (backend, relative_path, read_only).
    pub fn resolve(&self, path: &str) -> Result<(&dyn Backend, String, bool), VfsError> {
        let normalized = normalize_path(path);

        for mount in &self.mounts {
            if let Some(relative) = strip_mount_prefix(&normalized, &mount.path) {
                return Ok((mount.backend.as_ref(), relative, mount.read_only));
            }
        }

        Err(VfsError::NoMount(path.to_string()))
    }

    /// Get the mount for a path (for checking read-only status, etc.).
    #[allow(dead_code)]
    pub fn get_mount(&self, path: &str) -> Option<&Mount> {
        let normalized = normalize_path(path);
        self.mounts
            .iter()
            .find(|mount| strip_mount_prefix(&normalized, &mount.path).is_some())
    }
}

/// Normalize a path by ensuring it starts with / and has no trailing slash.
fn normalize_path(path: &str) -> String {
    let mut normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    // Remove trailing slash unless it's the root
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }

    normalized
}

/// Strip the mount prefix from a path and return the relative path.
fn strip_mount_prefix(path: &str, mount_path: &str) -> Option<String> {
    let mount_normalized = mount_path.trim_end_matches('/');

    if path == mount_normalized {
        // Exact match - return empty path (root of mount)
        return Some(String::new());
    }

    if let Some(suffix) = path.strip_prefix(mount_normalized) {
        if let Some(relative) = suffix.strip_prefix('/') {
            // Path is under this mount
            return Some(relative.to_string());
        }
    }

    // Special case: root mount "/"
    if mount_normalized.is_empty() || mount_normalized == "/" {
        return Some(path.trim_start_matches('/').to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use openfs_core::{BackendError, Entry};

    struct MockBackend;

    #[async_trait]
    impl Backend for MockBackend {
        async fn read(&self, _path: &str) -> Result<Vec<u8>, BackendError> {
            Ok(vec![])
        }
        async fn write(&self, _path: &str, _content: &[u8]) -> Result<(), BackendError> {
            Ok(())
        }
        async fn append(&self, _path: &str, _content: &[u8]) -> Result<(), BackendError> {
            Ok(())
        }
        async fn delete(&self, _path: &str) -> Result<(), BackendError> {
            Ok(())
        }
        async fn list(&self, _path: &str) -> Result<Vec<Entry>, BackendError> {
            Ok(vec![])
        }
        async fn exists(&self, _path: &str) -> Result<bool, BackendError> {
            Ok(true)
        }
        async fn stat(&self, _path: &str) -> Result<Entry, BackendError> {
            Ok(Entry::file(String::new(), String::new(), 0, None))
        }
        async fn rename(&self, _from: &str, _to: &str) -> Result<(), BackendError> {
            Ok(())
        }
    }

    #[test]
    fn test_longest_prefix_matching() {
        let router = Router::new(vec![
            Mount {
                path: "/".to_string(),
                backend: Arc::new(MockBackend),
                read_only: false,
            },
            Mount {
                path: "/workspace".to_string(),
                backend: Arc::new(MockBackend),
                read_only: false,
            },
        ]);

        // /workspace/file.txt should match /workspace mount
        let (_, relative, _) = router.resolve("/workspace/file.txt").unwrap();
        assert_eq!(relative, "file.txt");

        // /other/file.txt should match / mount
        let (_, relative, _) = router.resolve("/other/file.txt").unwrap();
        assert_eq!(relative, "other/file.txt");
    }

    #[test]
    fn test_exact_mount_match() {
        let router = Router::new(vec![Mount {
            path: "/workspace".to_string(),
            backend: Arc::new(MockBackend),
            read_only: false,
        }]);

        let (_, relative, _) = router.resolve("/workspace").unwrap();
        assert_eq!(relative, "");
    }

    #[test]
    fn test_no_mount_found() {
        let router = Router::new(vec![Mount {
            path: "/workspace".to_string(),
            backend: Arc::new(MockBackend),
            read_only: false,
        }]);

        let result = router.resolve("/other/file.txt");
        assert!(matches!(result, Err(VfsError::NoMount(_))));
    }
}

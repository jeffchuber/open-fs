use std::sync::Arc;

use async_trait::async_trait;
use ax_backends::Backend as BackendsBackend;
use ax_config::{BackendConfig, VfsConfig};
use tracing::{debug, instrument};

use crate::error::VfsError;
use crate::router::{Mount, Router};
use crate::traits::{Backend, Entry};

/// Wrapper that adapts ax_backends::FsBackend to our Backend trait.
struct FsBackendWrapper(ax_backends::FsBackend);

#[async_trait]
impl Backend for FsBackendWrapper {
    async fn read(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        BackendsBackend::read(&self.0, path).await.map_err(VfsError::from)
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        BackendsBackend::write(&self.0, path, content).await.map_err(VfsError::from)
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        BackendsBackend::append(&self.0, path, content).await.map_err(VfsError::from)
    }

    async fn delete(&self, path: &str) -> Result<(), VfsError> {
        BackendsBackend::delete(&self.0, path).await.map_err(VfsError::from)
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, VfsError> {
        BackendsBackend::list(&self.0, path)
            .await
            .map(|entries| entries.into_iter().map(Entry::from).collect())
            .map_err(VfsError::from)
    }

    async fn exists(&self, path: &str) -> Result<bool, VfsError> {
        BackendsBackend::exists(&self.0, path).await.map_err(VfsError::from)
    }

    async fn stat(&self, path: &str) -> Result<Entry, VfsError> {
        BackendsBackend::stat(&self.0, path)
            .await
            .map(Entry::from)
            .map_err(VfsError::from)
    }
}

/// Wrapper that adapts ax_backends::MemoryBackend to our Backend trait.
#[allow(dead_code)]
struct MemoryBackendWrapper(ax_backends::MemoryBackend);

#[async_trait]
impl Backend for MemoryBackendWrapper {
    async fn read(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        BackendsBackend::read(&self.0, path).await.map_err(VfsError::from)
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        BackendsBackend::write(&self.0, path, content).await.map_err(VfsError::from)
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        BackendsBackend::append(&self.0, path, content).await.map_err(VfsError::from)
    }

    async fn delete(&self, path: &str) -> Result<(), VfsError> {
        BackendsBackend::delete(&self.0, path).await.map_err(VfsError::from)
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, VfsError> {
        BackendsBackend::list(&self.0, path)
            .await
            .map(|entries| entries.into_iter().map(Entry::from).collect())
            .map_err(VfsError::from)
    }

    async fn exists(&self, path: &str) -> Result<bool, VfsError> {
        BackendsBackend::exists(&self.0, path).await.map_err(VfsError::from)
    }

    async fn stat(&self, path: &str) -> Result<Entry, VfsError> {
        BackendsBackend::stat(&self.0, path)
            .await
            .map(Entry::from)
            .map_err(VfsError::from)
    }
}

/// The main VFS struct that coordinates backends and routing.
pub struct Vfs {
    config: VfsConfig,
    router: Router,
}

impl Vfs {
    /// Create a new VFS from a configuration.
    pub async fn from_config(config: VfsConfig) -> Result<Self, VfsError> {
        // Apply defaults to get effective config
        let effective_config = config.effective();

        // Validate the config
        effective_config.validate_or_err()?;

        // Build backends
        let mut backend_instances: std::collections::HashMap<String, Arc<dyn Backend>> =
            std::collections::HashMap::new();

        for (name, backend_config) in &effective_config.backends {
            let backend: Arc<dyn Backend> = match backend_config {
                BackendConfig::Fs(fs_config) => {
                    let fs_backend = ax_backends::FsBackend::new(&fs_config.root)
                        .map_err(VfsError::from)?;
                    Arc::new(FsBackendWrapper(fs_backend))
                }
                BackendConfig::S3(_) => {
                    return Err(VfsError::Config(
                        "S3 backend not implemented in Phase 1".to_string(),
                    ));
                }
                BackendConfig::Postgres(_) => {
                    return Err(VfsError::Config(
                        "Postgres backend not implemented in Phase 1".to_string(),
                    ));
                }
                BackendConfig::Chroma(_) => {
                    return Err(VfsError::Config(
                        "Chroma backend not implemented in Phase 1".to_string(),
                    ));
                }
                BackendConfig::Api(_) => {
                    return Err(VfsError::Config(
                        "API backend not implemented in Phase 1".to_string(),
                    ));
                }
            };
            backend_instances.insert(name.clone(), backend);
        }

        // Build mounts
        let mut mounts = Vec::new();
        for mount_config in &effective_config.mounts {
            let backend_name = mount_config.backend.as_ref().ok_or_else(|| {
                VfsError::Config(format!(
                    "Mount '{}' has no backend specified",
                    mount_config.path
                ))
            })?;

            let backend = backend_instances.get(backend_name).ok_or_else(|| {
                VfsError::Config(format!(
                    "Backend '{}' not found for mount '{}'",
                    backend_name, mount_config.path
                ))
            })?;

            mounts.push(Mount {
                path: mount_config.path.clone(),
                backend: backend.clone(),
                read_only: mount_config.read_only,
            });
        }

        let router = Router::new(mounts);

        Ok(Vfs {
            config: effective_config,
            router,
        })
    }

    /// Read the contents of a file.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn read(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        debug!(relative = %relative, "resolved path");
        backend.read(&relative).await
    }

    /// Write content to a file.
    #[instrument(skip(self, content), fields(path = %path, size = content.len()))]
    pub async fn write(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        let (backend, relative, read_only) = self.router.resolve(path)?;
        if read_only {
            return Err(VfsError::ReadOnly(path.to_string()));
        }
        debug!(relative = %relative, "resolved path");
        backend.write(&relative, content).await
    }

    /// Append content to a file.
    #[instrument(skip(self, content), fields(path = %path, size = content.len()))]
    pub async fn append(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        let (backend, relative, read_only) = self.router.resolve(path)?;
        if read_only {
            return Err(VfsError::ReadOnly(path.to_string()));
        }
        debug!(relative = %relative, "resolved path");
        backend.append(&relative, content).await
    }

    /// Delete a file.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn delete(&self, path: &str) -> Result<(), VfsError> {
        let (backend, relative, read_only) = self.router.resolve(path)?;
        if read_only {
            return Err(VfsError::ReadOnly(path.to_string()));
        }
        debug!(relative = %relative, "resolved path");
        backend.delete(&relative).await
    }

    /// List entries in a directory.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn list(&self, path: &str) -> Result<Vec<Entry>, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        debug!(relative = %relative, "resolved path");
        backend.list(&relative).await
    }

    /// Check if a path exists.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn exists(&self, path: &str) -> Result<bool, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        backend.exists(&relative).await
    }

    /// Get metadata for a path.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn stat(&self, path: &str) -> Result<Entry, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        backend.stat(&relative).await
    }

    /// Get the effective configuration.
    pub fn effective_config(&self) -> &VfsConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_config(root: &str) -> VfsConfig {
        let yaml = format!(
            r#"
name: test-vfs
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /workspace
    backend: local
"#,
            root
        );
        VfsConfig::from_yaml(&yaml).unwrap()
    }

    #[tokio::test]
    async fn test_vfs_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_config(temp_dir.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/test.txt", b"hello world")
            .await
            .unwrap();
        let content = vfs.read("/workspace/test.txt").await.unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn test_vfs_list() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_config(temp_dir.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/file1.txt", b"content1")
            .await
            .unwrap();
        vfs.write("/workspace/file2.txt", b"content2")
            .await
            .unwrap();

        let entries = vfs.list("/workspace").await.unwrap();
        assert_eq!(entries.len(), 2);

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"file2.txt"));
    }

    #[tokio::test]
    async fn test_vfs_delete() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_config(temp_dir.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/test.txt", b"hello").await.unwrap();
        assert!(vfs.exists("/workspace/test.txt").await.unwrap());

        vfs.delete("/workspace/test.txt").await.unwrap();
        assert!(!vfs.exists("/workspace/test.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_vfs_read_only_mount() {
        let temp_dir = TempDir::new().unwrap();
        let yaml = format!(
            r#"
name: test-vfs
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /readonly
    backend: local
    read_only: true
"#,
            temp_dir.path().to_str().unwrap()
        );
        let config = VfsConfig::from_yaml(&yaml).unwrap();
        let vfs = Vfs::from_config(config).await.unwrap();

        let result = vfs.write("/readonly/test.txt", b"hello").await;
        assert!(matches!(result, Err(VfsError::ReadOnly(_))));
    }

    #[tokio::test]
    async fn test_vfs_no_mount() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_config(temp_dir.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        let result = vfs.read("/nonexistent/file.txt").await;
        assert!(matches!(result, Err(VfsError::NoMount(_))));
    }

    #[tokio::test]
    async fn test_vfs_effective_config() {
        let temp_dir = TempDir::new().unwrap();
        let yaml = format!(
            r#"
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /workspace
"#,
            temp_dir.path().to_str().unwrap()
        );
        let config = VfsConfig::from_yaml(&yaml).unwrap();
        let vfs = Vfs::from_config(config).await.unwrap();

        let effective = vfs.effective_config();
        // Backend should have been inferred
        assert_eq!(
            effective.mounts[0].backend,
            Some("local".to_string())
        );
        // Collection should have been inferred
        assert_eq!(
            effective.mounts[0].collection,
            Some("workspace".to_string())
        );
    }

    #[tokio::test]
    async fn test_vfs_nested_paths() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_config(temp_dir.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/dir/subdir/file.txt", b"nested content")
            .await
            .unwrap();

        let content = vfs.read("/workspace/dir/subdir/file.txt").await.unwrap();
        assert_eq!(content, b"nested content");

        let entries = vfs.list("/workspace/dir").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "subdir");
        assert!(entries[0].is_dir);
    }
}

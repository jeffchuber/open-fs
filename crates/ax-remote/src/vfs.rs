use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ax_config::{BackendConfig, DefaultsConfig, MountMode, SyncConfig as MountSyncConfig, VfsConfig, WriteMode};
use tracing::{debug, info, instrument, warn};

use ax_core::{Backend, BackendError, CacheConfig, Entry, VfsError};
use crate::backends;
use crate::cached_backend::CachedBackend;
use crate::chroma_http::ChromaHttpBackend;
use crate::router::{Mount, Router};
use crate::sync::{SyncConfig, SyncMode};
use crate::wal::{OutboxEntry, OutboxStatus, WalConfig, WalOpType, WriteAheadLog};

/// Wrapper to hold `Arc<dyn Backend>` as a concrete type for `CachedBackend<B>`.
#[derive(Clone)]
struct DynBackend(Arc<dyn Backend>);

#[async_trait]
impl Backend for DynBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        self.0.read(path).await
    }
    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        self.0.write(path, content).await
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

fn cache_config_for_mode(mode: MountMode) -> CacheConfig {
    let mut config = CacheConfig::default();
    config.enabled = matches!(
        mode,
        MountMode::WriteThrough | MountMode::WriteBack | MountMode::RemoteCached | MountMode::PullMirror
    );
    config
}

fn sync_config_for_mount(
    mode: MountMode,
    mount_sync: Option<&MountSyncConfig>,
    defaults: Option<&DefaultsConfig>,
) -> SyncConfig {
    let mut config = SyncConfig::default();

    let mut sync_mode = match mode {
        MountMode::WriteThrough => SyncMode::WriteThrough,
        MountMode::WriteBack => SyncMode::WriteBack,
        MountMode::PullMirror => SyncMode::PullMirror,
        MountMode::RemoteCached => SyncMode::WriteThrough,
        _ => SyncMode::None,
    };

    let sync_override = mount_sync.or_else(|| defaults.and_then(|d| d.sync.as_ref()));
    if let Some(sync_cfg) = sync_override {
        if sync_cfg.write_mode == WriteMode::Async && sync_mode == SyncMode::WriteThrough {
            sync_mode = SyncMode::WriteBack;
        }

        if let Some(interval) = sync_cfg.interval.as_ref() {
            config.flush_interval = interval.as_duration();
        }
    }

    config.mode = sync_mode;
    config
}

fn wal_dir() -> Result<PathBuf, VfsError> {
    if let Ok(path) = std::env::var("AX_WAL_DIR") {
        let path = PathBuf::from(path);
        std::fs::create_dir_all(&path).map_err(VfsError::from)?;
        return Ok(path);
    }

    let cwd = std::env::current_dir().map_err(VfsError::from)?;
    let dir = cwd.join(".ax");
    std::fs::create_dir_all(&dir).map_err(VfsError::from)?;
    Ok(dir)
}

fn sanitize_mount_for_filename(mount_path: &str) -> String {
    let trimmed = mount_path.trim_matches('/');
    if trimmed.is_empty() {
        return "root".to_string();
    }
    trimmed
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn wal_path_for_mount(mount_path: &str) -> Result<PathBuf, VfsError> {
    Ok(wal_dir()?.join(format!(
        "wal_{}.db",
        sanitize_mount_for_filename(mount_path)
    )))
}

async fn apply_outbox_entry(
    backend: Arc<dyn Backend>,
    entry: &OutboxEntry,
) -> Result<(), VfsError> {
    match entry.op_type {
        WalOpType::Write => {
            let content = entry.content.clone().ok_or_else(|| {
                VfsError::Config(format!("Outbox write entry {} missing content", entry.id))
            })?;
            backend.write(&entry.path, &content).await.map_err(VfsError::from)
        }
        WalOpType::Append => {
            let content = entry.content.clone().ok_or_else(|| {
                VfsError::Config(format!("Outbox append entry {} missing content", entry.id))
            })?;
            backend.append(&entry.path, &content).await.map_err(VfsError::from)
        }
        WalOpType::Delete => {
            match backend.delete(&entry.path).await {
                Ok(()) => Ok(()),
                Err(BackendError::NotFound(_)) => Ok(()),
                Err(e) => Err(VfsError::from(e)),
            }
        }
    }
}

async fn replay_outbox_entries(
    wal: &WriteAheadLog,
    backend: Arc<dyn Backend>,
) -> Result<usize, VfsError> {
    let mut applied = 0usize;

    loop {
        let ready = wal
            .fetch_ready_outbox(128)
            .map_err(|e| VfsError::Config(format!("Failed to fetch outbox entries: {}", e)))?;

        if ready.is_empty() {
            break;
        }

        for entry in ready {
            if entry.status != OutboxStatus::Pending {
                continue;
            }

            wal.mark_processing(entry.id)
                .map_err(|e| VfsError::Config(format!("Failed to mark outbox processing: {}", e)))?;

            match apply_outbox_entry(backend.clone(), &entry).await {
                Ok(()) => {
                    wal.complete_outbox(entry.id)
                        .map_err(|e| VfsError::Config(format!("Failed to complete outbox entry: {}", e)))?;
                    applied += 1;
                }
                Err(e) => {
                    let _ = wal.fail_outbox(entry.id, &e.to_string());
                    warn!("Outbox replay failed for {}: {}", entry.path, e);
                }
            }
        }
    }

    Ok(applied)
}

/// Create a backend instance from a BackendConfig.
async fn create_backend(
    name: &str,
    backend_config: &BackendConfig,
) -> Result<Arc<dyn Backend>, VfsError> {
    match backend_config {
        BackendConfig::Fs(fs_config) => {
            let fs_backend = backends::FsBackend::new(&fs_config.root)
                .map_err(VfsError::from)?;
            Ok(Arc::new(fs_backend))
        }
        BackendConfig::Memory(_) => {
            Ok(Arc::new(backends::MemoryBackend::new()))
        }
        BackendConfig::S3(s3_config) => {
            #[cfg(feature = "s3")]
            {
                let backend = backends::S3Backend::new(backends::S3Config {
                    bucket: s3_config.bucket.clone(),
                    prefix: s3_config.prefix.clone(),
                    region: s3_config.region.clone().unwrap_or_else(|| "us-east-1".to_string()),
                    endpoint: s3_config.endpoint.clone(),
                    access_key_id: s3_config.access_key_id.clone(),
                    secret_access_key: s3_config.secret_access_key.clone(),
                })
                .await
                .map_err(VfsError::from)?;
                Ok(Arc::new(backend) as Arc<dyn Backend>)
            }
            #[cfg(not(feature = "s3"))]
            {
                let _ = s3_config;
                Err(VfsError::Config(
                    "S3 backend requires the 's3' feature flag".to_string(),
                ))
            }
        }
        BackendConfig::Postgres(pg_config) => {
            #[cfg(feature = "postgres")]
            {
                let backend = backends::PostgresBackend::new(backends::PostgresConfig {
                    connection_url: pg_config.connection_url.clone(),
                    table_name: pg_config.table_name.clone().unwrap_or_else(|| "ax_files".to_string()),
                    max_connections: pg_config.max_connections.unwrap_or(5),
                })
                .await
                .map_err(VfsError::from)?;
                Ok(Arc::new(backend) as Arc<dyn Backend>)
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = pg_config;
                Err(VfsError::Config(
                    "Postgres backend requires the 'postgres' feature flag".to_string(),
                ))
            }
        }
        BackendConfig::Chroma(chroma_config) => {
            let collection_name = chroma_config.collection.as_deref().unwrap_or("default");
            let chroma_backend = ChromaHttpBackend::new(
                &chroma_config.url,
                collection_name,
            )
            .await
            .map_err(VfsError::from)?;
            Ok(Arc::new(chroma_backend) as Arc<dyn Backend>)
        }
        _ => {
            Err(VfsError::Config(
                format!("Unsupported backend type for '{}'", name),
            ))
        }
    }
}

/// The main VFS struct that coordinates backends and routing.
pub struct Vfs {
    config: VfsConfig,
    router: Router,
    mount_runtimes: Vec<MountRuntime>,
}

struct MountRuntime {
    mount_path: String,
    backend_name: String,
    sync_mode: SyncMode,
    read_only: bool,
    backend: Arc<dyn Backend>,
    cached_backend: Arc<CachedBackend<DynBackend>>,
}

#[derive(Debug, Clone)]
pub struct MountSyncStatus {
    pub mount_path: String,
    pub backend_name: String,
    pub sync_mode: SyncMode,
    pub read_only: bool,
    pub pending: usize,
    pub synced: u64,
    pub failed: u64,
    pub retries: u64,
    pub outbox_pending: Option<usize>,
    pub outbox_processing: Option<usize>,
    pub outbox_failed: Option<usize>,
    pub outbox_wal_unapplied: Option<usize>,
}

impl Vfs {
    /// Create a new VFS from a configuration.
    pub async fn from_config(config: VfsConfig) -> Result<Self, VfsError> {
        let effective_config = config.effective();
        effective_config.validate_or_err()?;

        // Build backends
        let mut backend_instances: HashMap<String, Arc<dyn Backend>> = HashMap::new();

        for (name, backend_config) in &effective_config.backends {
            let backend = create_backend(name, backend_config).await?;
            backend_instances.insert(name.clone(), backend);
        }

        // Build mounts
        let mut mounts = Vec::new();
        let mut mount_runtimes = Vec::new();
        for mount_config in &effective_config.mounts {
            let backend_name = mount_config.backend.as_ref().ok_or_else(|| {
                VfsError::Config(format!(
                    "Mount '{}' has no backend specified",
                    mount_config.path
                ))
            })?;

            let raw_backend = backend_instances.get(backend_name).ok_or_else(|| {
                VfsError::Config(format!(
                    "Backend '{}' not found for mount '{}'",
                    backend_name, mount_config.path
                ))
            })?;

            let mount_mode = mount_config.mode.unwrap_or(MountMode::LocalIndexed);
            let read_only = mount_config.read_only || mount_mode == MountMode::PullMirror;
            let mut cache_config = cache_config_for_mode(mount_mode);
            let sync_config = sync_config_for_mount(
                mount_mode,
                mount_config.sync.as_ref(),
                effective_config.defaults.as_ref(),
            );
            if sync_config.mode == SyncMode::WriteBack {
                cache_config.enabled = true;
            }

            let sync_ref = raw_backend.clone();
            let cached_backend = if sync_config.mode == SyncMode::WriteBack {
                let wal_path = wal_path_for_mount(&mount_config.path)?;
                let wal = Arc::new(
                    WriteAheadLog::new(&wal_path, WalConfig::default()).map_err(|e| {
                        VfsError::Config(format!(
                            "Failed to initialize WAL for mount '{}': {}",
                            mount_config.path, e
                        ))
                    })?,
                );

                let recovered = replay_outbox_entries(wal.as_ref(), raw_backend.clone()).await?;
                if recovered > 0 {
                    info!(
                        "Recovered {} outbox operation(s) for mount {}",
                        recovered, mount_config.path
                    );
                }

                Arc::new(CachedBackend::new_with_wal(
                    DynBackend(raw_backend.clone()),
                    cache_config,
                    sync_config.clone(),
                    read_only,
                    wal,
                ))
            } else {
                Arc::new(CachedBackend::new(
                    DynBackend(raw_backend.clone()),
                    cache_config,
                    sync_config.clone(),
                    read_only,
                ))
            };

            if sync_config.mode == SyncMode::WriteBack {
                cached_backend
                    .start_sync(move |path, content| {
                        let backend = sync_ref.clone();
                        async move {
                            backend
                                .write(&path, &content)
                                .await
                                .map_err(VfsError::from)
                        }
                    })
                    .await;
            }

            let mount_backend: Arc<dyn Backend> = cached_backend.clone();

            mounts.push(Mount {
                path: mount_config.path.clone(),
                backend: mount_backend,
                read_only,
            });

            mount_runtimes.push(MountRuntime {
                mount_path: mount_config.path.clone(),
                backend_name: backend_name.clone(),
                sync_mode: sync_config.mode,
                read_only,
                backend: raw_backend.clone(),
                cached_backend,
            });
        }

        let router = Router::new(mounts);

        Ok(Vfs {
            config: effective_config,
            router,
            mount_runtimes,
        })
    }

    /// Read the contents of a file.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn read(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        debug!(relative = %relative, "resolved path");
        backend.read(&relative).await.map_err(VfsError::from)
    }

    /// Write content to a file.
    #[instrument(skip(self, content), fields(path = %path, size = content.len()))]
    pub async fn write(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        let (backend, relative, read_only) = self.router.resolve(path)?;
        if read_only {
            return Err(VfsError::ReadOnly(path.to_string()));
        }
        debug!(relative = %relative, "resolved path");
        backend.write(&relative, content).await.map_err(VfsError::from)
    }

    /// Append content to a file.
    #[instrument(skip(self, content), fields(path = %path, size = content.len()))]
    pub async fn append(&self, path: &str, content: &[u8]) -> Result<(), VfsError> {
        let (backend, relative, read_only) = self.router.resolve(path)?;
        if read_only {
            return Err(VfsError::ReadOnly(path.to_string()));
        }
        debug!(relative = %relative, "resolved path");
        backend.append(&relative, content).await.map_err(VfsError::from)
    }

    /// Delete a file.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn delete(&self, path: &str) -> Result<(), VfsError> {
        let (backend, relative, read_only) = self.router.resolve(path)?;
        if read_only {
            return Err(VfsError::ReadOnly(path.to_string()));
        }
        debug!(relative = %relative, "resolved path");
        backend.delete(&relative).await.map_err(VfsError::from)
    }

    /// List entries in a directory.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn list(&self, path: &str) -> Result<Vec<Entry>, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        debug!(relative = %relative, "resolved path");
        backend.list(&relative).await.map_err(VfsError::from)
    }

    /// Check if a path exists.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn exists(&self, path: &str) -> Result<bool, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        backend.exists(&relative).await.map_err(VfsError::from)
    }

    /// Get metadata for a path.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn stat(&self, path: &str) -> Result<Entry, VfsError> {
        let (backend, relative, _) = self.router.resolve(path)?;
        backend.stat(&relative).await.map_err(VfsError::from)
    }

    /// Rename/move a file or directory.
    #[instrument(skip(self), fields(from = %from, to = %to))]
    pub async fn rename(&self, from: &str, to: &str) -> Result<(), VfsError> {
        let (from_backend, from_relative, from_read_only) = self.router.resolve(from)?;
        let (to_backend, to_relative, to_read_only) = self.router.resolve(to)?;

        if from_read_only {
            return Err(VfsError::ReadOnly(from.to_string()));
        }
        if to_read_only {
            return Err(VfsError::ReadOnly(to.to_string()));
        }

        // Check if both paths are on the same mount (same backend instance)
        if std::ptr::eq(
            from_backend as *const dyn Backend,
            to_backend as *const dyn Backend,
        ) {
            from_backend.rename(&from_relative, &to_relative).await.map_err(VfsError::from)
        } else {
            // Different backends, must copy and delete
            let content = from_backend.read(&from_relative).await.map_err(VfsError::from)?;
            to_backend.write(&to_relative, &content).await.map_err(VfsError::from)?;
            from_backend.delete(&from_relative).await.map_err(VfsError::from)?;
            Ok(())
        }
    }

    /// Get the effective configuration.
    pub fn effective_config(&self) -> &VfsConfig {
        &self.config
    }

    /// Return per-mount sync status (including durable outbox counts when WAL is enabled).
    pub async fn sync_statuses(&self) -> Result<Vec<MountSyncStatus>, VfsError> {
        let mut statuses = Vec::with_capacity(self.mount_runtimes.len());

        for runtime in &self.mount_runtimes {
            let sync = runtime.cached_backend.sync_stats().await;
            let outbox = runtime
                .cached_backend
                .wal()
                .map(|wal| {
                    wal.outbox_stats()
                        .map_err(|e| VfsError::Config(format!("Failed to read outbox stats: {}", e)))
                })
                .transpose()?;

            statuses.push(MountSyncStatus {
                mount_path: runtime.mount_path.clone(),
                backend_name: runtime.backend_name.clone(),
                sync_mode: runtime.sync_mode,
                read_only: runtime.read_only,
                pending: sync.pending,
                synced: sync.synced,
                failed: sync.failed,
                retries: sync.retries,
                outbox_pending: outbox.as_ref().map(|s| s.pending),
                outbox_processing: outbox.as_ref().map(|s| s.processing),
                outbox_failed: outbox.as_ref().map(|s| s.failed),
                outbox_wal_unapplied: outbox.as_ref().map(|s| s.wal_unapplied),
            });
        }

        Ok(statuses)
    }

    /// Flush all write-back mounts and replay any remaining durable outbox entries.
    pub async fn flush_write_back(&self) -> Result<usize, VfsError> {
        let mut flushed_mounts = 0usize;

        for runtime in &self.mount_runtimes {
            if runtime.sync_mode != SyncMode::WriteBack {
                continue;
            }

            runtime.cached_backend.shutdown_sync().await;

            if let Some(wal) = runtime.cached_backend.wal() {
                let replayed = replay_outbox_entries(wal.as_ref(), runtime.backend.clone()).await?;
                if replayed > 0 {
                    info!(
                        "Replayed {} outbox operation(s) during flush for mount {}",
                        replayed, runtime.mount_path
                    );
                }
            }

            flushed_mounts += 1;
        }

        Ok(flushed_mounts)
    }

    /// Resolve a VFS path to its physical filesystem path.
    /// Returns None for non-fs backends (S3, Postgres, Chroma, API).
    pub fn resolve_fs_path(&self, vfs_path: &str) -> Option<std::path::PathBuf> {
        for mount_config in &self.config.mounts {
            let mount_path = mount_config.path.trim_end_matches('/');
            if vfs_path == mount_path || vfs_path.starts_with(&format!("{}/", mount_path)) {
                if let Some(ref backend_name) = mount_config.backend {
                    if let Some(backend_config) = self.config.backends.get(backend_name) {
                        if let BackendConfig::Fs(fs_config) = backend_config {
                            let relative = if vfs_path == mount_path {
                                ""
                            } else {
                                &vfs_path[mount_path.len() + 1..]
                            };
                            let fs_root = std::path::Path::new(&fs_config.root);
                            return Some(fs_root.join(relative));
                        }
                    }
                }
                return None;
            }
        }
        None
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

    fn make_write_back_config(root: &str, mount_path: &str, interval: &str) -> VfsConfig {
        let yaml = format!(
            r#"
name: test-vfs-write-back
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: {}
    backend: local
    mode: write_back
    sync:
      interval: {}
"#,
            root, mount_path, interval
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
        assert_eq!(
            effective.mounts[0].backend,
            Some("local".to_string())
        );
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

    #[tokio::test]
    async fn test_vfs_rename() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_config(temp_dir.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/original.txt", b"content")
            .await
            .unwrap();
        assert!(vfs.exists("/workspace/original.txt").await.unwrap());

        vfs.rename("/workspace/original.txt", "/workspace/renamed.txt")
            .await
            .unwrap();

        assert!(!vfs.exists("/workspace/original.txt").await.unwrap());
        assert!(vfs.exists("/workspace/renamed.txt").await.unwrap());

        let content = vfs.read("/workspace/renamed.txt").await.unwrap();
        assert_eq!(content, b"content");
    }

    #[tokio::test]
    async fn test_vfs_flush_write_back() {
        let mount_path = "/wb_flush_test";
        let wal_path = wal_path_for_mount(mount_path).unwrap();
        let _ = std::fs::remove_file(&wal_path);

        let temp_dir = TempDir::new().unwrap();
        let config = make_write_back_config(
            temp_dir.path().to_str().unwrap(),
            mount_path,
            "24h",
        );
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/wb_flush_test/file.txt", b"flush me").await.unwrap();
        vfs.flush_write_back().await.unwrap();

        let on_disk = std::fs::read(temp_dir.path().join("file.txt")).unwrap();
        assert_eq!(on_disk, b"flush me");
    }

    #[tokio::test]
    async fn test_vfs_recovers_write_back_outbox_on_startup() {
        let mount_path = "/wb_recover_test";
        let wal_path = wal_path_for_mount(mount_path).unwrap();
        let _ = std::fs::remove_file(&wal_path);

        let temp_dir = TempDir::new().unwrap();
        let config = make_write_back_config(
            temp_dir.path().to_str().unwrap(),
            mount_path,
            "24h",
        );

        {
            let vfs = Vfs::from_config(config.clone()).await.unwrap();
            vfs.write("/wb_recover_test/file.txt", b"recover me")
                .await
                .unwrap();
            // Intentionally drop without flush to simulate abrupt process end.
        }

        let recovered_vfs = Vfs::from_config(config).await.unwrap();
        let recovered = std::fs::read(temp_dir.path().join("file.txt")).unwrap();
        assert_eq!(recovered, b"recover me");

        // Ensure the recovered instance stays usable.
        recovered_vfs
            .write("/wb_recover_test/other.txt", b"ok")
            .await
            .unwrap();
    }
}

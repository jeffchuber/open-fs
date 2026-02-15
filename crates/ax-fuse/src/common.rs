//! Platform-neutral FUSE core logic.
//!
//! This module contains the shared VFS-interaction code for the Unix
//! (`fuser`) FUSE implementation.

use std::path::Path;
use std::sync::Arc;

use ax_config::VfsConfig;
use ax_remote::Vfs;
use tracing::info;

use crate::async_bridge::{block_on, init_runtime};
use crate::inode::{InodeAttr, InodeKind, InodeTable, ROOT_INO, VIRTUAL_INO_BASE};
use crate::search_dir::{SearchDir, QUERY_DIR_PATH, SEARCH_DIR_PATH};

/// Errors returned by filesystem operations.
#[derive(Debug)]
pub enum FsOpError {
    /// File or directory not found.
    NotFound,
    /// Operation not permitted (read-only mount or virtual directory).
    ReadOnly,
    /// Invalid argument (e.g., bad filename encoding).
    InvalidArg,
    /// Directory is not empty.
    NotEmpty,
    /// Generic I/O error.
    Io(String),
    /// Not a symlink.
    NotSymlink,
    /// Is a directory (tried to read as file).
    IsDir,
}

impl From<crate::async_bridge::FuseError> for FsOpError {
    fn from(e: crate::async_bridge::FuseError) -> Self {
        match e {
            crate::async_bridge::FuseError::NotFound => FsOpError::NotFound,
            crate::async_bridge::FuseError::ReadOnly => FsOpError::ReadOnly,
            crate::async_bridge::FuseError::IsDir => FsOpError::IsDir,
            crate::async_bridge::FuseError::NotEmpty => FsOpError::NotEmpty,
            crate::async_bridge::FuseError::PermissionDenied => FsOpError::ReadOnly,
            _ => FsOpError::Io(e.to_string()),
        }
    }
}

/// Core FUSE filesystem logic, platform-independent.
pub struct AxFsCore {
    /// The underlying VFS.
    pub vfs: Arc<Vfs>,
    /// Inode management.
    pub inodes: Arc<InodeTable>,
    /// Virtual search directory.
    pub search_dir: Arc<SearchDir>,
    /// Auto-index on write.
    pub auto_index: bool,
}

impl AxFsCore {
    fn materialize_query_results(&self, query_path: &str) -> Result<(), FsOpError> {
        let query = SearchDir::extract_query(query_path).ok_or(FsOpError::NotFound)?;
        if self.search_dir.has_query(&query) {
            return Ok(());
        }

        let vfs = self.vfs.clone();
        let grep_result = block_on(async {
            let opts = ax_remote::GrepOptions {
                recursive: true,
                max_matches: 200,
                max_depth: 20,
            };
            ax_remote::grep(&vfs, &query, "/", &opts).await
        })?;

        let tuples: Vec<(String, String, f32, usize, usize)> = match grep_result {
            Ok(matches) => matches
                .into_iter()
                .map(|m| (m.path, m.line, 1.0, m.line_number, m.line_number))
                .collect(),
            Err(_) => Vec::new(),
        };

        let entries = self.search_dir.create_result_entries(&tuples);
        self.search_dir.store_results(&query, entries);
        Ok(())
    }

    /// Create a new core from a config file.
    pub fn from_config_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let config = VfsConfig::from_file(path)?;
        Self::from_config(config)
    }

    /// Create a new core from a config.
    pub fn from_config(config: VfsConfig) -> Result<Self, Box<dyn std::error::Error>> {
        init_runtime()?;

        let vfs = block_on(async { Vfs::from_config(config).await })??;

        let inodes = Arc::new(InodeTable::new());
        let search_dir = Arc::new(SearchDir::new(inodes.clone()));

        Ok(AxFsCore {
            vfs: Arc::new(vfs),
            inodes,
            search_dir,
            auto_index: false,
        })
    }

    /// Enable auto-indexing on file writes.
    pub fn with_auto_index(mut self, enabled: bool) -> Self {
        self.auto_index = enabled;
        self
    }

    /// Get the path for an inode.
    pub fn get_path(&self, ino: u64) -> Option<String> {
        self.inodes.get_path(ino)
    }

    /// Resolve a child path from parent + name.
    pub fn child_path(parent_path: &str, name: &str) -> String {
        if parent_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent_path, name)
        }
    }

    /// Perform a lookup operation.
    pub fn do_lookup(&self, parent: u64, name: &str) -> Result<InodeAttr, FsOpError> {
        let parent_path = self.get_path(parent).ok_or(FsOpError::NotFound)?;
        let child_path = Self::child_path(&parent_path, name);

        if SearchDir::is_query_dir(&parent_path) {
            let _ = self.materialize_query_results(&child_path);
        } else if SearchDir::is_query_path(&parent_path) {
            let _ = self.materialize_query_results(&parent_path);
        }

        // Check for virtual .search directory
        if SearchDir::is_search_path(&child_path) {
            if let Some(attr) = self.search_dir.getattr(&child_path) {
                return Ok(attr);
            }
        }

        // Handle .search as special name in root
        if parent == ROOT_INO && name == ".search" {
            if let Some(attr) = self.search_dir.getattr(SEARCH_DIR_PATH) {
                return Ok(attr);
            }
        }

        // Check within .search directory
        if SearchDir::is_search_path(&parent_path) {
            if let Some((_ino, attr)) = self.search_dir.lookup(&parent_path, name) {
                return Ok(attr);
            }
            return Err(FsOpError::NotFound);
        }

        // Regular VFS lookup
        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.stat(&child_path).await })?;

        match result {
            Ok(entry) => {
                let ino = self.inodes.get_or_create(
                    &child_path,
                    entry.is_dir,
                    entry.size.unwrap_or(0),
                );
                self.inodes.get_attr(ino).ok_or(FsOpError::Io("inode missing after creation".to_string()))
            }
            Err(_) => Err(FsOpError::NotFound),
        }
    }

    /// Perform a getattr operation.
    pub fn do_getattr(&self, ino: u64) -> Result<InodeAttr, FsOpError> {
        // Check for virtual inode
        if ino >= VIRTUAL_INO_BASE {
            if let Some(attr) = self.inodes.get_attr(ino) {
                return Ok(attr);
            }
        }

        let path = match self.get_path(ino) {
            Some(p) => p,
            None => {
                if let Some(attr) = self.search_dir.getattr(&format!("/{}", ino)) {
                    return Ok(attr);
                }
                return Err(FsOpError::NotFound);
            }
        };

        // Handle virtual .search directory
        if SearchDir::is_search_path(&path) {
            if let Some(attr) = self.search_dir.getattr(&path) {
                return Ok(attr);
            }
        }

        // Root directory
        if ino == ROOT_INO {
            if let Some(attr) = self.inodes.get_attr(ino) {
                return Ok(attr);
            }
        }

        // Regular VFS stat
        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.stat(&path).await })?;

        match result {
            Ok(entry) => {
                self.inodes.update_attr(ino, entry.size.unwrap_or(0), entry.is_dir);
                self.inodes.get_attr(ino).ok_or(FsOpError::NotFound)
            }
            Err(_) => Err(FsOpError::NotFound),
        }
    }

    /// Perform a read operation.
    pub fn do_read(&self, ino: u64, offset: i64, size: u32) -> Result<Vec<u8>, FsOpError> {
        let path = self.get_path(ino).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_search_path(&path) {
            if self.search_dir.read_file(&path).is_none()
                && path.starts_with(&format!("{}/", QUERY_DIR_PATH))
            {
                if let Some((query_path, _)) = path.rsplit_once('/') {
                    let _ = self.materialize_query_results(query_path);
                }
            }

            if let Some(data) = self.search_dir.read_file(&path) {
                let start = offset as usize;
                if start >= data.len() {
                    return Ok(Vec::new());
                }
                let end = (start + size as usize).min(data.len());
                return Ok(data[start..end].to_vec());
            }
            return Err(FsOpError::IsDir);
        }

        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.read(&path).await })?;

        match result {
            Ok(data) => {
                let start = offset as usize;
                if start >= data.len() {
                    Ok(Vec::new())
                } else {
                    let end = (start + size as usize).min(data.len());
                    Ok(data[start..end].to_vec())
                }
            }
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// Perform a write operation.
    pub fn do_write(&self, ino: u64, offset: i64, data: &[u8]) -> Result<u32, FsOpError> {
        let path = self.get_path(ino).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_search_path(&path) {
            return Err(FsOpError::ReadOnly);
        }

        let vfs = self.vfs.clone();
        let result = block_on(async {
            if offset == 0 {
                vfs.write(&path, data).await
            } else {
                let existing = match vfs.read(&path).await {
                    Ok(content) => content,
                    Err(ax_core::VfsError::NotFound(_)) => Vec::new(),
                    Err(e) => return Err(e),
                };
                let mut new_content = existing;
                let start = offset as usize;
                if start > new_content.len() {
                    new_content.resize(start, 0);
                }
                if start + data.len() > new_content.len() {
                    new_content.resize(start + data.len(), 0);
                }
                new_content[start..start + data.len()].copy_from_slice(data);
                vfs.write(&path, &new_content).await
            }
        })?;

        match result {
            Ok(()) => {
                let new_size = if offset == 0 {
                    data.len() as u64
                } else {
                    (offset as u64) + (data.len() as u64)
                };
                self.inodes.update_attr(ino, new_size, false);
                Ok(data.len() as u32)
            }
            Err(ax_core::VfsError::ReadOnly(_)) => Err(FsOpError::ReadOnly),
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// List a directory. Returns (ino, name, kind) tuples.
    pub fn do_readdir(&self, ino: u64) -> Result<ReadDirResult, FsOpError> {
        let path = self.get_path(ino).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_query_path(&path) {
            let _ = self.materialize_query_results(&path);
        }

        // Handle virtual .search directory
        if SearchDir::is_search_path(&path) {
            if let Some(entries) = self.search_dir.readdir(&path) {
                let parent_ino = if path == SEARCH_DIR_PATH { ROOT_INO } else { ino };
                return Ok(ReadDirResult {
                    ino,
                    parent_ino,
                    entries: entries.into_iter().map(|(entry_ino, name, kind)| {
                        DirEntry { ino: entry_ino, name, kind }
                    }).collect(),
                    is_root: false,
                });
            }
        }

        // Regular VFS directory listing
        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.list(&path).await })?;

        match result {
            Ok(vfs_entries) => {
                let parent_path = if path == "/" {
                    "/".to_string()
                } else {
                    path.rsplit_once('/')
                        .map(|(p, _)| if p.is_empty() { "/" } else { p })
                        .unwrap_or("/")
                        .to_string()
                };
                let parent_ino = self.inodes.get_ino(&parent_path).unwrap_or(ROOT_INO);

                let entries: Vec<DirEntry> = vfs_entries.into_iter().map(|entry| {
                    let child_path = Self::child_path(&path, &entry.name);
                    let entry_ino = self.inodes.get_or_create(
                        &child_path,
                        entry.is_dir,
                        entry.size.unwrap_or(0),
                    );
                    let kind = if entry.is_dir { InodeKind::Directory } else { InodeKind::File };
                    DirEntry { ino: entry_ino, name: entry.name, kind }
                }).collect();

                Ok(ReadDirResult {
                    ino,
                    parent_ino,
                    entries,
                    is_root: path == "/",
                })
            }
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// Create a new file.
    pub fn do_create(&self, parent: u64, name: &str) -> Result<InodeAttr, FsOpError> {
        let parent_path = self.get_path(parent).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_search_path(&parent_path) {
            return Err(FsOpError::ReadOnly);
        }

        let child_path = Self::child_path(&parent_path, name);

        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.write(&child_path, &[]).await })?;

        match result {
            Ok(()) => {
                let ino = self.inodes.get_or_create(&child_path, false, 0);
                self.inodes.get_attr(ino).ok_or(FsOpError::Io("inode missing after creation".to_string()))
            }
            Err(ax_core::VfsError::ReadOnly(_)) => Err(FsOpError::ReadOnly),
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// Create a new directory.
    pub fn do_mkdir(&self, parent: u64, name: &str) -> Result<InodeAttr, FsOpError> {
        let parent_path = self.get_path(parent).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_search_path(&parent_path) {
            return Err(FsOpError::ReadOnly);
        }

        let child_path = Self::child_path(&parent_path, name);
        let placeholder_path = format!("{}/.axkeep", child_path);

        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.write(&placeholder_path, b"").await })?;

        match result {
            Ok(()) => {
                let ino = self.inodes.get_or_create(&child_path, true, 0);
                self.inodes.get_attr(ino).ok_or(FsOpError::Io("inode missing after creation".to_string()))
            }
            Err(ax_core::VfsError::ReadOnly(_)) => Err(FsOpError::ReadOnly),
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// Delete a file.
    pub fn do_unlink(&self, parent: u64, name: &str) -> Result<(), FsOpError> {
        let parent_path = self.get_path(parent).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_search_path(&parent_path) {
            return Err(FsOpError::ReadOnly);
        }

        let child_path = Self::child_path(&parent_path, name);

        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.delete(&child_path).await })?;

        match result {
            Ok(()) => {
                self.inodes.remove_path(&child_path);
                Ok(())
            }
            Err(ax_core::VfsError::ReadOnly(_)) => Err(FsOpError::ReadOnly),
            Err(ax_core::VfsError::NotFound(_)) => Err(FsOpError::NotFound),
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// Remove a directory.
    pub fn do_rmdir(&self, parent: u64, name: &str) -> Result<(), FsOpError> {
        let parent_path = self.get_path(parent).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_search_path(&parent_path) {
            return Err(FsOpError::ReadOnly);
        }

        let child_path = Self::child_path(&parent_path, name);

        let vfs = self.vfs.clone();
        let list_result = block_on(async { vfs.list(&child_path).await })?;

        match list_result {
            Ok(entries) => {
                let non_placeholder: Vec<_> = entries.iter().filter(|e| e.name != ".axkeep").collect();
                if !non_placeholder.is_empty() {
                    return Err(FsOpError::NotEmpty);
                }

                let axkeep_path = format!("{}/.axkeep", child_path);
                if let Ok(Err(e)) = block_on(async { vfs.delete(&axkeep_path).await }) {
                    tracing::warn!("Failed to delete .axkeep: {}", e);
                }

                self.inodes.remove_path(&child_path);
                Ok(())
            }
            Err(ax_core::VfsError::NotFound(_)) => Err(FsOpError::NotFound),
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// Read a symlink target.
    pub fn do_readlink(&self, ino: u64) -> Result<String, FsOpError> {
        if let Some(target) = self.search_dir.readlink(ino) {
            return Ok(target);
        }
        Err(FsOpError::NotSymlink)
    }

    /// Check if a path exists.
    pub fn do_access(&self, ino: u64) -> Result<(), FsOpError> {
        if self.get_path(ino).is_some() || ino >= VIRTUAL_INO_BASE {
            Ok(())
        } else {
            Err(FsOpError::NotFound)
        }
    }

    /// Handle setattr (truncate).
    pub fn do_setattr(&self, ino: u64, size: Option<u64>) -> Result<InodeAttr, FsOpError> {
        let path = self.get_path(ino).ok_or(FsOpError::NotFound)?;

        if let Some(new_size) = size {
            if new_size == 0 {
                let vfs = self.vfs.clone();
                let result = block_on(async { vfs.write(&path, &[]).await })?;
                if let Err(e) = result {
                    return Err(FsOpError::Io(e.to_string()));
                }
            }
            self.inodes.update_attr(ino, new_size, false);
        }

        self.inodes.get_attr(ino).ok_or(FsOpError::NotFound)
    }

    /// Handle rename.
    pub fn do_rename(
        &self,
        parent: u64,
        name: &str,
        newparent: u64,
        newname: &str,
    ) -> Result<(), FsOpError> {
        let parent_path = self.get_path(parent).ok_or(FsOpError::NotFound)?;
        let newparent_path = self.get_path(newparent).ok_or(FsOpError::NotFound)?;

        if SearchDir::is_search_path(&parent_path) || SearchDir::is_search_path(&newparent_path) {
            return Err(FsOpError::ReadOnly);
        }

        let src_path = Self::child_path(&parent_path, name);
        let dst_path = Self::child_path(&newparent_path, newname);

        let vfs = self.vfs.clone();
        let result = block_on(async { vfs.rename(&src_path, &dst_path).await })?;

        match result {
            Ok(()) => {
                self.inodes.remove_path(&src_path);
                Ok(())
            }
            Err(ax_core::VfsError::ReadOnly(_)) => Err(FsOpError::ReadOnly),
            Err(ax_core::VfsError::NotFound(_)) => Err(FsOpError::NotFound),
            Err(e) => Err(FsOpError::Io(e.to_string())),
        }
    }

    /// Mount the filesystem (platform-specific dispatch).
    #[cfg(unix)]
    pub fn mount(self, mountpoint: &Path) -> Result<(), Box<dyn std::error::Error>> {
        use crate::unix_fuse::UnixFuse;
        use fuser::MountOption;

        let options = vec![
            MountOption::FSName("axfs".to_string()),
            MountOption::AutoUnmount,
            MountOption::AllowOther,
            MountOption::DefaultPermissions,
        ];

        info!("Mounting AX VFS at {:?}", mountpoint);
        fuser::mount2(UnixFuse(self), mountpoint, &options)?;
        info!("AX VFS unmounted");

        Ok(())
    }

    /// Mount the filesystem in the foreground.
    #[cfg(unix)]
    pub fn mount_foreground(self, mountpoint: &Path) -> Result<(), Box<dyn std::error::Error>> {
        use crate::unix_fuse::UnixFuse;
        use fuser::MountOption;

        let options = vec![
            MountOption::FSName("axfs".to_string()),
            MountOption::AutoUnmount,
        ];

        info!("Mounting AX VFS at {:?} (foreground)", mountpoint);
        fuser::mount2(UnixFuse(self), mountpoint, &options)?;

        Ok(())
    }
}

/// Result from a readdir operation.
pub struct ReadDirResult {
    /// Inode of the directory being listed.
    pub ino: u64,
    /// Inode of the parent directory.
    pub parent_ino: u64,
    /// Entries in the directory.
    pub entries: Vec<DirEntry>,
    /// Whether this is the root directory.
    pub is_root: bool,
}

/// A single directory entry.
pub struct DirEntry {
    /// Inode number.
    pub ino: u64,
    /// Entry name.
    pub name: String,
    /// Entry kind.
    pub kind: InodeKind,
}

/// Convert an `InodeAttr` to a platform-specific `FileAttr` representation.
/// This is a helper used by the FUSE drivers.
#[cfg(unix)]
pub fn inode_attr_to_file_attr(attr: &InodeAttr) -> fuser::FileAttr {
    use fuser::{FileAttr, FileType};

    let kind = match attr.kind {
        InodeKind::File => FileType::RegularFile,
        InodeKind::Directory => FileType::Directory,
        InodeKind::Symlink => FileType::Symlink,
    };

    FileAttr {
        ino: attr.ino,
        size: attr.size,
        blocks: attr.blocks,
        atime: attr.atime,
        mtime: attr.mtime,
        ctime: attr.ctime,
        crtime: attr.crtime,
        kind,
        perm: attr.perm,
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_bridge::block_on;
    use tempfile::TempDir;

    fn make_test_config(root: &str) -> VfsConfig {
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

    #[test]
    fn test_core_from_config() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config);
        assert!(core.is_ok());
    }

    #[test]
    fn test_core_with_auto_index() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap().with_auto_index(true);
        assert!(core.auto_index);
    }

    #[test]
    fn test_core_default_auto_index_false() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();
        assert!(!core.auto_index);
    }

    #[test]
    fn test_core_has_root_inode() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();
        assert_eq!(core.get_path(ROOT_INO), Some("/".to_string()));
    }

    #[test]
    fn test_child_path_from_root() {
        assert_eq!(AxFsCore::child_path("/", "file.txt"), "/file.txt");
    }

    #[test]
    fn test_child_path_from_subdir() {
        assert_eq!(AxFsCore::child_path("/workspace", "file.txt"), "/workspace/file.txt");
    }

    #[test]
    fn test_core_vfs_read_write() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async {
            core.vfs.write("/workspace/test.txt", b"hello world").await.unwrap();
        }).unwrap();

        let content = block_on(async {
            core.vfs.read("/workspace/test.txt").await.unwrap()
        }).unwrap();
        assert_eq!(content, b"hello world");
    }

    #[test]
    fn test_core_vfs_list() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async {
            core.vfs.write("/workspace/file1.txt", b"content1").await.unwrap();
            core.vfs.write("/workspace/file2.txt", b"content2").await.unwrap();
        }).unwrap();

        let entries = block_on(async {
            core.vfs.list("/workspace").await.unwrap()
        }).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_core_vfs_exists() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let exists = block_on(async { core.vfs.exists("/workspace/test.txt").await.unwrap() }).unwrap();
        assert!(!exists);

        block_on(async { core.vfs.write("/workspace/test.txt", b"content").await.unwrap() }).unwrap();

        let exists = block_on(async { core.vfs.exists("/workspace/test.txt").await.unwrap() }).unwrap();
        assert!(exists);
    }

    #[test]
    fn test_core_vfs_stat() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async { core.vfs.write("/workspace/test.txt", b"hello").await.unwrap() }).unwrap();

        let entry = block_on(async { core.vfs.stat("/workspace/test.txt").await.unwrap() }).unwrap();
        assert_eq!(entry.name, "test.txt");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, Some(5));
    }

    #[test]
    fn test_core_vfs_delete() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async {
            core.vfs.write("/workspace/test.txt", b"content").await.unwrap();
            assert!(core.vfs.exists("/workspace/test.txt").await.unwrap());
            core.vfs.delete("/workspace/test.txt").await.unwrap();
            assert!(!core.vfs.exists("/workspace/test.txt").await.unwrap());
        }).unwrap();
    }

    #[test]
    fn test_core_search_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let attr = core.search_dir.getattr("/.search");
        assert!(attr.is_some());
    }

    #[test]
    fn test_core_search_dir_is_virtual() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let attr = core.search_dir.getattr("/.search").unwrap();
        assert!(core.inodes.is_virtual(attr.ino));
    }

    #[test]
    fn test_core_get_path_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();
        assert_eq!(core.get_path(99999), None);
    }

    #[test]
    fn test_core_get_path_after_create() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let ino = core.inodes.get_or_create("/workspace/test.txt", false, 100);
        assert_eq!(core.get_path(ino), Some("/workspace/test.txt".to_string()));
    }

    #[test]
    fn test_core_inode_updates_on_write() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async { core.vfs.write("/workspace/test.txt", b"hello").await.unwrap() }).unwrap();

        let ino = core.inodes.get_or_create("/workspace/test.txt", false, 5);
        let attr = core.inodes.get_attr(ino).unwrap();
        assert_eq!(attr.size, 5);

        core.inodes.update_attr(ino, 100, false);
        let attr = core.inodes.get_attr(ino).unwrap();
        assert_eq!(attr.size, 100);
    }

    #[test]
    fn test_core_effective_config() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let effective = core.vfs.effective_config();
        assert_eq!(effective.name, Some("test-vfs".to_string()));
        assert!(!effective.mounts.is_empty());
    }

    #[test]
    fn test_core_search_results_accessible() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let results = vec![
            ("/workspace/auth.py".to_string(), "auth code".to_string(), 0.95, 10, 20),
        ];
        let entries = core.search_dir.create_result_entries(&results);
        core.search_dir.store_results("auth", entries);

        let dir_entries = core.search_dir.readdir("/.search/query/auth").unwrap();
        assert_eq!(dir_entries.len(), 1);
    }

    #[test]
    fn test_core_handles_special_characters() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async {
            core.vfs.write("/workspace/file with spaces.txt", b"content").await.unwrap();
        }).unwrap();

        let content = block_on(async {
            core.vfs.read("/workspace/file with spaces.txt").await.unwrap()
        }).unwrap();
        assert_eq!(content, b"content");
    }

    #[test]
    fn test_core_handles_deep_paths() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let deep_path = "/workspace/a/b/c/d/e/f/g/file.txt";
        block_on(async { core.vfs.write(deep_path, b"deep content").await.unwrap() }).unwrap();

        let content = block_on(async { core.vfs.read(deep_path).await.unwrap() }).unwrap();
        assert_eq!(content, b"deep content");
    }

    #[test]
    fn test_core_handles_large_file() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let large_content = vec![b'x'; 1024 * 1024];
        block_on(async { core.vfs.write("/workspace/large.bin", &large_content).await.unwrap() }).unwrap();

        let content = block_on(async { core.vfs.read("/workspace/large.bin").await.unwrap() }).unwrap();
        assert_eq!(content.len(), 1024 * 1024);
    }

    #[test]
    fn test_core_handles_binary_content() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        let binary_content: Vec<u8> = (0..=255).collect();
        block_on(async { core.vfs.write("/workspace/binary.bin", &binary_content).await.unwrap() }).unwrap();

        let content = block_on(async { core.vfs.read("/workspace/binary.bin").await.unwrap() }).unwrap();
        assert_eq!(content, binary_content);
    }

    #[test]
    fn test_core_handles_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async { core.vfs.write("/workspace/empty.txt", b"").await.unwrap() }).unwrap();

        let content = block_on(async { core.vfs.read("/workspace/empty.txt").await.unwrap() }).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_core_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = Arc::new(AxFsCore::from_config(config).unwrap());

        {
            let core = Arc::clone(&core);
            block_on(async move {
                core.vfs.write("/workspace/shared.txt", b"shared content").await.unwrap();
            }).unwrap();
        }

        let handles: Vec<_> = (0..10)
            .map(|_| {
                let core = Arc::clone(&core);
                thread::spawn(move || {
                    block_on(async move { core.vfs.read("/workspace/shared.txt").await.unwrap() }).unwrap()
                })
            })
            .collect();

        for handle in handles {
            let content = handle.join().unwrap();
            assert_eq!(content, b"shared content");
        }
    }

    #[test]
    fn test_core_concurrent_inode_access() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let config = make_test_config(temp_dir.path().to_str().unwrap());
        let core = Arc::new(AxFsCore::from_config(config).unwrap());

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let core = Arc::clone(&core);
                thread::spawn(move || {
                    for j in 0..100 {
                        let path = format!("/workspace/thread{}_file{}.txt", i, j);
                        core.inodes.get_or_create(&path, false, j as u64);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        for i in 0..10 {
            for j in 0..100 {
                let path = format!("/workspace/thread{}_file{}.txt", i, j);
                assert!(core.inodes.get_ino(&path).is_some());
            }
        }
    }

    fn make_multi_mount_config(root1: &str, root2: &str) -> VfsConfig {
        let yaml = format!(
            r#"
name: multi-mount-vfs
backends:
  local1:
    type: fs
    root: {}
  local2:
    type: fs
    root: {}
mounts:
  - path: /workspace
    backend: local1
  - path: /docs
    backend: local2
    read_only: true
"#,
            root1, root2
        );
        VfsConfig::from_yaml(&yaml).unwrap()
    }

    #[test]
    fn test_core_multi_mount() {
        let temp_dir1 = TempDir::new().unwrap();
        let temp_dir2 = TempDir::new().unwrap();
        let config = make_multi_mount_config(
            temp_dir1.path().to_str().unwrap(),
            temp_dir2.path().to_str().unwrap(),
        );
        let core = AxFsCore::from_config(config).unwrap();

        block_on(async { core.vfs.write("/workspace/file.txt", b"content").await.unwrap() }).unwrap();

        let exists = block_on(async { core.vfs.exists("/workspace/file.txt").await.unwrap() }).unwrap();
        assert!(exists);

        let exists = block_on(async { core.vfs.exists("/docs/file.txt").await.unwrap() }).unwrap();
        assert!(!exists);
    }

    #[test]
    fn test_core_read_only_mount() {
        let temp_dir1 = TempDir::new().unwrap();
        let temp_dir2 = TempDir::new().unwrap();

        std::fs::write(temp_dir2.path().join("existing.txt"), "content").unwrap();

        let config = make_multi_mount_config(
            temp_dir1.path().to_str().unwrap(),
            temp_dir2.path().to_str().unwrap(),
        );
        let core = AxFsCore::from_config(config).unwrap();

        let content = block_on(async { core.vfs.read("/docs/existing.txt").await.unwrap() }).unwrap();
        assert_eq!(content, b"content");

        let result = block_on(async { core.vfs.write("/docs/new.txt", b"content").await }).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_core_empty_config_name() {
        let temp_dir = TempDir::new().unwrap();
        let yaml = format!(
            r#"
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /workspace
    backend: local
"#,
            temp_dir.path().to_str().unwrap()
        );
        let config = VfsConfig::from_yaml(&yaml).unwrap();
        let core = AxFsCore::from_config(config);
        assert!(core.is_ok());
    }
}

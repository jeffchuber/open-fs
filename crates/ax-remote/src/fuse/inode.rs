//! Inode management for FUSE filesystem.
//!
//! Provides bidirectional mapping between Unix inodes and VFS paths.
//! FUSE requires stable inode numbers for directory entries and file handles.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use parking_lot::RwLock;

/// Reserved inode for the root directory.
pub const ROOT_INO: u64 = 1;

/// Reserved inode prefix for virtual directories (like .search).
pub const VIRTUAL_INO_BASE: u64 = 0x1000_0000_0000_0000;

/// Inode attributes matching FUSE requirements.
#[derive(Debug, Clone)]
pub struct InodeAttr {
    /// Inode number.
    pub ino: u64,
    /// Size in bytes.
    pub size: u64,
    /// Number of blocks (512-byte blocks).
    pub blocks: u64,
    /// Access time.
    pub atime: SystemTime,
    /// Modification time.
    pub mtime: SystemTime,
    /// Change time.
    pub ctime: SystemTime,
    /// Creation time.
    pub crtime: SystemTime,
    /// File type and mode.
    pub kind: InodeKind,
    /// Permission mode bits.
    pub perm: u16,
    /// Number of hard links.
    pub nlink: u32,
    /// Owner user ID.
    pub uid: u32,
    /// Owner group ID.
    pub gid: u32,
}

/// Type of inode (file, directory, or symlink).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeKind {
    File,
    Directory,
    Symlink,
}

impl InodeAttr {
    /// Create attributes for a directory.
    pub fn directory(ino: u64) -> Self {
        let now = SystemTime::now();
        InodeAttr {
            ino,
            size: 4096,
            blocks: 8,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: InodeKind::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }
    }

    /// Create attributes for a regular file.
    pub fn file(ino: u64, size: u64) -> Self {
        let now = SystemTime::now();
        InodeAttr {
            ino,
            size,
            blocks: size.div_ceil(512),
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: InodeKind::File,
            perm: 0o644,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }
    }

    /// Create attributes for a symlink.
    pub fn symlink(ino: u64, target_len: u64) -> Self {
        let now = SystemTime::now();
        InodeAttr {
            ino,
            size: target_len,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: InodeKind::Symlink,
            perm: 0o777,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }
    }

    /// Update modification time.
    pub fn touch(&mut self) {
        let now = SystemTime::now();
        self.mtime = now;
        self.ctime = now;
    }

    /// Update size (for files).
    pub fn set_size(&mut self, size: u64) {
        self.size = size;
        self.blocks = size.div_ceil(512);
        self.touch();
    }

    /// Default TTL for attributes.
    pub fn ttl() -> Duration {
        Duration::from_secs(1)
    }
}

/// Inode table managing path-to-inode and inode-to-path mappings.
pub struct InodeTable {
    /// Path to inode mapping.
    path_to_ino: RwLock<HashMap<String, u64>>,
    /// Inode to path mapping.
    ino_to_path: RwLock<HashMap<u64, String>>,
    /// Inode attributes cache.
    attrs: RwLock<HashMap<u64, InodeAttr>>,
    /// Next available inode number.
    next_ino: RwLock<u64>,
}

impl InodeTable {
    /// Create a new inode table with root directory initialized.
    pub fn new() -> Self {
        let table = InodeTable {
            path_to_ino: RwLock::new(HashMap::new()),
            ino_to_path: RwLock::new(HashMap::new()),
            attrs: RwLock::new(HashMap::new()),
            next_ino: RwLock::new(ROOT_INO + 1),
        };

        // Initialize root directory
        {
            let mut path_to_ino = table.path_to_ino.write();
            let mut ino_to_path = table.ino_to_path.write();
            let mut attrs = table.attrs.write();

            path_to_ino.insert("/".to_string(), ROOT_INO);
            ino_to_path.insert(ROOT_INO, "/".to_string());
            attrs.insert(ROOT_INO, InodeAttr::directory(ROOT_INO));
        }

        table
    }

    /// Get or create an inode for a path.
    pub fn get_or_create(&self, path: &str, is_dir: bool, size: u64) -> u64 {
        let normalized = Self::normalize_path(path);

        // Check if already exists
        {
            let path_to_ino = self.path_to_ino.read();
            if let Some(&ino) = path_to_ino.get(&normalized) {
                // Update attributes if needed
                let mut attrs = self.attrs.write();
                if let Some(attr) = attrs.get_mut(&ino) {
                    if !is_dir {
                        attr.set_size(size);
                    }
                }
                return ino;
            }
        }

        // Allocate new inode
        let ino = {
            let mut next = self.next_ino.write();
            let ino = *next;
            *next += 1;
            ino
        };

        // Insert mappings
        {
            let mut path_to_ino = self.path_to_ino.write();
            let mut ino_to_path = self.ino_to_path.write();
            let mut attrs = self.attrs.write();

            path_to_ino.insert(normalized.clone(), ino);
            ino_to_path.insert(ino, normalized);

            let attr = if is_dir {
                InodeAttr::directory(ino)
            } else {
                InodeAttr::file(ino, size)
            };
            attrs.insert(ino, attr);
        }

        ino
    }

    /// Get inode for a path (if exists).
    pub fn get_ino(&self, path: &str) -> Option<u64> {
        let normalized = Self::normalize_path(path);
        let path_to_ino = self.path_to_ino.read();
        path_to_ino.get(&normalized).copied()
    }

    /// Get path for an inode (if exists).
    pub fn get_path(&self, ino: u64) -> Option<String> {
        let ino_to_path = self.ino_to_path.read();
        ino_to_path.get(&ino).cloned()
    }

    /// Get attributes for an inode.
    pub fn get_attr(&self, ino: u64) -> Option<InodeAttr> {
        let attrs = self.attrs.read();
        attrs.get(&ino).cloned()
    }

    /// Update attributes for an inode.
    pub fn update_attr(&self, ino: u64, size: u64, is_dir: bool) {
        let mut attrs = self.attrs.write();
        if let Some(attr) = attrs.get_mut(&ino) {
            if !is_dir {
                attr.set_size(size);
            }
        }
    }

    /// Remove an inode mapping.
    pub fn remove(&self, ino: u64) {
        let path = {
            let ino_to_path = self.ino_to_path.read();
            ino_to_path.get(&ino).cloned()
        };

        if let Some(path) = path {
            let mut path_to_ino = self.path_to_ino.write();
            let mut ino_to_path = self.ino_to_path.write();
            let mut attrs = self.attrs.write();

            path_to_ino.remove(&path);
            ino_to_path.remove(&ino);
            attrs.remove(&ino);
        }
    }

    /// Remove an inode by path.
    pub fn remove_path(&self, path: &str) {
        let normalized = Self::normalize_path(path);
        let ino = {
            let path_to_ino = self.path_to_ino.read();
            path_to_ino.get(&normalized).copied()
        };

        if let Some(ino) = ino {
            self.remove(ino);
        }
    }

    /// Create a symlink inode.
    pub fn create_symlink(&self, path: &str, target: &str) -> u64 {
        let normalized = Self::normalize_path(path);

        // Allocate new inode
        let ino = {
            let mut next = self.next_ino.write();
            let ino = *next;
            *next += 1;
            ino
        };

        // Insert mappings
        {
            let mut path_to_ino = self.path_to_ino.write();
            let mut ino_to_path = self.ino_to_path.write();
            let mut attrs = self.attrs.write();

            path_to_ino.insert(normalized.clone(), ino);
            ino_to_path.insert(ino, normalized);
            attrs.insert(ino, InodeAttr::symlink(ino, target.len() as u64));
        }

        ino
    }

    /// Check if an inode is a virtual inode (for .search directory).
    pub fn is_virtual(&self, ino: u64) -> bool {
        ino >= VIRTUAL_INO_BASE
    }

    /// Allocate a virtual inode number.
    pub fn alloc_virtual_ino(&self) -> u64 {
        let mut next = self.next_ino.write();
        let ino = VIRTUAL_INO_BASE + *next;
        *next += 1;
        ino
    }

    /// Normalize a path for consistent lookup.
    fn normalize_path(path: &str) -> String {
        let mut normalized = path.to_string();

        // Ensure leading slash
        if !normalized.starts_with('/') {
            normalized = format!("/{}", normalized);
        }

        // Remove trailing slash (except for root)
        if normalized.len() > 1 && normalized.ends_with('/') {
            normalized.pop();
        }

        normalized
    }

    /// Resolve a child path from parent inode and name.
    pub fn resolve_child(&self, parent_ino: u64, name: &str) -> Option<String> {
        let parent_path = self.get_path(parent_ino)?;
        let child_path = if parent_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent_path, name)
        };
        Some(child_path)
    }
}

impl Default for InodeTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // ============== InodeAttr Tests ==============

    #[test]
    fn test_inode_attr_directory() {
        let attr = InodeAttr::directory(42);
        assert_eq!(attr.ino, 42);
        assert_eq!(attr.kind, InodeKind::Directory);
        assert_eq!(attr.perm, 0o755);
        assert_eq!(attr.nlink, 2);
        assert_eq!(attr.size, 4096);
        assert_eq!(attr.blocks, 8);
    }

    #[test]
    fn test_inode_attr_file() {
        let attr = InodeAttr::file(42, 1024);
        assert_eq!(attr.ino, 42);
        assert_eq!(attr.kind, InodeKind::File);
        assert_eq!(attr.perm, 0o644);
        assert_eq!(attr.nlink, 1);
        assert_eq!(attr.size, 1024);
        assert_eq!(attr.blocks, 2); // ceil(1024/512) = 2
    }

    #[test]
    fn test_inode_attr_file_block_calculation() {
        // Test various file sizes and block calculations
        let attr = InodeAttr::file(1, 0);
        assert_eq!(attr.blocks, 0);

        let attr = InodeAttr::file(1, 1);
        assert_eq!(attr.blocks, 1);

        let attr = InodeAttr::file(1, 512);
        assert_eq!(attr.blocks, 1);

        let attr = InodeAttr::file(1, 513);
        assert_eq!(attr.blocks, 2);

        let attr = InodeAttr::file(1, 1024);
        assert_eq!(attr.blocks, 2);

        let attr = InodeAttr::file(1, 10000);
        assert_eq!(attr.blocks, 20); // ceil(10000/512) = 20
    }

    #[test]
    fn test_inode_attr_symlink() {
        let attr = InodeAttr::symlink(42, 25);
        assert_eq!(attr.ino, 42);
        assert_eq!(attr.kind, InodeKind::Symlink);
        assert_eq!(attr.perm, 0o777);
        assert_eq!(attr.nlink, 1);
        assert_eq!(attr.size, 25);
        assert_eq!(attr.blocks, 0);
    }

    #[test]
    fn test_inode_attr_touch() {
        let mut attr = InodeAttr::file(1, 100);
        let original_mtime = attr.mtime;

        // Sleep briefly to ensure time difference
        std::thread::sleep(std::time::Duration::from_millis(10));

        attr.touch();
        assert!(attr.mtime > original_mtime);
        assert!(attr.ctime > original_mtime);
    }

    #[test]
    fn test_inode_attr_set_size() {
        let mut attr = InodeAttr::file(1, 100);
        assert_eq!(attr.size, 100);
        assert_eq!(attr.blocks, 1);

        attr.set_size(2048);
        assert_eq!(attr.size, 2048);
        assert_eq!(attr.blocks, 4);
    }

    #[test]
    fn test_inode_attr_ttl() {
        let ttl = InodeAttr::ttl();
        assert_eq!(ttl, Duration::from_secs(1));
    }

    // ============== InodeTable Basic Tests ==============

    #[test]
    fn test_inode_table_root() {
        let table = InodeTable::new();

        assert_eq!(table.get_ino("/"), Some(ROOT_INO));
        assert_eq!(table.get_path(ROOT_INO), Some("/".to_string()));

        let attr = table.get_attr(ROOT_INO).unwrap();
        assert_eq!(attr.kind, InodeKind::Directory);
    }

    #[test]
    fn test_inode_table_default() {
        let table = InodeTable::default();
        assert_eq!(table.get_ino("/"), Some(ROOT_INO));
    }

    #[test]
    fn test_inode_table_get_or_create() {
        let table = InodeTable::new();

        let ino1 = table.get_or_create("/workspace/test.txt", false, 100);
        let ino2 = table.get_or_create("/workspace/test.txt", false, 100);

        // Same path should return same inode
        assert_eq!(ino1, ino2);
        assert_ne!(ino1, ROOT_INO);

        // Should be retrievable
        assert_eq!(table.get_path(ino1), Some("/workspace/test.txt".to_string()));
        assert_eq!(table.get_ino("/workspace/test.txt"), Some(ino1));

        // Check attributes
        let attr = table.get_attr(ino1).unwrap();
        assert_eq!(attr.kind, InodeKind::File);
        assert_eq!(attr.size, 100);
    }

    #[test]
    fn test_inode_table_get_or_create_directory() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/workspace/subdir", true, 0);
        let attr = table.get_attr(ino).unwrap();

        assert_eq!(attr.kind, InodeKind::Directory);
        assert_eq!(attr.size, 4096); // Default dir size
    }

    #[test]
    fn test_inode_table_unique_inodes() {
        let table = InodeTable::new();

        let ino1 = table.get_or_create("/file1.txt", false, 100);
        let ino2 = table.get_or_create("/file2.txt", false, 200);
        let ino3 = table.get_or_create("/dir1", true, 0);

        // All inodes should be unique
        assert_ne!(ino1, ino2);
        assert_ne!(ino2, ino3);
        assert_ne!(ino1, ino3);
        assert_ne!(ino1, ROOT_INO);
    }

    #[test]
    fn test_inode_table_size_update_on_get_or_create() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/test.txt", false, 100);
        let attr = table.get_attr(ino).unwrap();
        assert_eq!(attr.size, 100);

        // Getting with different size should update
        let ino2 = table.get_or_create("/test.txt", false, 500);
        assert_eq!(ino, ino2);

        let attr = table.get_attr(ino).unwrap();
        assert_eq!(attr.size, 500);
    }

    #[test]
    fn test_inode_table_size_not_updated_for_directory() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/mydir", true, 0);
        let attr = table.get_attr(ino).unwrap();
        assert_eq!(attr.size, 4096);

        // Size should not change for directories
        let ino2 = table.get_or_create("/mydir", true, 999);
        assert_eq!(ino, ino2);

        let attr = table.get_attr(ino).unwrap();
        assert_eq!(attr.size, 4096); // Still default
    }

    // ============== Path Normalization Tests ==============

    #[test]
    fn test_inode_table_normalize() {
        let table = InodeTable::new();

        // With and without leading slash should be same
        let ino1 = table.get_or_create("workspace/test.txt", false, 100);
        let ino2 = table.get_or_create("/workspace/test.txt", false, 100);
        assert_eq!(ino1, ino2);

        // Trailing slash should be removed
        let ino3 = table.get_or_create("/workspace/dir/", true, 0);
        let ino4 = table.get_or_create("/workspace/dir", true, 0);
        assert_eq!(ino3, ino4);
    }

    #[test]
    fn test_inode_table_normalize_root() {
        let table = InodeTable::new();

        // Various root representations should all resolve to ROOT_INO
        assert_eq!(table.get_ino("/"), Some(ROOT_INO));
    }

    #[test]
    fn test_inode_table_normalize_complex_paths() {
        let table = InodeTable::new();

        // Deep nesting
        let ino1 = table.get_or_create("/a/b/c/d/e/f.txt", false, 10);
        let ino2 = table.get_or_create("a/b/c/d/e/f.txt", false, 10);
        assert_eq!(ino1, ino2);

        // Multiple trailing slashes (first one removed)
        let ino3 = table.get_or_create("/dir/", true, 0);
        let ino4 = table.get_or_create("/dir", true, 0);
        assert_eq!(ino3, ino4);
    }

    #[test]
    fn test_inode_table_special_characters_in_path() {
        let table = InodeTable::new();

        // Spaces
        let ino1 = table.get_or_create("/path with spaces/file.txt", false, 100);
        assert!(table.get_path(ino1).is_some());

        // Unicode
        let ino2 = table.get_or_create("/\u{6587}\u{4ef6}\u{5939}/\u{6587}\u{4ef6}.txt", false, 100);
        assert!(table.get_path(ino2).is_some());

        // Dots
        let ino3 = table.get_or_create("/path/.hidden", false, 100);
        assert!(table.get_path(ino3).is_some());

        // All different
        assert_ne!(ino1, ino2);
        assert_ne!(ino2, ino3);
    }

    // ============== Remove Tests ==============

    #[test]
    fn test_inode_table_remove() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/workspace/test.txt", false, 100);
        assert!(table.get_path(ino).is_some());

        table.remove(ino);
        assert!(table.get_path(ino).is_none());
        assert!(table.get_ino("/workspace/test.txt").is_none());
    }

    #[test]
    fn test_inode_table_remove_nonexistent() {
        let table = InodeTable::new();

        // Should not panic when removing nonexistent inode
        table.remove(99999);
    }

    #[test]
    fn test_inode_table_remove_path() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/test.txt", false, 100);
        assert!(table.get_attr(ino).is_some());

        table.remove_path("/test.txt");

        assert!(table.get_ino("/test.txt").is_none());
        assert!(table.get_path(ino).is_none());
        assert!(table.get_attr(ino).is_none());
    }

    #[test]
    fn test_inode_table_remove_path_nonexistent() {
        let table = InodeTable::new();

        // Should not panic
        table.remove_path("/nonexistent/path");
    }

    #[test]
    fn test_inode_table_remove_and_recreate() {
        let table = InodeTable::new();

        let ino1 = table.get_or_create("/test.txt", false, 100);
        table.remove(ino1);

        // Creating same path should get new inode
        let ino2 = table.get_or_create("/test.txt", false, 100);
        assert_ne!(ino1, ino2);
    }

    // ============== Update Attr Tests ==============

    #[test]
    fn test_inode_table_update_attr() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/test.txt", false, 100);
        table.update_attr(ino, 500, false);

        let attr = table.get_attr(ino).unwrap();
        assert_eq!(attr.size, 500);
    }

    #[test]
    fn test_inode_table_update_attr_directory_ignored() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/testdir", true, 0);
        let original_size = table.get_attr(ino).unwrap().size;

        // Update with is_dir=true should not change size
        table.update_attr(ino, 999, true);

        let attr = table.get_attr(ino).unwrap();
        assert_eq!(attr.size, original_size);
    }

    #[test]
    fn test_inode_table_update_attr_nonexistent() {
        let table = InodeTable::new();

        // Should not panic
        table.update_attr(99999, 100, false);
    }

    // ============== Symlink Tests ==============

    #[test]
    fn test_inode_table_create_symlink() {
        let table = InodeTable::new();

        let ino = table.create_symlink("/link", "/target/path");
        assert!(table.get_path(ino).is_some());

        let attr = table.get_attr(ino).unwrap();
        assert_eq!(attr.kind, InodeKind::Symlink);
        assert_eq!(attr.size, "/target/path".len() as u64);
    }

    #[test]
    fn test_inode_table_multiple_symlinks() {
        let table = InodeTable::new();

        let ino1 = table.create_symlink("/link1", "/target1");
        let ino2 = table.create_symlink("/link2", "/target2");

        assert_ne!(ino1, ino2);

        let attr1 = table.get_attr(ino1).unwrap();
        let attr2 = table.get_attr(ino2).unwrap();

        assert_eq!(attr1.kind, InodeKind::Symlink);
        assert_eq!(attr2.kind, InodeKind::Symlink);
    }

    // ============== Virtual Inode Tests ==============

    #[test]
    fn test_inode_table_is_virtual() {
        let table = InodeTable::new();

        assert!(!table.is_virtual(ROOT_INO));
        assert!(!table.is_virtual(100));
        assert!(table.is_virtual(VIRTUAL_INO_BASE));
        assert!(table.is_virtual(VIRTUAL_INO_BASE + 1));
    }

    #[test]
    fn test_inode_table_alloc_virtual_ino() {
        let table = InodeTable::new();

        let vino1 = table.alloc_virtual_ino();
        let vino2 = table.alloc_virtual_ino();

        assert!(table.is_virtual(vino1));
        assert!(table.is_virtual(vino2));
        assert_ne!(vino1, vino2);
    }

    #[test]
    fn test_virtual_ino_base_is_large() {
        // Virtual inodes should be in a separate space from regular inodes
        assert!(VIRTUAL_INO_BASE > 1_000_000_000);
    }

    // ============== Resolve Child Tests ==============

    #[test]
    fn test_resolve_child() {
        let table = InodeTable::new();

        // Root child
        let child = table.resolve_child(ROOT_INO, "workspace");
        assert_eq!(child, Some("/workspace".to_string()));

        // Nested child
        let dir_ino = table.get_or_create("/workspace", true, 0);
        let child = table.resolve_child(dir_ino, "test.txt");
        assert_eq!(child, Some("/workspace/test.txt".to_string()));
    }

    #[test]
    fn test_resolve_child_deep_nesting() {
        let table = InodeTable::new();

        let ino = table.get_or_create("/a/b/c", true, 0);
        let child = table.resolve_child(ino, "d");

        assert_eq!(child, Some("/a/b/c/d".to_string()));
    }

    #[test]
    fn test_resolve_child_nonexistent_parent() {
        let table = InodeTable::new();

        let child = table.resolve_child(99999, "test");
        assert_eq!(child, None);
    }

    // ============== Get Methods Tests ==============

    #[test]
    fn test_get_ino_nonexistent() {
        let table = InodeTable::new();
        assert_eq!(table.get_ino("/nonexistent"), None);
    }

    #[test]
    fn test_get_path_nonexistent() {
        let table = InodeTable::new();
        assert_eq!(table.get_path(99999), None);
    }

    #[test]
    fn test_get_attr_nonexistent() {
        let table = InodeTable::new();
        assert!(table.get_attr(99999).is_none());
    }

    // ============== Large Scale Tests ==============

    #[test]
    fn test_inode_table_many_entries() {
        let table = InodeTable::new();

        // Create 1000 files
        let mut inodes = Vec::new();
        for i in 0..1000 {
            let path = format!("/dir/file{}.txt", i);
            let ino = table.get_or_create(&path, false, i as u64 * 100);
            inodes.push((path, ino));
        }

        // Verify all are retrievable
        for (path, ino) in &inodes {
            assert_eq!(table.get_ino(path), Some(*ino));
            assert_eq!(table.get_path(*ino).as_ref(), Some(path));
        }
    }

    #[test]
    fn test_inode_table_deep_hierarchy() {
        let table = InodeTable::new();

        // Create a deep directory hierarchy
        let mut path = String::new();
        let mut inodes = Vec::new();

        for i in 0..50 {
            path.push_str(&format!("/level{}", i));
            let ino = table.get_or_create(&path, true, 0);
            inodes.push((path.clone(), ino));
        }

        // Verify all directories exist
        for (path, ino) in &inodes {
            assert_eq!(table.get_ino(path), Some(*ino));
        }
    }

    // ============== Concurrent Access Tests ==============

    #[test]
    fn test_inode_table_concurrent_reads() {
        use std::sync::Arc;

        let table = Arc::new(InodeTable::new());

        // Create some entries
        for i in 0..100 {
            table.get_or_create(&format!("/file{}.txt", i), false, i as u64);
        }

        // Spawn multiple threads to read
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let table = Arc::clone(&table);
                thread::spawn(move || {
                    for i in 0..100 {
                        let path = format!("/file{}.txt", i);
                        assert!(table.get_ino(&path).is_some());
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_inode_table_concurrent_writes() {
        use std::sync::Arc;

        let table = Arc::new(InodeTable::new());

        // Spawn multiple threads to write
        let handles: Vec<_> = (0..10)
            .map(|t| {
                let table = Arc::clone(&table);
                thread::spawn(move || {
                    for i in 0..100 {
                        let path = format!("/thread{}/file{}.txt", t, i);
                        table.get_or_create(&path, false, i as u64);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all entries exist
        for t in 0..10 {
            for i in 0..100 {
                let path = format!("/thread{}/file{}.txt", t, i);
                assert!(table.get_ino(&path).is_some());
            }
        }
    }

    #[test]
    fn test_inode_table_concurrent_read_write() {
        use std::sync::Arc;

        let table = Arc::new(InodeTable::new());

        // Pre-populate
        for i in 0..50 {
            table.get_or_create(&format!("/existing{}.txt", i), false, i as u64);
        }

        // Mix of readers and writers
        let handles: Vec<_> = (0..10)
            .map(|t| {
                let table = Arc::clone(&table);
                thread::spawn(move || {
                    if t % 2 == 0 {
                        // Reader
                        for i in 0..50 {
                            let _ = table.get_ino(&format!("/existing{}.txt", i));
                        }
                    } else {
                        // Writer
                        for i in 0..50 {
                            table.get_or_create(&format!("/new{}.txt", i + t * 100), false, i as u64);
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }
}

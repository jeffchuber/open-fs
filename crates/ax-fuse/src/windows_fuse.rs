//! Windows FUSE implementation using WinFsp.
//!
//! This module provides a Windows-compatible FUSE driver using the WinFsp
//! (Windows File System Proxy) library via the `winfsp` crate.
//!
//! # Architecture
//!
//! All VFS interaction logic lives in `common::AxFsCore`. This module
//! translates between WinFsp's callback interface and `AxFsCore` operations,
//! mapping `FsOpError` variants to NTSTATUS codes.
//!
//! # Path Handling
//!
//! - Internal VFS paths remain Unix-style (`/mount/file`)
//! - WinFsp presents paths as Windows-style (`\mount\file`)
//! - Conversion happens at the WinFsp boundary in this module
//! - Mount points can be drive letters (`X:`) or UNC paths

use crate::common::{AxFsCore, FsOpError};

/// Windows FUSE filesystem wrapper around `AxFsCore`.
pub struct WindowsFuse(pub AxFsCore);

impl WindowsFuse {
    /// Convert a WinFsp path (backslashes) to a VFS path (forward slashes).
    fn winfsp_to_vfs(path: &str) -> String {
        let vfs_path = path.replace('\\', "/");
        if vfs_path.is_empty() {
            "/".to_string()
        } else if !vfs_path.starts_with('/') {
            format!("/{}", vfs_path)
        } else {
            vfs_path
        }
    }

    /// Convert a VFS path (forward slashes) to a WinFsp path (backslashes).
    fn vfs_to_winfsp(path: &str) -> String {
        path.replace('/', "\\")
    }
}

// Windows FUSE via WinFsp is not yet implemented.
// The `winfsp::FileSystemInterface` trait needs to be implemented for `WindowsFuse`,
// mirroring unix_fuse.rs but translating `FsOpError` to NTSTATUS codes.
// This requires a Windows development environment with WinFsp installed.
#[cfg(windows)]
compile_error!(
    "Windows FUSE support is not yet implemented. \
     The WindowsFuse struct exists for path conversion utilities, \
     but the winfsp::FileSystemInterface trait has not been implemented. \
     Build without the `fuse` feature on Windows."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_winfsp_to_vfs_root() {
        assert_eq!(WindowsFuse::winfsp_to_vfs("\\"), "/");
    }

    #[test]
    fn test_winfsp_to_vfs_file() {
        assert_eq!(
            WindowsFuse::winfsp_to_vfs("\\workspace\\file.txt"),
            "/workspace/file.txt"
        );
    }

    #[test]
    fn test_winfsp_to_vfs_deep_path() {
        assert_eq!(
            WindowsFuse::winfsp_to_vfs("\\a\\b\\c\\d.txt"),
            "/a/b/c/d.txt"
        );
    }

    #[test]
    fn test_winfsp_to_vfs_empty() {
        assert_eq!(WindowsFuse::winfsp_to_vfs(""), "/");
    }

    #[test]
    fn test_vfs_to_winfsp_root() {
        assert_eq!(WindowsFuse::vfs_to_winfsp("/"), "\\");
    }

    #[test]
    fn test_vfs_to_winfsp_file() {
        assert_eq!(
            WindowsFuse::vfs_to_winfsp("/workspace/file.txt"),
            "\\workspace\\file.txt"
        );
    }

    #[test]
    fn test_vfs_to_winfsp_deep_path() {
        assert_eq!(
            WindowsFuse::vfs_to_winfsp("/a/b/c/d.txt"),
            "\\a\\b\\c\\d.txt"
        );
    }

    #[test]
    fn test_path_roundtrip() {
        let original = "/workspace/src/main.rs";
        let windows = WindowsFuse::vfs_to_winfsp(original);
        let back = WindowsFuse::winfsp_to_vfs(&windows);
        assert_eq!(back, original);
    }
}

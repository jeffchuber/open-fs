//! FUSE filesystem implementation for AX VFS.
//!
//! This module provides a FUSE (Filesystem in Userspace) interface to the AX
//! virtual filesystem. It allows mounting the VFS as a native filesystem,
//! enabling transparent access to all AX backends through standard file
//! operations.
//!
//! # Features
//!
//! - **Transparent Integration**: Claude Code and other tools use standard
//!   file operations (read, write, glob, grep) without knowing about AX.
//! - **Per-Mount Sync Strategies**: Different mounts can have different
//!   caching and sync behaviors (WriteThrough, WriteBack, PullMirror).
//! - **Virtual .search Directory**: Semantic search exposed as filesystem
//!   operations through a virtual `/.search/query/` directory.
//!
//! # Architecture
//!
//! The module is split into platform-neutral and platform-specific submodules:
//! - `common` — `AxFsCore` struct with all VFS interaction logic
//! - `unix_fuse` — `fuser::Filesystem` impl for macOS/Linux
//!
//! # Example
//!
//! ```ignore
//! use ax_remote::fuse::AxFuse;
//!
//! // Mount AX VFS
//! let ax = AxFuse::from_config("ax.yaml")?;
//! ax.mount("/mnt/ax")?;
//! ```

mod async_bridge;
pub(crate) mod common;
mod inode;
mod search_dir;
#[cfg(unix)]
pub(crate) mod unix_fuse;

pub use async_bridge::{block_on, init_runtime, spawn, FuseError, FuseResult};
pub use common::{AxFsCore, DirEntry, FsOpError, ReadDirResult};
pub use inode::{InodeAttr, InodeKind, InodeTable, ROOT_INO};
pub use search_dir::{SearchDir, SearchResultEntry, QUERY_DIR_PATH, SEARCH_DIR_PATH};

/// The main FUSE filesystem type.
///
/// This is a type alias for `AxFsCore`, which contains all the platform-neutral
/// filesystem logic. Use `mount()` or `mount_foreground()` to mount it.
pub type AxFuse = AxFsCore;

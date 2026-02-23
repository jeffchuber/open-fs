//! Bridge between synchronous FUSE callbacks and async VFS operations.
//!
//! FUSE callbacks are synchronous, but our VFS operations are async.
//! This module provides utilities to bridge the two worlds safely.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

/// Global tokio runtime for FUSE callbacks.
///
/// Stores the result of runtime creation so that initialization errors are
/// propagated without panicking.
static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

/// Initialize the async runtime for FUSE operations.
///
/// Returns an error if the runtime could not be created.
pub fn init_runtime() -> Result<&'static Runtime, FuseError> {
    let result = RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("openfs-fuse-worker")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    });
    match result {
        Ok(rt) => Ok(rt),
        Err(e) => Err(FuseError::Other(format!(
            "Failed to create FUSE async runtime: {}",
            e
        ))),
    }
}

/// Get the FUSE async runtime, returning an error if not initialized.
pub fn runtime() -> Result<&'static Runtime, FuseError> {
    match RUNTIME.get() {
        Some(Ok(rt)) => Ok(rt),
        Some(Err(e)) => Err(FuseError::Other(format!(
            "FUSE runtime failed to initialize: {}",
            e
        ))),
        None => Err(FuseError::Other(
            "FUSE runtime not initialized - call init_runtime first".to_string(),
        )),
    }
}

/// Run an async operation synchronously in the FUSE runtime.
///
/// This is the primary way to call async VFS methods from FUSE callbacks.
///
/// # Example
/// ```ignore
/// let content = block_on(async {
///     vfs.read(path).await
/// })?;
/// ```
pub fn block_on<F, T>(future: F) -> Result<T, FuseError>
where
    F: Future<Output = T>,
{
    let rt = runtime()?;
    Ok(rt.block_on(future))
}

/// Spawn an async task in the FUSE runtime.
///
/// Use this for fire-and-forget operations like indexing updates.
pub fn spawn<F>(future: F) -> Result<(), FuseError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let rt = runtime()?;
    rt.spawn(future);
    Ok(())
}

/// Result type for FUSE operations.
pub type FuseResult<T> = Result<T, FuseError>;

/// Errors that can occur in FUSE operations.
#[derive(Debug, thiserror::Error)]
pub enum FuseError {
    /// File or directory not found.
    #[error("not found")]
    NotFound,
    /// Permission denied.
    #[error("permission denied")]
    PermissionDenied,
    /// Path is a directory (when file expected).
    #[error("is a directory")]
    IsDir,
    /// Path is not a directory (when directory expected).
    #[error("not a directory")]
    NotDir,
    /// Path already exists.
    #[error("already exists")]
    Exists,
    /// Directory not empty.
    #[error("directory not empty")]
    NotEmpty,
    /// Read-only filesystem.
    #[error("read-only filesystem")]
    ReadOnly,
    /// I/O error.
    #[error("I/O error: {0}")]
    Io(std::io::Error),
    /// Other error.
    #[error("{0}")]
    Other(String),
}

#[cfg(unix)]
impl FuseError {
    /// Convert to a libc errno.
    pub fn to_errno(&self) -> i32 {
        match self {
            FuseError::NotFound => libc::ENOENT,
            FuseError::PermissionDenied => libc::EACCES,
            FuseError::IsDir => libc::EISDIR,
            FuseError::NotDir => libc::ENOTDIR,
            FuseError::Exists => libc::EEXIST,
            FuseError::NotEmpty => libc::ENOTEMPTY,
            FuseError::ReadOnly => libc::EROFS,
            FuseError::Io(e) => e.raw_os_error().unwrap_or(libc::EIO),
            FuseError::Other(_) => libc::EIO,
        }
    }
}

impl From<std::io::Error> for FuseError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => FuseError::NotFound,
            std::io::ErrorKind::PermissionDenied => FuseError::PermissionDenied,
            std::io::ErrorKind::AlreadyExists => FuseError::Exists,
            _ => FuseError::Io(e),
        }
    }
}

impl From<openfs_core::VfsError> for FuseError {
    fn from(e: openfs_core::VfsError) -> Self {
        match e {
            openfs_core::VfsError::NotFound(_) => FuseError::NotFound,
            openfs_core::VfsError::ReadOnly(_) => FuseError::ReadOnly,
            openfs_core::VfsError::NoMount(_) => FuseError::NotFound,
            openfs_core::VfsError::Io(io_err) => FuseError::Io(io_err),
            openfs_core::VfsError::Config(msg) => FuseError::Other(msg),
            openfs_core::VfsError::Backend(e) => FuseError::Other(e.to_string()),
            openfs_core::VfsError::Watch(msg) => FuseError::Other(msg),
            openfs_core::VfsError::Indexing(msg) => FuseError::Other(msg),
            _ => FuseError::Other(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ============== Runtime Tests ==============

    #[test]
    fn test_runtime_init() {
        let rt = init_runtime().unwrap();
        let result = rt.block_on(async { 42 });
        assert_eq!(result, 42);
    }

    #[test]
    fn test_runtime_init_idempotent() {
        // Calling init_runtime multiple times should return same runtime
        let rt1 = init_runtime().unwrap();
        let rt2 = init_runtime().unwrap();

        // Both should work
        let r1 = rt1.block_on(async { 1 });
        let r2 = rt2.block_on(async { 2 });

        assert_eq!(r1, 1);
        assert_eq!(r2, 2);
    }

    #[test]
    fn test_block_on() {
        init_runtime().unwrap();
        let result = block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            "hello"
        })
        .unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_block_on_returns_value() {
        init_runtime().unwrap();

        let result: i32 = block_on(async { 42 }).unwrap();
        assert_eq!(result, 42);

        let result: String = block_on(async { "test".to_string() }).unwrap();
        assert_eq!(result, "test");

        let result: Vec<i32> = block_on(async { vec![1, 2, 3] }).unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn test_block_on_with_result() {
        init_runtime().unwrap();

        let result: Result<i32, &str> = block_on(async { Ok(42) }).unwrap();
        assert_eq!(result, Ok(42));

        let result: Result<i32, &str> = block_on(async { Err("error") }).unwrap();
        assert_eq!(result, Err("error"));
    }

    #[test]
    fn test_block_on_nested_await() {
        init_runtime().unwrap();

        let result = block_on(async {
            let a = async { 1 }.await;
            let b = async { 2 }.await;
            let c = async { 3 }.await;
            a + b + c
        })
        .unwrap();

        assert_eq!(result, 6);
    }

    #[test]
    fn test_spawn_executes() {
        init_runtime().unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        spawn(async move {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        // Give it time to execute
        std::thread::sleep(std::time::Duration::from_millis(50));

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_spawn_multiple() {
        init_runtime().unwrap();

        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..10 {
            let counter_clone = Arc::clone(&counter);
            spawn(async move {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
        }

        // Give them time to execute
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    // ============== FuseError Tests ==============

    #[cfg(unix)]
    #[test]
    fn test_fuse_error_to_errno() {
        assert_eq!(FuseError::NotFound.to_errno(), libc::ENOENT);
        assert_eq!(FuseError::PermissionDenied.to_errno(), libc::EACCES);
        assert_eq!(FuseError::ReadOnly.to_errno(), libc::EROFS);
    }

    #[cfg(unix)]
    #[test]
    fn test_fuse_error_to_errno_all_variants() {
        assert_eq!(FuseError::NotFound.to_errno(), libc::ENOENT);
        assert_eq!(FuseError::PermissionDenied.to_errno(), libc::EACCES);
        assert_eq!(FuseError::IsDir.to_errno(), libc::EISDIR);
        assert_eq!(FuseError::NotDir.to_errno(), libc::ENOTDIR);
        assert_eq!(FuseError::Exists.to_errno(), libc::EEXIST);
        assert_eq!(FuseError::NotEmpty.to_errno(), libc::ENOTEMPTY);
        assert_eq!(FuseError::ReadOnly.to_errno(), libc::EROFS);
        assert_eq!(FuseError::Other("test".to_string()).to_errno(), libc::EIO);
    }

    #[cfg(unix)]
    #[test]
    fn test_fuse_error_io_with_raw_os_error() {
        let io_err = std::io::Error::from_raw_os_error(libc::ENOSPC);
        let fuse_err = FuseError::Io(io_err);
        assert_eq!(fuse_err.to_errno(), libc::ENOSPC);
    }

    #[cfg(unix)]
    #[test]
    fn test_fuse_error_io_without_raw_os_error() {
        let io_err = std::io::Error::other("custom error");
        let fuse_err = FuseError::Io(io_err);
        assert_eq!(fuse_err.to_errno(), libc::EIO);
    }

    #[test]
    fn test_fuse_error_debug() {
        // Ensure Debug is implemented
        let err = FuseError::NotFound;
        let _ = format!("{:?}", err);

        let err = FuseError::Other("test message".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("test message"));
    }

    // ============== From<std::io::Error> Tests ==============

    #[test]
    fn test_from_io_error_not_found() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let fuse_err = FuseError::from(io_err);
        assert!(matches!(fuse_err, FuseError::NotFound));
    }

    #[test]
    fn test_from_io_error_permission_denied() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let fuse_err = FuseError::from(io_err);
        assert!(matches!(fuse_err, FuseError::PermissionDenied));
    }

    #[test]
    fn test_from_io_error_already_exists() {
        let io_err = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exists");
        let fuse_err = FuseError::from(io_err);
        assert!(matches!(fuse_err, FuseError::Exists));
    }

    #[test]
    fn test_from_io_error_other() {
        let io_err = std::io::Error::other("other");
        let fuse_err = FuseError::from(io_err);
        assert!(matches!(fuse_err, FuseError::Io(_)));
    }

    #[test]
    fn test_from_io_error_various_kinds() {
        // These should all map to FuseError::Io
        let kinds = [
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::UnexpectedEof,
        ];

        for kind in kinds {
            let io_err = std::io::Error::new(kind, "test");
            let fuse_err = FuseError::from(io_err);
            assert!(
                matches!(fuse_err, FuseError::Io(_)),
                "Failed for kind: {:?}",
                kind
            );
        }
    }

    // ============== From<VfsError> Tests ==============

    #[test]
    fn test_from_vfs_error_not_found() {
        let vfs_err = openfs_core::VfsError::NotFound("/test".to_string());
        let fuse_err = FuseError::from(vfs_err);
        assert!(matches!(fuse_err, FuseError::NotFound));
    }

    #[test]
    fn test_from_vfs_error_read_only() {
        let vfs_err = openfs_core::VfsError::ReadOnly("/test".to_string());
        let fuse_err = FuseError::from(vfs_err);
        assert!(matches!(fuse_err, FuseError::ReadOnly));
    }

    #[test]
    fn test_from_vfs_error_no_mount() {
        let vfs_err = openfs_core::VfsError::NoMount("/test".to_string());
        let fuse_err = FuseError::from(vfs_err);
        assert!(matches!(fuse_err, FuseError::NotFound));
    }

    #[test]
    fn test_from_vfs_error_config() {
        let vfs_err = openfs_core::VfsError::Config("config error".to_string());
        let fuse_err = FuseError::from(vfs_err);
        assert!(matches!(fuse_err, FuseError::Other(_)));
    }

    #[test]
    fn test_from_vfs_error_io() {
        let io_err = std::io::Error::other("io error");
        let vfs_err = openfs_core::VfsError::Io(io_err);
        let fuse_err = FuseError::from(vfs_err);
        assert!(matches!(fuse_err, FuseError::Io(_)));
    }

    // ============== FuseResult Tests ==============

    #[test]
    fn test_fuse_result_ok() {
        let result: FuseResult<i32> = Ok(42);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_fuse_result_err() {
        let result: FuseResult<i32> = Err(FuseError::NotFound);
        assert!(result.is_err());
    }

    #[test]
    fn test_fuse_result_map() {
        let result: FuseResult<i32> = Ok(21);
        let doubled = result.map(|x| x * 2);
        assert_eq!(doubled.unwrap(), 42);
    }

    #[cfg(unix)]
    #[test]
    fn test_fuse_result_map_err() {
        let result: FuseResult<i32> = Err(FuseError::NotFound);
        let mapped = result.map_err(|e| e.to_errno());
        assert_eq!(mapped.unwrap_err(), libc::ENOENT);
    }

    // ============== Concurrency Tests ==============

    #[test]
    fn test_block_on_concurrent_calls() {
        use std::thread;

        init_runtime().unwrap();

        let handles: Vec<_> = (0..10)
            .map(|i| {
                thread::spawn(move || {
                    block_on(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        i
                    })
                    .unwrap()
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // All threads should have completed with their values
        for (i, result) in results.into_iter().enumerate() {
            assert_eq!(result, i);
        }
    }

    #[test]
    fn test_spawn_and_block_on_interleaved() {
        init_runtime().unwrap();

        let counter = Arc::new(AtomicUsize::new(0));

        // Spawn some background tasks
        for _ in 0..5 {
            let counter_clone = Arc::clone(&counter);
            spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                counter_clone.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
        }

        // Do some blocking operations
        for _ in 0..5 {
            block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            })
            .unwrap();
        }

        // Wait for spawned tasks
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }
}

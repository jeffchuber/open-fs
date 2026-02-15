//! Backend Conformance Test Suite
//!
//! A single `run_conformance` function exercises all Backend trait operations
//! against any implementation, ensuring behavioral consistency across backends.

use ax_backends::{Backend, BackendError, Entry, FsBackend, MemoryBackend};

/// Run the full conformance suite against any backend implementation.
async fn run_conformance(backend: &dyn Backend) {
    // 1. Write + Read
    backend.write("hello.txt", b"hello world").await.unwrap();
    let content = backend.read("hello.txt").await.unwrap();
    assert_eq!(content, b"hello world");

    // 2. Read nonexistent → NotFound
    let err = backend.read("nonexistent.txt").await.unwrap_err();
    assert!(
        matches!(err, BackendError::NotFound(_)),
        "Expected NotFound, got: {:?}",
        err
    );

    // 3. Overwrite
    backend.write("hello.txt", b"overwritten").await.unwrap();
    let content = backend.read("hello.txt").await.unwrap();
    assert_eq!(content, b"overwritten");

    // 4. Delete
    backend.delete("hello.txt").await.unwrap();
    let exists = backend.exists("hello.txt").await.unwrap();
    assert!(!exists, "File should not exist after delete");

    // 5. Delete nonexistent → NotFound
    let err = backend.delete("nonexistent.txt").await.unwrap_err();
    assert!(
        matches!(err, BackendError::NotFound(_)),
        "Expected NotFound on delete, got: {:?}",
        err
    );

    // 6. Append to existing file
    backend.write("append.txt", b"first").await.unwrap();
    backend.append("append.txt", b" second").await.unwrap();
    let content = backend.read("append.txt").await.unwrap();
    assert_eq!(content, b"first second");

    // 7. Append to new file (creates it)
    backend.append("new-append.txt", b"created").await.unwrap();
    let content = backend.read("new-append.txt").await.unwrap();
    assert_eq!(content, b"created");

    // 8. List empty directory
    let entries = backend.list("empty-dir-that-has-no-files").await;
    // Backends may return Ok([]) or an error for non-existent directories
    if let Ok(entries) = entries {
        assert!(entries.is_empty());
    }

    // 9. List with files and dirs (dirs first)
    backend
        .write("listdir/file1.txt", b"content1")
        .await
        .unwrap();
    backend
        .write("listdir/file2.txt", b"content2")
        .await
        .unwrap();
    backend
        .write("listdir/subdir/nested.txt", b"nested")
        .await
        .unwrap();

    let entries = backend.list("listdir").await.unwrap();
    assert!(entries.len() >= 3, "Expected at least 3 entries, got {}", entries.len());

    // Directories should appear before files
    let first_dir_idx = entries.iter().position(|e| e.is_dir);
    let last_file_idx = entries.iter().rposition(|e| !e.is_dir);
    if let (Some(dir_idx), Some(file_idx)) = (first_dir_idx, last_file_idx) {
        assert!(
            dir_idx < file_idx,
            "Directories should be sorted before files"
        );
    }

    // 10. Exists (file + directory)
    assert!(backend.exists("listdir/file1.txt").await.unwrap());
    assert!(backend.exists("listdir/subdir").await.unwrap());
    assert!(!backend.exists("does-not-exist.txt").await.unwrap());

    // 11. Stat file
    let stat = backend.stat("listdir/file1.txt").await.unwrap();
    assert_eq!(stat.name, "file1.txt");
    assert!(!stat.is_dir);
    assert_eq!(stat.size, Some(8)); // "content1" = 8 bytes

    // 12. Rename
    backend.write("rename-src.txt", b"rename me").await.unwrap();
    backend
        .rename("rename-src.txt", "rename-dst.txt")
        .await
        .unwrap();
    assert!(!backend.exists("rename-src.txt").await.unwrap());
    assert!(backend.exists("rename-dst.txt").await.unwrap());
    let content = backend.read("rename-dst.txt").await.unwrap();
    assert_eq!(content, b"rename me");
}

#[tokio::test]
async fn test_memory_backend_conformance() {
    let backend = MemoryBackend::new();
    run_conformance(&backend).await;
}

#[tokio::test]
async fn test_fs_backend_conformance() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = FsBackend::new(temp_dir.path()).unwrap();
    run_conformance(&backend).await;
}

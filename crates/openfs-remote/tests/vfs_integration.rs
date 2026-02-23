//! Multi-Mount VFS Integration Tests
//!
//! Tests the VFS layer with multiple mounts pointing at different backends,
//! exercising routing isolation, read-only enforcement, and cross-mount operations.

use openfs_config::VfsConfig;
use openfs_core::VfsError;
use openfs_remote::Vfs;
use tempfile::TempDir;

/// Helper to create a VFS with two fs mounts.
async fn make_dual_fs_vfs(tmp1: &TempDir, tmp2: &TempDir) -> Vfs {
    let yaml = format!(
        r#"
name: dual-mount-test
backends:
  fs1:
    type: fs
    root: {}
  fs2:
    type: fs
    root: {}
mounts:
  - path: /alpha
    backend: fs1
  - path: /beta
    backend: fs2
"#,
        tmp1.path().to_str().unwrap(),
        tmp2.path().to_str().unwrap()
    );
    let config = VfsConfig::from_yaml(&yaml).unwrap();
    Vfs::from_config(config).await.unwrap()
}

#[tokio::test]
async fn test_multi_mount_routing_isolation() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let vfs = make_dual_fs_vfs(&tmp1, &tmp2).await;

    // Write to /alpha, should not be visible in /beta
    vfs.write("/alpha/file.txt", b"alpha content")
        .await
        .unwrap();
    assert!(vfs.exists("/alpha/file.txt").await.unwrap());
    assert!(!vfs.exists("/beta/file.txt").await.unwrap());

    // Write to /beta, should not be visible in /alpha
    vfs.write("/beta/file.txt", b"beta content").await.unwrap();
    assert!(vfs.exists("/beta/file.txt").await.unwrap());

    // Content is distinct
    let alpha = vfs.read("/alpha/file.txt").await.unwrap();
    let beta = vfs.read("/beta/file.txt").await.unwrap();
    assert_eq!(alpha, b"alpha content");
    assert_eq!(beta, b"beta content");
}

#[tokio::test]
async fn test_read_only_mount_enforcement() {
    let tmp = TempDir::new().unwrap();
    let yaml = format!(
        r#"
name: readonly-test
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /ro
    backend: local
    read_only: true
"#,
        tmp.path().to_str().unwrap()
    );
    let config = VfsConfig::from_yaml(&yaml).unwrap();
    let vfs = Vfs::from_config(config).await.unwrap();

    // Write should fail
    let err = vfs.write("/ro/test.txt", b"nope").await.unwrap_err();
    assert!(matches!(err, VfsError::ReadOnly(_)));

    // Append should fail
    let err = vfs.append("/ro/test.txt", b"nope").await.unwrap_err();
    assert!(matches!(err, VfsError::ReadOnly(_)));

    // Delete should fail
    let err = vfs.delete("/ro/test.txt").await.unwrap_err();
    assert!(matches!(err, VfsError::ReadOnly(_)));
}

#[tokio::test]
async fn test_cross_mount_rename() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let vfs = make_dual_fs_vfs(&tmp1, &tmp2).await;

    // Write to /alpha
    vfs.write("/alpha/moved.txt", b"cross mount data")
        .await
        .unwrap();

    // Rename across mounts (different backends) uses copy+delete
    vfs.rename("/alpha/moved.txt", "/beta/moved.txt")
        .await
        .unwrap();

    assert!(!vfs.exists("/alpha/moved.txt").await.unwrap());
    assert!(vfs.exists("/beta/moved.txt").await.unwrap());
    let content = vfs.read("/beta/moved.txt").await.unwrap();
    assert_eq!(content, b"cross mount data");
}

#[tokio::test]
async fn test_same_mount_rename() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let vfs = make_dual_fs_vfs(&tmp1, &tmp2).await;

    vfs.write("/alpha/old.txt", b"same mount").await.unwrap();
    vfs.rename("/alpha/old.txt", "/alpha/new.txt")
        .await
        .unwrap();

    assert!(!vfs.exists("/alpha/old.txt").await.unwrap());
    assert!(vfs.exists("/alpha/new.txt").await.unwrap());
    let content = vfs.read("/alpha/new.txt").await.unwrap();
    assert_eq!(content, b"same mount");
}

#[tokio::test]
async fn test_no_mount_error() {
    let tmp = TempDir::new().unwrap();
    let yaml = format!(
        r#"
name: no-mount-test
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /only
    backend: local
"#,
        tmp.path().to_str().unwrap()
    );
    let config = VfsConfig::from_yaml(&yaml).unwrap();
    let vfs = Vfs::from_config(config).await.unwrap();

    let err = vfs.read("/unmounted/file.txt").await.unwrap_err();
    assert!(matches!(err, VfsError::NoMount(_)));
}

#[tokio::test]
async fn test_memory_backend_via_config() {
    let yaml = r#"
name: memory-test
backends:
  mem:
    type: memory
mounts:
  - path: /mem
    backend: mem
"#;
    let config = VfsConfig::from_yaml(yaml).unwrap();
    let vfs = Vfs::from_config(config).await.unwrap();

    vfs.write("/mem/test.txt", b"in memory").await.unwrap();
    let content = vfs.read("/mem/test.txt").await.unwrap();
    assert_eq!(content, b"in memory");

    let entries = vfs.list("/mem").await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "test.txt");
}

#[tokio::test]
async fn test_mixed_fs_memory_backends() {
    let tmp = TempDir::new().unwrap();
    let yaml = format!(
        r#"
name: mixed-test
backends:
  disk:
    type: fs
    root: {}
  mem:
    type: memory
mounts:
  - path: /disk
    backend: disk
  - path: /mem
    backend: mem
"#,
        tmp.path().to_str().unwrap()
    );
    let config = VfsConfig::from_yaml(&yaml).unwrap();
    let vfs = Vfs::from_config(config).await.unwrap();

    // Write to both
    vfs.write("/disk/a.txt", b"on disk").await.unwrap();
    vfs.write("/mem/b.txt", b"in memory").await.unwrap();

    // Read from both
    assert_eq!(vfs.read("/disk/a.txt").await.unwrap(), b"on disk");
    assert_eq!(vfs.read("/mem/b.txt").await.unwrap(), b"in memory");

    // Isolation
    assert!(!vfs.exists("/disk/b.txt").await.unwrap());
    assert!(!vfs.exists("/mem/a.txt").await.unwrap());
}

#[tokio::test]
async fn test_overlapping_mount_paths_rejected() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let yaml = format!(
        r#"
name: nested-mount-test
backends:
  broad:
    type: fs
    root: {}
  narrow:
    type: fs
    root: {}
mounts:
  - path: /data
    backend: broad
  - path: /data/special
    backend: narrow
"#,
        tmp1.path().to_str().unwrap(),
        tmp2.path().to_str().unwrap()
    );
    let config = VfsConfig::from_yaml(&yaml).unwrap();

    // The validator correctly rejects overlapping mount paths
    let result = Vfs::from_config(config).await;
    assert!(
        result.is_err(),
        "Overlapping mount paths should be rejected"
    );
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("Overlapping mount paths") || err.contains("validation error"),
        "Expected overlapping mount path error, got: {}",
        err
    );
}

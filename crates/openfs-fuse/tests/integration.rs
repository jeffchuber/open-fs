#![cfg(not(target_os = "macos"))]
//! Integration tests for openfs-fuse.
//!
//! These tests verify the complete FUSE filesystem behavior
//! with a real VFS backend.

use std::sync::Arc;
use std::thread;

use openfs_config::VfsConfig;
use openfs_fuse::{block_on, init_runtime, OpenFsFuse, InodeAttr, InodeTable, SearchDir};
use tempfile::TempDir;

// ============== Test Helpers ==============

fn make_config(root: &str) -> VfsConfig {
    let yaml = format!(
        r#"
name: integration-test
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

fn make_multi_mount_config(root1: &str, root2: &str, root3: &str) -> VfsConfig {
    let yaml = format!(
        r#"
name: multi-mount-test
backends:
  local1:
    type: fs
    root: {}
  local2:
    type: fs
    root: {}
  local3:
    type: fs
    root: {}
mounts:
  - path: /workspace
    backend: local1
  - path: /docs
    backend: local2
    read_only: true
  - path: /cache
    backend: local3
"#,
        root1, root2, root3
    );
    VfsConfig::from_yaml(&yaml).unwrap()
}

// ============== OpenFsFuse Integration Tests ==============

#[test]
fn test_axfuse_full_lifecycle() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Write
    block_on(async {
        ax.vfs.write("/workspace/test.txt", b"hello").await.unwrap();
    })
    .unwrap();

    // Read
    let content = block_on(async { ax.vfs.read("/workspace/test.txt").await.unwrap() }).unwrap();
    assert_eq!(content, b"hello");

    // Update
    block_on(async {
        ax.vfs
            .write("/workspace/test.txt", b"hello world")
            .await
            .unwrap();
    })
    .unwrap();

    let content = block_on(async { ax.vfs.read("/workspace/test.txt").await.unwrap() }).unwrap();
    assert_eq!(content, b"hello world");

    // Delete
    block_on(async {
        ax.vfs.delete("/workspace/test.txt").await.unwrap();
    })
    .unwrap();

    let exists = block_on(async { ax.vfs.exists("/workspace/test.txt").await.unwrap() }).unwrap();
    assert!(!exists);
}

#[test]
fn test_axfuse_nested_directories() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Create deeply nested structure
    let paths = [
        "/workspace/a/file.txt",
        "/workspace/a/b/file.txt",
        "/workspace/a/b/c/file.txt",
        "/workspace/a/b/c/d/file.txt",
        "/workspace/a/b/c/d/e/file.txt",
    ];

    for (i, path) in paths.iter().enumerate() {
        let content = format!("content at level {}", i);
        block_on(async {
            ax.vfs.write(path, content.as_bytes()).await.unwrap();
        })
        .unwrap();
    }

    // Verify all files
    for (i, path) in paths.iter().enumerate() {
        let expected = format!("content at level {}", i);
        let content = block_on(async { ax.vfs.read(path).await.unwrap() }).unwrap();
        assert_eq!(String::from_utf8(content).unwrap(), expected);
    }

    // Verify directory structure
    let entries = block_on(async { ax.vfs.list("/workspace/a/b/c").await.unwrap() }).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"d"));
    assert!(names.contains(&"file.txt"));
}

#[test]
fn test_axfuse_multi_mount_isolation() {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();
    let temp_dir3 = TempDir::new().unwrap();

    let config = make_multi_mount_config(
        temp_dir1.path().to_str().unwrap(),
        temp_dir2.path().to_str().unwrap(),
        temp_dir3.path().to_str().unwrap(),
    );

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Write to workspace
    block_on(async {
        ax.vfs
            .write("/workspace/file.txt", b"workspace content")
            .await
            .unwrap();
    })
    .unwrap();

    // Write to cache
    block_on(async {
        ax.vfs
            .write("/cache/file.txt", b"cache content")
            .await
            .unwrap();
    })
    .unwrap();

    // Verify isolation
    let ws_content = block_on(async { ax.vfs.read("/workspace/file.txt").await.unwrap() }).unwrap();
    let cache_content = block_on(async { ax.vfs.read("/cache/file.txt").await.unwrap() }).unwrap();

    assert_eq!(ws_content, b"workspace content");
    assert_eq!(cache_content, b"cache content");

    // Verify docs is read-only (can't write)
    let result = block_on(async { ax.vfs.write("/docs/file.txt", b"content").await }).unwrap();
    assert!(result.is_err());
}

#[test]
fn test_axfuse_no_mount_error() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Access path outside any mount
    let result = block_on(async { ax.vfs.read("/nonexistent/file.txt").await }).unwrap();
    assert!(result.is_err());
}

#[test]
fn test_axfuse_file_not_found_error() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Try to read nonexistent file
    let result = block_on(async { ax.vfs.read("/workspace/nonexistent.txt").await }).unwrap();
    assert!(result.is_err());
}

#[test]
fn test_axfuse_stat_file_and_directory() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    block_on(async {
        ax.vfs
            .write("/workspace/dir/file.txt", b"content")
            .await
            .unwrap();
    })
    .unwrap();

    // Stat file
    let file_entry =
        block_on(async { ax.vfs.stat("/workspace/dir/file.txt").await.unwrap() }).unwrap();
    assert_eq!(file_entry.name, "file.txt");
    assert!(!file_entry.is_dir);
    assert_eq!(file_entry.size, Some(7));

    // Stat directory
    let dir_entry = block_on(async { ax.vfs.stat("/workspace/dir").await.unwrap() }).unwrap();
    assert_eq!(dir_entry.name, "dir");
    assert!(dir_entry.is_dir);
}

#[test]
fn test_axfuse_append_operations() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Initial write
    block_on(async {
        ax.vfs
            .write("/workspace/append.txt", b"Hello")
            .await
            .unwrap();
    })
    .unwrap();

    // Append multiple times
    for i in 0..5 {
        let append_content = format!(" {}", i);
        block_on(async {
            ax.vfs
                .append("/workspace/append.txt", append_content.as_bytes())
                .await
                .unwrap();
        })
        .unwrap();
    }

    let content = block_on(async { ax.vfs.read("/workspace/append.txt").await.unwrap() }).unwrap();
    assert_eq!(String::from_utf8(content).unwrap(), "Hello 0 1 2 3 4");
}

#[test]
fn test_axfuse_binary_files() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // All possible byte values
    let binary_data: Vec<u8> = (0..=255).collect();

    block_on(async {
        ax.vfs
            .write("/workspace/binary.bin", &binary_data)
            .await
            .unwrap();
    })
    .unwrap();

    let content = block_on(async { ax.vfs.read("/workspace/binary.bin").await.unwrap() }).unwrap();
    assert_eq!(content, binary_data);
}

#[test]
fn test_axfuse_unicode_filenames() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    let unicode_files = [
        "/workspace/文件.txt",
        "/workspace/файл.txt",
        "/workspace/αρχείο.txt",
        "/workspace/ファイル.txt",
    ];

    for (i, path) in unicode_files.iter().enumerate() {
        let content = format!("content {}", i);
        block_on(async {
            ax.vfs.write(path, content.as_bytes()).await.unwrap();
        })
        .unwrap();
    }

    for (i, path) in unicode_files.iter().enumerate() {
        let expected = format!("content {}", i);
        let content = block_on(async { ax.vfs.read(path).await.unwrap() }).unwrap();
        assert_eq!(String::from_utf8(content).unwrap(), expected);
    }
}

#[test]
fn test_axfuse_special_characters_in_names() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    let special_files = vec![
        "/workspace/file with spaces.txt",
        "/workspace/file-with-dashes.txt",
        "/workspace/file_with_underscores.txt",
        "/workspace/file.multiple.dots.txt",
        "/workspace/.hidden",
        "/workspace/file@symbol.txt",
        "/workspace/file#hash.txt",
    ];

    for path in &special_files {
        block_on(async {
            ax.vfs.write(path, b"content").await.unwrap();
        })
        .unwrap();

        let content = block_on(async { ax.vfs.read(path).await.unwrap() }).unwrap();
        assert_eq!(content, b"content");
    }
}

#[test]
fn test_axfuse_empty_directory_listing() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    let entries = block_on(async { ax.vfs.list("/workspace").await.unwrap() }).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn test_axfuse_large_directory_listing() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Create 500 files
    for i in 0..500 {
        let path = format!("/workspace/file_{:04}.txt", i);
        block_on(async {
            ax.vfs.write(&path, b"content").await.unwrap();
        })
        .unwrap();
    }

    let entries = block_on(async { ax.vfs.list("/workspace").await.unwrap() }).unwrap();
    assert_eq!(entries.len(), 500);
}

#[test]
fn test_axfuse_overwrite_with_smaller_content() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Write large content
    let large_content = "x".repeat(1000);
    block_on(async {
        ax.vfs
            .write("/workspace/shrink.txt", large_content.as_bytes())
            .await
            .unwrap();
    })
    .unwrap();

    // Overwrite with smaller content
    block_on(async {
        ax.vfs
            .write("/workspace/shrink.txt", b"small")
            .await
            .unwrap();
    })
    .unwrap();

    let content = block_on(async { ax.vfs.read("/workspace/shrink.txt").await.unwrap() }).unwrap();
    assert_eq!(content, b"small");
}

#[test]
fn test_axfuse_overwrite_with_larger_content() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    // Write small content
    block_on(async {
        ax.vfs.write("/workspace/grow.txt", b"small").await.unwrap();
    })
    .unwrap();

    // Overwrite with larger content
    let large_content = "x".repeat(1000);
    block_on(async {
        ax.vfs
            .write("/workspace/grow.txt", large_content.as_bytes())
            .await
            .unwrap();
    })
    .unwrap();

    let content = block_on(async { ax.vfs.read("/workspace/grow.txt").await.unwrap() }).unwrap();
    assert_eq!(content.len(), 1000);
}

// ============== Inode Table Stress Tests ==============

#[test]
fn test_inode_table_stress_many_files() {
    let table = InodeTable::new();

    // Create 10000 files
    for i in 0..10000 {
        let path = format!("/dir/file_{}.txt", i);
        table.get_or_create(&path, false, i as u64);
    }

    // Verify all exist
    for i in 0..10000 {
        let path = format!("/dir/file_{}.txt", i);
        assert!(table.get_ino(&path).is_some());
    }
}

#[test]
fn test_inode_table_stress_deep_hierarchy() {
    let table = InodeTable::new();

    // Create 100-level deep hierarchy
    let mut path = String::new();
    for i in 0..100 {
        path.push_str(&format!("/level{}", i));
        table.get_or_create(&path, true, 0);
    }

    // Verify all exist
    path.clear();
    for i in 0..100 {
        path.push_str(&format!("/level{}", i));
        assert!(table.get_ino(&path).is_some());
    }
}

#[test]
fn test_inode_table_concurrent_stress() {
    use std::sync::Arc;

    let table = Arc::new(InodeTable::new());
    let mut handles = vec![];

    // 20 threads, each creating 1000 files
    for t in 0..20 {
        let table = Arc::clone(&table);
        handles.push(thread::spawn(move || {
            for i in 0..1000 {
                let path = format!("/thread{}/file{}.txt", t, i);
                table.get_or_create(&path, false, i as u64);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all 20000 files exist
    for t in 0..20 {
        for i in 0..1000 {
            let path = format!("/thread{}/file{}.txt", t, i);
            assert!(table.get_ino(&path).is_some());
        }
    }
}

#[test]
fn test_inode_table_remove_stress() {
    let table = InodeTable::new();

    // Create and remove many times
    for round in 0..100 {
        for i in 0..100 {
            let path = format!("/round{}/file{}.txt", round, i);
            let ino = table.get_or_create(&path, false, 0);
            table.remove(ino);
        }
    }

    // All should be removed
    for round in 0..100 {
        for i in 0..100 {
            let path = format!("/round{}/file{}.txt", round, i);
            assert!(table.get_ino(&path).is_none());
        }
    }
}

// ============== Search Directory Tests ==============

#[test]
fn test_search_dir_many_queries() {
    let inodes = Arc::new(InodeTable::new());
    let search_dir = SearchDir::new(inodes.clone());

    // Store 100 different queries
    for q in 0..100 {
        let results: Vec<_> = (0..10)
            .map(|i| {
                (
                    format!("/workspace/q{}_file{}.py", q, i),
                    "content".to_string(),
                    0.9 - (i as f32 * 0.01),
                    1,
                    10,
                )
            })
            .collect();

        let entries = search_dir.create_result_entries(&results);
        search_dir.store_results(&format!("query{}", q), entries);
    }

    // Verify all queries accessible
    for q in 0..100 {
        let query_path = format!("/.search/query/query{}", q);
        let entries = search_dir.readdir(&query_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "results.txt");
    }
}

#[test]
fn test_search_dir_large_result_set() {
    let inodes = Arc::new(InodeTable::new());
    let search_dir = SearchDir::new(inodes.clone());

    // Create query with 1000 results
    let results: Vec<_> = (0..1000)
        .map(|i| {
            (
                format!("/workspace/file{}.py", i),
                "content".to_string(),
                0.9,
                1,
                10,
            )
        })
        .collect();

    let entries = search_dir.create_result_entries(&results);
    assert_eq!(entries.len(), 1);

    search_dir.store_results("big_query", entries);

    let dir_entries = search_dir.readdir("/.search/query/big_query").unwrap();
    assert_eq!(dir_entries.len(), 1);
    assert_eq!(dir_entries[0].1, "results.txt");

    let bytes = search_dir
        .read_file("/.search/query/big_query/results.txt")
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("/workspace/file0.py"));
    assert!(text.contains("/workspace/file999.py"));
}

#[test]
fn test_search_dir_query_update() {
    let inodes = Arc::new(InodeTable::new());
    let search_dir = SearchDir::new(inodes.clone());

    // Initial results
    let results1 = vec![("/file1.py".to_string(), "content".to_string(), 0.9, 1, 10)];
    search_dir.store_results("query", search_dir.create_result_entries(&results1));

    // Update with new results
    let results2 = vec![
        ("/file2.py".to_string(), "content".to_string(), 0.8, 1, 10),
        ("/file3.py".to_string(), "content".to_string(), 0.7, 1, 10),
    ];
    search_dir.store_results("query", search_dir.create_result_entries(&results2));

    // Should have new results
    let entries = search_dir.readdir("/.search/query/query").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1, "results.txt");

    let bytes = search_dir
        .read_file("/.search/query/query/results.txt")
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("/file2.py"));
    assert!(text.contains("/file3.py"));
    assert!(!text.contains("/file1.py"));
}

#[test]
fn test_search_dir_concatenated_results_content() {
    let inodes = Arc::new(InodeTable::new());
    let search_dir = SearchDir::new(inodes.clone());

    let results = vec![
        (
            "/workspace/src/auth/login.py".to_string(),
            "login content".to_string(),
            0.9,
            10,
            20,
        ),
        (
            "/workspace/tests/test_auth.py".to_string(),
            "test content".to_string(),
            0.8,
            5,
            15,
        ),
    ];

    let entries = search_dir.create_result_entries(&results);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "results.txt");
    assert!(entries[0]
        .content
        .contains("/workspace/src/auth/login.py:10-20"));
    assert!(entries[0].content.contains("login content"));
    assert!(entries[0]
        .content
        .contains("/workspace/tests/test_auth.py:5-15"));
    assert!(entries[0].content.contains("test content"));
}

#[test]
fn test_search_dir_encoded_query_roundtrip() {
    let inodes = Arc::new(InodeTable::new());
    let search_dir = SearchDir::new(inodes.clone());

    // Query with special characters
    let query = "how does authentication work?";
    let results = vec![("/auth.py".to_string(), "content".to_string(), 0.9, 1, 10)];
    search_dir.store_results(query, search_dir.create_result_entries(&results));

    // Should appear URL-encoded in listing
    let queries = search_dir.readdir("/.search/query").unwrap();
    assert_eq!(queries.len(), 1);

    // The encoded query should contain %20 for spaces and %3F for ?
    let (_, encoded_name, _) = &queries[0];
    assert!(encoded_name.contains("%20") || encoded_name.contains("+")); // URL encoding
}

// ============== Async Bridge Tests ==============

#[test]
fn test_block_on_nested_futures() {
    init_runtime().unwrap();

    let result = block_on(async {
        let a = async { 1 }.await;
        let b = async {
            let inner = async { 2 }.await;
            inner * 2
        }
        .await;
        let c = async {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            3
        }
        .await;
        a + b + c
    })
    .unwrap();

    assert_eq!(result, 1 + 4 + 3);
}

#[test]
fn test_block_on_error_propagation() {
    init_runtime().unwrap();

    let result: Result<i32, &str> = block_on(async {
        let x = async { Ok::<i32, &str>(1) }.await?;
        let y: i32 = async { Err::<i32, _>("error") }.await?;
        Ok(x + y)
    })
    .unwrap();

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "error");
}

#[test]
fn test_block_on_with_spawned_tasks() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    init_runtime().unwrap();

    let counter = Arc::new(AtomicUsize::new(0));

    block_on(async {
        let mut handles = vec![];

        for _ in 0..10 {
            let counter = Arc::clone(&counter);
            handles.push(tokio::spawn(async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }
    })
    .unwrap();

    assert_eq!(counter.load(Ordering::SeqCst), 10);
}

// ============== Error Handling Tests ==============

#[test]
fn test_axfuse_handles_concurrent_access_same_file() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = Arc::new(OpenFsFuse::from_config(config).unwrap());

    // Initial write
    {
        let ax = Arc::clone(&ax);
        block_on(async move {
            ax.vfs
                .write("/workspace/concurrent.txt", b"initial")
                .await
                .unwrap();
        })
        .unwrap();
    }

    // Concurrent reads and writes
    let mut handles = vec![];

    for i in 0..10 {
        let ax = Arc::clone(&ax);
        handles.push(thread::spawn(move || {
            for j in 0..100 {
                if j % 2 == 0 {
                    let _ =
                        block_on(async { ax.vfs.read("/workspace/concurrent.txt").await }).unwrap();
                } else {
                    let content = format!("update {} {}", i, j);
                    let _ = block_on(async {
                        ax.vfs
                            .write("/workspace/concurrent.txt", content.as_bytes())
                            .await
                    })
                    .unwrap();
                }
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // File should still be readable
    let ax = Arc::clone(&ax);
    let content =
        block_on(async move { ax.vfs.read("/workspace/concurrent.txt").await.unwrap() }).unwrap();
    assert!(!content.is_empty());
}

#[test]
fn test_axfuse_delete_then_recreate() {
    let temp_dir = TempDir::new().unwrap();
    let config = make_config(temp_dir.path().to_str().unwrap());

    let ax = OpenFsFuse::from_config(config).unwrap();

    for round in 0..10 {
        let path = "/workspace/ephemeral.txt";
        let content = format!("round {}", round);

        block_on(async {
            ax.vfs.write(path, content.as_bytes()).await.unwrap();
        })
        .unwrap();

        let read_content = block_on(async { ax.vfs.read(path).await.unwrap() }).unwrap();
        assert_eq!(String::from_utf8(read_content).unwrap(), content);

        block_on(async {
            ax.vfs.delete(path).await.unwrap();
        })
        .unwrap();

        assert!(!block_on(async { ax.vfs.exists(path).await.unwrap() }).unwrap());
    }
}

// ============== Inode Attribute Tests ==============

#[test]
fn test_inode_attr_timestamps_advance() {
    let attr1 = InodeAttr::file(1, 100);
    std::thread::sleep(std::time::Duration::from_millis(10));
    let attr2 = InodeAttr::file(2, 100);

    // Second attr should have later timestamp
    assert!(attr2.mtime >= attr1.mtime);
}

#[test]
fn test_inode_attr_block_calculation_edge_cases() {
    // Exactly on block boundary
    let attr = InodeAttr::file(1, 512);
    assert_eq!(attr.blocks, 1);

    let attr = InodeAttr::file(1, 1024);
    assert_eq!(attr.blocks, 2);

    // Just over boundary
    let attr = InodeAttr::file(1, 513);
    assert_eq!(attr.blocks, 2);

    let attr = InodeAttr::file(1, 1025);
    assert_eq!(attr.blocks, 3);

    // Large file
    let attr = InodeAttr::file(1, 1024 * 1024 * 1024); // 1GB
    assert_eq!(attr.blocks, 2 * 1024 * 1024); // 2M blocks
}

#[test]
fn test_inode_attr_permissions() {
    let file_attr = InodeAttr::file(1, 100);
    assert_eq!(file_attr.perm, 0o644);

    let dir_attr = InodeAttr::directory(2);
    assert_eq!(dir_attr.perm, 0o755);

    let symlink_attr = InodeAttr::symlink(3, 10);
    assert_eq!(symlink_attr.perm, 0o777);
}

#[test]
fn test_inode_attr_nlink() {
    let file_attr = InodeAttr::file(1, 100);
    assert_eq!(file_attr.nlink, 1);

    let dir_attr = InodeAttr::directory(2);
    assert_eq!(dir_attr.nlink, 2); // . and ..

    let symlink_attr = InodeAttr::symlink(3, 10);
    assert_eq!(symlink_attr.nlink, 1);
}

// ============== Virtual Path Tests ==============

#[test]
fn test_search_path_detection_comprehensive() {
    // Valid search paths
    assert!(SearchDir::is_search_path("/.search"));
    assert!(SearchDir::is_search_path("/.search/"));
    assert!(SearchDir::is_search_path("/.search/query"));
    assert!(SearchDir::is_search_path("/.search/query/"));
    assert!(SearchDir::is_search_path("/.search/query/test"));
    assert!(SearchDir::is_search_path("/.search/query/test/result"));
    assert!(SearchDir::is_search_path("/.search/anything/else/here"));

    // Invalid search paths
    assert!(!SearchDir::is_search_path("/"));
    assert!(!SearchDir::is_search_path("/search"));
    assert!(!SearchDir::is_search_path("/.searchx"));
    assert!(!SearchDir::is_search_path("/x.search"));
    assert!(!SearchDir::is_search_path("/workspace/.search"));
    assert!(!SearchDir::is_search_path("/.Search"));
    assert!(!SearchDir::is_search_path("/.SEARCH"));
}

#[test]
fn test_extract_query_comprehensive() {
    // Valid queries
    assert_eq!(
        SearchDir::extract_query("/.search/query/simple"),
        Some("simple".to_string())
    );
    assert_eq!(
        SearchDir::extract_query("/.search/query/with%20spaces"),
        Some("with spaces".to_string())
    );
    assert_eq!(
        SearchDir::extract_query("/.search/query/query/with/slashes"),
        Some("query".to_string()) // Only first component
    );

    // Invalid - no query
    assert_eq!(SearchDir::extract_query("/.search"), None);
    assert_eq!(SearchDir::extract_query("/.search/"), None);
    assert_eq!(SearchDir::extract_query("/.search/query"), None);
    assert_eq!(SearchDir::extract_query("/.search/query/"), None);

    // Invalid - not search path
    assert_eq!(SearchDir::extract_query("/workspace/query/test"), None);
}

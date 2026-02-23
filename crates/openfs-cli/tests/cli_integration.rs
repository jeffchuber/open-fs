use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Helper to create a test config file.
fn create_test_config(temp_dir: &TempDir) -> String {
    let data_dir = temp_dir.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();

    let config_content = format!(
        r#"name: test-vfs
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /workspace
    backend: local
"#,
        data_dir.display()
    );

    let config_path = temp_dir.path().join("openfs.yaml");
    fs::write(&config_path, &config_content).unwrap();

    config_path.to_str().unwrap().to_string()
}

/// Get path to the openfs binary.
fn openfs_binary() -> String {
    // In tests, the binary is in target/debug
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // Remove test binary name
    path.pop(); // Remove deps
    path.push("openfs");
    path.to_str().unwrap().to_string()
}

#[test]
fn test_cli_write_and_cat() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write a file
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/test.txt",
            "hello world",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "write failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Read it back with cat
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/workspace/test.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "cat failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "hello world"
    );
}

#[test]
fn test_cli_ls() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write some files
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/file1.txt",
            "content1",
        ])
        .output()
        .expect("Failed to execute command");

    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/file2.txt",
            "content2",
        ])
        .output()
        .expect("Failed to execute command");

    // List the directory
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "ls", "/workspace"])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "ls failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("file1.txt"));
    assert!(stdout.contains("file2.txt"));
}

#[test]
fn test_cli_rm() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write a file
    let write_output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/to_delete.txt",
            "delete me",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        write_output.status.success(),
        "write failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&write_output.stdout),
        String::from_utf8_lossy(&write_output.stderr)
    );

    // Verify it exists
    let exists_output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "exists",
            "/workspace/to_delete.txt",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        exists_output.status.success(),
        "exists check failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&exists_output.stdout),
        String::from_utf8_lossy(&exists_output.stderr)
    );
    let exists_stdout = String::from_utf8_lossy(&exists_output.stdout);
    assert!(
        exists_stdout.contains("exists"),
        "Expected 'exists' in output: {}",
        exists_stdout
    );

    // Delete it
    let rm_output = Command::new(openfs_binary())
        .args(["--config", &config_path, "rm", "/workspace/to_delete.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(
        rm_output.status.success(),
        "rm failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&rm_output.stdout),
        String::from_utf8_lossy(&rm_output.stderr)
    );

    // Verify it's gone (exists command returns exit code 1 when file doesn't exist)
    let final_output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "exists",
            "/workspace/to_delete.txt",
        ])
        .output()
        .expect("Failed to execute command");

    // Exit code 1 means file doesn't exist, which is what we expect
    assert!(
        !final_output.status.success(),
        "Expected file to NOT exist after deletion"
    );
    let final_stdout = String::from_utf8_lossy(&final_output.stdout);
    assert!(
        final_stdout.contains("does not exist"),
        "Expected 'does not exist' in output: {}",
        final_stdout
    );
}

#[test]
fn test_cli_stat() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write a file
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/stat_test.txt",
            "some content",
        ])
        .output()
        .expect("Failed to execute command");

    // Get stat
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "stat", "/workspace/stat_test.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "stat failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("stat_test.txt"));
    assert!(stdout.contains("12")); // "some content" is 12 bytes
}

#[test]
fn test_cli_tree() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Create nested structure
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/a/b/c.txt",
            "nested",
        ])
        .output()
        .expect("Failed to execute command");

    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/root.txt",
            "root",
        ])
        .output()
        .expect("Failed to execute command");

    // Get tree
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "tree", "/workspace"])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "tree failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Tree output should show the structure
    assert!(stdout.contains("a") || stdout.contains("root.txt"));
}

#[test]
fn test_cli_config() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Show config
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "config"])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "config failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("test-vfs") || stdout.contains("name:"));
}

#[test]
fn test_cli_tools() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Generate tools JSON
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "tools"])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "tools failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should be valid JSON with tool definitions
    assert!(stdout.contains("read") || stdout.contains("write"));
}

#[test]
fn test_cli_no_mount_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Try to read from non-existent mount
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/nonexistent/file.txt"])
        .output()
        .expect("Failed to execute command");

    // Should fail with no mount error
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("NoMount") || stderr.contains("mount"));
}

#[test]
fn test_cli_append() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write initial content
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/append.txt",
            "hello",
        ])
        .output()
        .expect("Failed to execute command");

    // Append more content
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "append",
            "/workspace/append.txt",
            " world",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "append failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Read it back
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/workspace/append.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "hello world"
    );
}

#[test]
fn test_cli_cp() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write a file
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/source.txt",
            "copy me",
        ])
        .output()
        .expect("Failed to execute command");

    // Copy it
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "cp",
            "/workspace/source.txt",
            "/workspace/dest.txt",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "cp failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify destination exists with same content
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/workspace/dest.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "copy me");

    // Verify source still exists
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "exists", "/workspace/source.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
}

#[test]
fn test_cli_mv() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write a file
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/to_move.txt",
            "move me",
        ])
        .output()
        .expect("Failed to execute command");

    // Move it
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "mv",
            "/workspace/to_move.txt",
            "/workspace/moved.txt",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "mv failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify destination exists with same content
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/workspace/moved.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "move me");

    // Verify source no longer exists
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "exists", "/workspace/to_move.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
}

#[test]
fn test_cli_nested_directories() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write to a deeply nested path
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/a/b/c/deep.txt",
            "deep content",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success(), "write to nested path failed");

    // Read it back
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/workspace/a/b/c/deep.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "deep content"
    );

    // Verify intermediate directories exist
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "exists", "/workspace/a/b"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
}

#[test]
fn test_cli_find() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Create some files
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/test1.txt",
            "content",
        ])
        .output()
        .expect("Failed to execute command");

    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/test2.txt",
            "content",
        ])
        .output()
        .expect("Failed to execute command");

    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/other.md",
            "content",
        ])
        .output()
        .expect("Failed to execute command");

    // Find txt files using regex pattern
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "find",
            r"\.txt$",
            "-p",
            "/workspace",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "find failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("test1.txt"));
    assert!(stdout.contains("test2.txt"));
}

#[test]
fn test_cli_grep() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Create files with different content
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/has_match.txt",
            "foo bar baz",
        ])
        .output()
        .expect("Failed to execute command");

    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/no_match.txt",
            "nothing here",
        ])
        .output()
        .expect("Failed to execute command");

    // Search for pattern in specific file
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "grep",
            "bar",
            "/workspace/has_match.txt",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "grep failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should find the match
    assert!(stdout.contains("bar") || stdout.contains("foo"));
}

#[test]
fn test_cli_tools_formats() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Test JSON format
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "tools", "--format", "json"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"tools\""));

    // Test MCP format
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "tools", "--format", "mcp"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("input_schema"));

    // Test OpenAI format
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "tools", "--format", "openai"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("function"));
}

#[test]
fn test_cli_overwrite_file() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write initial content
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/overwrite.txt",
            "original",
        ])
        .output()
        .expect("Failed to execute command");

    // Overwrite with new content
    Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/overwrite.txt",
            "modified",
        ])
        .output()
        .expect("Failed to execute command");

    // Read and verify
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/workspace/overwrite.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "modified");
}

#[test]
fn test_cli_unicode_content() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    let unicode_content = "Hello \u{4e16}\u{754c} \u{1F600}";

    // Write unicode content
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/unicode.txt",
            unicode_content,
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    // Read it back
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "cat", "/workspace/unicode.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        unicode_content
    );
}

#[test]
fn test_cli_empty_file() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write empty content
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/empty.txt",
            "",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    // Verify it exists
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "exists", "/workspace/empty.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    // Stat should show size 0
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "stat", "/workspace/empty.txt"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0") || stdout.contains("empty"));
}

#[test]
fn test_cli_special_characters_in_filename() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Write file with special characters in name
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "write",
            "/workspace/file-with_special.chars.txt",
            "content",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    // Read it back
    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "cat",
            "/workspace/file-with_special.chars.txt",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "content");
}

#[test]
fn test_cli_multiple_files_in_directory() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    // Create many files
    for i in 0..10 {
        Command::new(openfs_binary())
            .args([
                "--config",
                &config_path,
                "write",
                &format!("/workspace/file_{}.txt", i),
                &format!("content_{}", i),
            ])
            .output()
            .expect("Failed to execute command");
    }

    // List directory
    let output = Command::new(openfs_binary())
        .args(["--config", &config_path, "ls", "/workspace"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // All files should be listed
    for i in 0..10 {
        assert!(stdout.contains(&format!("file_{}.txt", i)));
    }
}

#[test]
fn test_cli_cat_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir);

    let output = Command::new(openfs_binary())
        .args([
            "--config",
            &config_path,
            "cat",
            "/workspace/does_not_exist.txt",
        ])
        .output()
        .expect("Failed to execute command");

    // Should fail
    assert!(!output.status.success());
}

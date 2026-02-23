//! Shared grep implementation for regex-based file searching.

use regex::Regex;

use crate::vfs::Vfs;
use openfs_core::VfsError;

/// A single grep match.
#[derive(Debug, Clone)]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

/// Options for grep operations.
pub struct GrepOptions {
    /// Whether to recurse into subdirectories.
    pub recursive: bool,
    /// Maximum number of matches to return.
    pub max_matches: usize,
    /// Maximum directory recursion depth.
    pub max_depth: usize,
}

impl Default for GrepOptions {
    fn default() -> Self {
        GrepOptions {
            recursive: false,
            max_matches: 1000,
            max_depth: 10,
        }
    }
}

/// Search files in the VFS for lines matching a regex pattern.
///
/// If `path` points to a file, greps that file directly.
/// If `path` points to a directory, greps files in that directory
/// (recursively if `options.recursive` is true).
pub async fn grep(
    vfs: &Vfs,
    pattern: &str,
    path: &str,
    options: &GrepOptions,
) -> Result<Vec<GrepMatch>, VfsError> {
    let re = Regex::new(pattern).map_err(|e| VfsError::Config(format!("Invalid regex: {}", e)))?;
    let mut matches = Vec::new();

    // Try reading as a file first
    if let Ok(content) = vfs.read(path).await {
        grep_content(path, &content, &re, &mut matches, options.max_matches);
        return Ok(matches);
    }

    // Otherwise treat as directory
    if options.recursive {
        grep_recursive(
            vfs,
            path,
            &re,
            &mut matches,
            options.max_matches,
            options.max_depth,
        )
        .await;
    } else {
        grep_directory(vfs, path, &re, &mut matches, options.max_matches).await;
    }

    Ok(matches)
}

fn grep_content(path: &str, content: &[u8], re: &Regex, matches: &mut Vec<GrepMatch>, max: usize) {
    let text = match std::str::from_utf8(content) {
        Ok(t) => t,
        Err(_) => return, // Skip binary files
    };

    for (i, line) in text.lines().enumerate() {
        if matches.len() >= max {
            return;
        }
        if re.is_match(line) {
            matches.push(GrepMatch {
                path: path.to_string(),
                line_number: i + 1,
                line: line.to_string(),
            });
        }
    }
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

async fn grep_directory(
    vfs: &Vfs,
    path: &str,
    re: &Regex,
    matches: &mut Vec<GrepMatch>,
    max: usize,
) {
    if let Ok(entries) = vfs.list(path).await {
        for entry in entries {
            if matches.len() >= max {
                return;
            }
            if !entry.is_dir {
                let full_path = join_path(path, &entry.name);
                if let Ok(content) = vfs.read(&full_path).await {
                    grep_content(&full_path, &content, re, matches, max);
                }
            }
        }
    }
}

async fn grep_recursive(
    vfs: &Vfs,
    path: &str,
    re: &Regex,
    matches: &mut Vec<GrepMatch>,
    max: usize,
    depth: usize,
) {
    if depth == 0 || matches.len() >= max {
        return;
    }
    if let Ok(entries) = vfs.list(path).await {
        for entry in entries {
            if matches.len() >= max {
                return;
            }
            let full_path = join_path(path, &entry.name);
            if entry.is_dir {
                Box::pin(grep_recursive(vfs, &full_path, re, matches, max, depth - 1)).await;
            } else if let Ok(content) = vfs.read(&full_path).await {
                grep_content(&full_path, &content, re, matches, max);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfs_config::VfsConfig;
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

    #[tokio::test]
    async fn test_grep_single_file() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(tmp.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/test.txt", b"hello world\nfoo bar\nhello again")
            .await
            .unwrap();

        let matches = grep(
            &vfs,
            "hello",
            "/workspace/test.txt",
            &GrepOptions::default(),
        )
        .await
        .unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 1);
        assert_eq!(matches[1].line_number, 3);
    }

    #[tokio::test]
    async fn test_grep_directory() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(tmp.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/a.txt", b"hello world").await.unwrap();
        vfs.write("/workspace/b.txt", b"goodbye world")
            .await
            .unwrap();

        let matches = grep(&vfs, "hello", "/workspace", &GrepOptions::default())
            .await
            .unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "/workspace/a.txt");
    }

    #[tokio::test]
    async fn test_grep_recursive() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(tmp.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        vfs.write("/workspace/a.txt", b"hello top").await.unwrap();
        vfs.write("/workspace/sub/b.txt", b"hello nested")
            .await
            .unwrap();

        let opts = GrepOptions {
            recursive: true,
            ..Default::default()
        };
        let matches = grep(&vfs, "hello", "/workspace", &opts).await.unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[tokio::test]
    async fn test_grep_max_matches() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(tmp.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        let content = (0..100)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        vfs.write("/workspace/big.txt", content.as_bytes())
            .await
            .unwrap();

        let opts = GrepOptions {
            max_matches: 5,
            ..Default::default()
        };
        let matches = grep(&vfs, "line", "/workspace/big.txt", &opts)
            .await
            .unwrap();
        assert_eq!(matches.len(), 5);
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(tmp.path().to_str().unwrap());
        let vfs = Vfs::from_config(config).await.unwrap();

        let result = grep(&vfs, "[invalid", "/workspace", &GrepOptions::default()).await;
        assert!(result.is_err());
    }
}

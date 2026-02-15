use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Persistent state tracking indexed files.
///
/// Stores a mapping of file paths to their last-known size, modification time,
/// chunk count, and indexing timestamp. Used by `IncrementalIndexer` to
/// determine which files need re-indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexState {
    /// State format version.
    pub version: String,
    /// When the state was last updated.
    pub last_updated: DateTime<Utc>,
    /// Per-file tracking data.
    pub files: HashMap<String, FileState>,
}

/// Tracked state for a single indexed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileState {
    /// File size in bytes at time of indexing.
    pub size: u64,
    /// File modification time at time of indexing.
    pub mtime: Option<DateTime<Utc>>,
    /// Number of chunks created during indexing.
    pub chunks: usize,
    /// When this file was last indexed.
    pub indexed_at: DateTime<Utc>,
    /// BLAKE3 content hash (hex) -- used for deduplication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

/// The result of computing a delta between current files and the index state.
#[derive(Debug, Default)]
pub struct DeltaResult {
    /// Files that are new (not in the index state).
    pub new_files: Vec<String>,
    /// Files that have been modified (size or mtime changed).
    pub modified_files: Vec<String>,
    /// Files that have been deleted (in state but not on disk).
    pub deleted_files: Vec<String>,
    /// Files that are unchanged.
    pub unchanged_files: Vec<String>,
}

/// Information about a file on disk, used for delta computation.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
    pub mtime: Option<DateTime<Utc>>,
    /// Optional content hash for dedup-aware delta computation.
    pub content_hash: Option<String>,
}

/// Action to take during cold boot reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileAction {
    /// File is new and needs to be indexed.
    Index { path: String },
    /// File was modified and needs re-indexing.
    Reindex { path: String },
    /// File metadata changed but content hash matches -- skip re-indexing.
    SkipUnchangedContent { path: String },
    /// File was deleted while offline and should be removed from the index.
    RemoveOrphan { path: String },
}

/// Result of a cold boot reconciliation.
#[derive(Debug, Default)]
pub struct ReconcileResult {
    pub actions: Vec<ReconcileAction>,
    pub index_count: usize,
    pub reindex_count: usize,
    pub skip_count: usize,
    pub orphan_count: usize,
}

impl IndexState {
    /// Create a new empty index state.
    pub fn new() -> Self {
        IndexState {
            version: "1".to_string(),
            last_updated: Utc::now(),
            files: HashMap::new(),
        }
    }

    /// Load index state from a JSON file.
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })
    }

    /// Save index state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            std::io::Error::other(e.to_string())
        })?;
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)
    }

    /// Compute the delta between the current files and the saved state.
    ///
    /// If `FileInfo.content_hash` is provided and the stored state also has a hash,
    /// files with matching hashes are treated as unchanged even if size/mtime differ.
    pub fn compute_delta(&self, current_files: &[FileInfo]) -> DeltaResult {
        let mut result = DeltaResult::default();

        let current_set: HashMap<&str, &FileInfo> = current_files
            .iter()
            .map(|f| (f.path.as_str(), f))
            .collect();

        // Check for new and modified files
        for file_info in current_files {
            match self.files.get(&file_info.path) {
                None => {
                    result.new_files.push(file_info.path.clone());
                }
                Some(state) => {
                    // If both have content hashes, use hash comparison (dedup-aware)
                    if let (Some(current_hash), Some(stored_hash)) =
                        (&file_info.content_hash, &state.content_hash)
                    {
                        if current_hash == stored_hash {
                            result.unchanged_files.push(file_info.path.clone());
                        } else {
                            result.modified_files.push(file_info.path.clone());
                        }
                    } else if state.size != file_info.size || state.mtime != file_info.mtime {
                        result.modified_files.push(file_info.path.clone());
                    } else {
                        result.unchanged_files.push(file_info.path.clone());
                    }
                }
            }
        }

        // Check for deleted files
        for path in self.files.keys() {
            if !current_set.contains_key(path.as_str()) {
                result.deleted_files.push(path.clone());
            }
        }

        result
    }

    /// Perform cold boot reconciliation.
    ///
    /// Compares the saved index state against a list of files currently on disk
    /// (with optional content hashes) and returns a list of actions needed to
    /// bring the index up to date:
    /// - New files need indexing
    /// - Modified files need re-indexing (unless content hash matches)
    /// - Orphaned entries (files deleted while offline) need removal
    pub fn reconcile(&self, current_files: &[FileInfo]) -> ReconcileResult {
        let mut result = ReconcileResult::default();

        let current_set: HashMap<&str, &FileInfo> = current_files
            .iter()
            .map(|f| (f.path.as_str(), f))
            .collect();

        // Check current files against state
        for file_info in current_files {
            match self.files.get(&file_info.path) {
                None => {
                    // New file -- needs indexing
                    result.actions.push(ReconcileAction::Index {
                        path: file_info.path.clone(),
                    });
                    result.index_count += 1;
                }
                Some(state) => {
                    // Check if content hash matches (skip re-indexing)
                    if let (Some(current_hash), Some(stored_hash)) =
                        (&file_info.content_hash, &state.content_hash)
                    {
                        if current_hash == stored_hash {
                            result.actions.push(ReconcileAction::SkipUnchangedContent {
                                path: file_info.path.clone(),
                            });
                            result.skip_count += 1;
                            continue;
                        }
                    }

                    // Check metadata
                    if state.size != file_info.size || state.mtime != file_info.mtime {
                        result.actions.push(ReconcileAction::Reindex {
                            path: file_info.path.clone(),
                        });
                        result.reindex_count += 1;
                    } else {
                        result.skip_count += 1;
                    }
                }
            }
        }

        // Check for orphaned entries (in state but not on disk)
        for path in self.files.keys() {
            if !current_set.contains_key(path.as_str()) {
                result.actions.push(ReconcileAction::RemoveOrphan {
                    path: path.clone(),
                });
                result.orphan_count += 1;
            }
        }

        result
    }

    /// Record that a file has been indexed.
    pub fn record_indexed(&mut self, path: &str, size: u64, mtime: Option<DateTime<Utc>>, chunks: usize) {
        let now = Utc::now();
        self.files.insert(
            path.to_string(),
            FileState {
                size,
                mtime,
                chunks,
                indexed_at: now,
                content_hash: None,
            },
        );
        self.last_updated = now;
    }

    /// Record that a file has been indexed, with a content hash for deduplication.
    pub fn record_indexed_with_hash(
        &mut self,
        path: &str,
        size: u64,
        mtime: Option<DateTime<Utc>>,
        chunks: usize,
        content_hash: String,
    ) {
        let now = Utc::now();
        self.files.insert(
            path.to_string(),
            FileState {
                size,
                mtime,
                chunks,
                indexed_at: now,
                content_hash: Some(content_hash),
            },
        );
        self.last_updated = now;
    }

    /// Check if a file's content hash matches the stored hash.
    /// Returns true if hashes match (content unchanged), false otherwise.
    pub fn content_unchanged(&self, path: &str, hash: &str) -> bool {
        self.files
            .get(path)
            .and_then(|state| state.content_hash.as_deref())
            .map(|stored| stored == hash)
            .unwrap_or(false)
    }

    /// Remove a file from the state (e.g., when deleted).
    pub fn remove_file(&mut self, path: &str) {
        self.files.remove(path);
        self.last_updated = Utc::now();
    }

    /// Get the default state file path relative to a directory.
    pub fn default_path(base_dir: &Path) -> PathBuf {
        base_dir.join(".ax-index-state.json")
    }

    /// Get the total number of tracked files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Get the total number of chunks across all files.
    pub fn total_chunks(&self) -> usize {
        self.files.values().map(|f| f.chunks).sum()
    }
}

impl Default for IndexState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_new_state() {
        let state = IndexState::new();
        assert_eq!(state.version, "1");
        assert!(state.files.is_empty());
    }

    #[test]
    fn test_record_and_query() {
        let mut state = IndexState::new();
        let mtime = Utc::now();

        state.record_indexed("/test.txt", 100, Some(mtime), 3);

        assert_eq!(state.file_count(), 1);
        assert_eq!(state.total_chunks(), 3);

        let file_state = state.files.get("/test.txt").unwrap();
        assert_eq!(file_state.size, 100);
        assert_eq!(file_state.mtime, Some(mtime));
        assert_eq!(file_state.chunks, 3);
    }

    #[test]
    fn test_remove_file() {
        let mut state = IndexState::new();
        state.record_indexed("/test.txt", 100, None, 3);
        assert_eq!(state.file_count(), 1);

        state.remove_file("/test.txt");
        assert_eq!(state.file_count(), 0);
    }

    #[test]
    fn test_save_and_load() {
        let tmp = TempDir::new().unwrap();
        let state_path = tmp.path().join("state.json");

        let mut state = IndexState::new();
        let mtime = Utc::now();
        state.record_indexed("/file1.txt", 100, Some(mtime), 2);
        state.record_indexed("/file2.txt", 200, None, 5);

        state.save(&state_path).unwrap();

        let loaded = IndexState::load(&state_path).unwrap();
        assert_eq!(loaded.version, "1");
        assert_eq!(loaded.file_count(), 2);
        assert_eq!(loaded.total_chunks(), 7);

        let f1 = loaded.files.get("/file1.txt").unwrap();
        assert_eq!(f1.size, 100);
        assert_eq!(f1.chunks, 2);
    }

    #[test]
    fn test_compute_delta_new_files() {
        let state = IndexState::new();

        let current = vec![
            FileInfo { path: "/new1.txt".to_string(), size: 100, mtime: None, content_hash: None },
            FileInfo { path: "/new2.txt".to_string(), size: 200, mtime: None, content_hash: None },
        ];

        let delta = state.compute_delta(&current);
        assert_eq!(delta.new_files.len(), 2);
        assert!(delta.modified_files.is_empty());
        assert!(delta.deleted_files.is_empty());
        assert!(delta.unchanged_files.is_empty());
    }

    #[test]
    fn test_default_path() {
        let path = IndexState::default_path(Path::new("/workspace"));
        assert_eq!(path, PathBuf::from("/workspace/.ax-index-state.json"));
    }
}

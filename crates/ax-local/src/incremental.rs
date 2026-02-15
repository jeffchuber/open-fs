use std::path::Path;
use std::time::Instant;

use ax_core::{Backend, VfsError};
use tracing::{debug, info, warn};

use crate::index_state::{FileInfo, IndexState};
use crate::pipeline::{IndexingPipeline, PipelineConfig};

/// Result of an incremental indexing operation.
#[derive(Debug)]
pub struct IncrementalResult {
    /// Number of new files indexed.
    pub new_files: usize,
    /// Number of modified files re-indexed.
    pub modified_files: usize,
    /// Number of deleted files cleaned up.
    pub deleted_files: usize,
    /// Number of unchanged files skipped.
    pub unchanged_files: usize,
    /// Total chunks created.
    pub total_chunks: usize,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Errors encountered during indexing.
    pub errors: Vec<(String, String)>,
}

/// Incremental indexer that wraps an `IndexingPipeline` with state tracking.
///
/// It compares the current state of files against the persisted `IndexState`
/// and only re-indexes files that are new or modified.
pub struct IncrementalIndexer {
    pipeline: IndexingPipeline,
    state: IndexState,
    state_path: std::path::PathBuf,
}

impl IncrementalIndexer {
    /// Create a new incremental indexer.
    pub fn new(
        config: PipelineConfig,
        state_path: &Path,
    ) -> Result<Self, VfsError> {
        let pipeline = IndexingPipeline::new(config)?;

        let state = if state_path.exists() {
            IndexState::load(state_path).unwrap_or_else(|e| {
                warn!("Failed to load index state, starting fresh: {}", e);
                IndexState::new()
            })
        } else {
            IndexState::new()
        };

        Ok(IncrementalIndexer {
            pipeline,
            state,
            state_path: state_path.to_path_buf(),
        })
    }

    /// Get a reference to the underlying pipeline.
    pub fn pipeline(&self) -> &IndexingPipeline {
        &self.pipeline
    }

    /// Get a reference to the current index state.
    pub fn state(&self) -> &IndexState {
        &self.state
    }

    /// Index a directory incrementally -- only new and modified files are processed.
    pub async fn index_directory<B: Backend>(
        &mut self,
        backend: &B,
        dir_path: &str,
        recursive: bool,
    ) -> Result<IncrementalResult, VfsError> {
        let start = Instant::now();
        let mut total_chunks = 0;
        let mut errors = Vec::new();

        // Collect current file states
        let mut current_files = Vec::new();
        self.collect_file_info(backend, dir_path, recursive, &mut current_files)
            .await?;

        info!(
            "Found {} files in {}, computing delta",
            current_files.len(),
            dir_path
        );

        // Compute delta
        let delta = self.state.compute_delta(&current_files);

        debug!(
            "Delta: {} new, {} modified, {} deleted, {} unchanged",
            delta.new_files.len(),
            delta.modified_files.len(),
            delta.deleted_files.len(),
            delta.unchanged_files.len()
        );

        // Index new and modified files
        let files_to_index: Vec<String> = delta
            .new_files
            .iter()
            .chain(delta.modified_files.iter())
            .cloned()
            .collect();

        let file_info_map: std::collections::HashMap<&str, &FileInfo> = current_files
            .iter()
            .map(|f| (f.path.as_str(), f))
            .collect();

        for path in &files_to_index {
            match backend.read(path).await {
                Ok(content) => match self.pipeline.index_file(path, &content).await {
                    Ok(result) => {
                        total_chunks += result.chunks_created;
                        if let Some(info) = file_info_map.get(path.as_str()) {
                            self.state.record_indexed(
                                path,
                                info.size,
                                info.mtime,
                                result.chunks_created,
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Failed to index {}: {}", path, e);
                        errors.push((path.clone(), e.to_string()));
                    }
                },
                Err(e) => {
                    warn!("Failed to read {}: {}", path, e);
                    errors.push((path.clone(), e.to_string()));
                }
            }
        }

        // Clean up deleted files
        for path in &delta.deleted_files {
            if let Err(e) = self.pipeline.delete_file(path).await {
                warn!("Failed to clean up index for deleted file {}: {}", path, e);
            }
            self.state.remove_file(path);
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(IncrementalResult {
            new_files: delta.new_files.len(),
            modified_files: delta.modified_files.len(),
            deleted_files: delta.deleted_files.len(),
            unchanged_files: delta.unchanged_files.len(),
            total_chunks,
            duration_ms,
            errors,
        })
    }

    /// Handle a single file change event.
    pub async fn handle_change<B: Backend>(
        &mut self,
        backend: &B,
        path: &str,
        deleted: bool,
    ) -> Result<(), VfsError> {
        if deleted {
            self.pipeline.delete_file(path).await?;
            self.state.remove_file(path);
        } else {
            let content = backend.read(path).await.map_err(VfsError::from)?;
            let stat = backend.stat(path).await.map_err(VfsError::from)?;
            let result = self.pipeline.index_file(path, &content).await?;
            self.state.record_indexed(
                path,
                stat.size.unwrap_or(0),
                stat.modified,
                result.chunks_created,
            );
        }
        Ok(())
    }

    /// Persist the index state to disk.
    pub fn persist_state(&self) -> Result<(), VfsError> {
        self.state.save(&self.state_path).map_err(VfsError::Io)
    }

    /// Recursively collect file info (path, size, mtime) from a directory.
    async fn collect_file_info<B: Backend>(
        &self,
        backend: &B,
        dir_path: &str,
        recursive: bool,
        files: &mut Vec<FileInfo>,
    ) -> Result<(), VfsError> {
        let entries = backend.list(dir_path).await.map_err(VfsError::from)?;

        for entry in entries {
            if entry.is_dir {
                if recursive {
                    Box::pin(self.collect_file_info(backend, &entry.path, recursive, files))
                        .await?;
                }
            } else {
                files.push(FileInfo {
                    path: entry.path,
                    size: entry.size.unwrap_or(0),
                    mtime: entry.modified,
                    content_hash: None,
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ax_remote::MemoryBackend;
    use tempfile::TempDir;

    fn make_indexer(tmp: &TempDir) -> IncrementalIndexer {
        let state_path = tmp.path().join(".ax-index-state.json");
        IncrementalIndexer::new(PipelineConfig::default(), &state_path).unwrap()
    }

    #[tokio::test]
    async fn test_incremental_index_new_files() {
        let tmp = TempDir::new().unwrap();
        let mut indexer = make_indexer(&tmp);

        let backend = MemoryBackend::new();
        backend
            .write("/dir/file1.txt", b"Hello world")
            .await
            .unwrap();
        backend
            .write("/dir/file2.txt", b"Goodbye world")
            .await
            .unwrap();

        let result = indexer.index_directory(&backend, "/dir", true).await.unwrap();

        assert_eq!(result.new_files, 2);
        assert_eq!(result.modified_files, 0);
        assert_eq!(result.deleted_files, 0);
        assert_eq!(result.unchanged_files, 0);
        assert!(result.total_chunks >= 2);
    }

    #[tokio::test]
    async fn test_incremental_index_unchanged() {
        let tmp = TempDir::new().unwrap();
        let mut indexer = make_indexer(&tmp);

        let backend = MemoryBackend::new();
        backend
            .write("/dir/file1.txt", b"Hello world")
            .await
            .unwrap();

        // First index
        let result1 = indexer.index_directory(&backend, "/dir", true).await.unwrap();
        assert_eq!(result1.new_files, 1);

        // Second index -- nothing changed
        let result2 = indexer.index_directory(&backend, "/dir", true).await.unwrap();
        assert_eq!(result2.new_files, 0);
        assert_eq!(result2.modified_files, 0);
        assert_eq!(result2.unchanged_files, 1);
    }

    #[tokio::test]
    async fn test_handle_change_create() {
        let tmp = TempDir::new().unwrap();
        let mut indexer = make_indexer(&tmp);

        let backend = MemoryBackend::new();
        backend
            .write("/file.txt", b"New file content")
            .await
            .unwrap();

        indexer
            .handle_change(&backend, "/file.txt", false)
            .await
            .unwrap();

        assert_eq!(indexer.state().file_count(), 1);
    }

    #[tokio::test]
    async fn test_handle_change_delete() {
        let tmp = TempDir::new().unwrap();
        let mut indexer = make_indexer(&tmp);

        let backend = MemoryBackend::new();
        backend
            .write("/file.txt", b"Content")
            .await
            .unwrap();

        // First, index the file
        indexer
            .handle_change(&backend, "/file.txt", false)
            .await
            .unwrap();
        assert_eq!(indexer.state().file_count(), 1);

        // Then handle delete
        indexer
            .handle_change(&backend, "/file.txt", true)
            .await
            .unwrap();
        assert_eq!(indexer.state().file_count(), 0);
    }
}

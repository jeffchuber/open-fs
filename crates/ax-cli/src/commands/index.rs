use std::sync::Arc;

use ax_config::BackendConfig;
use ax_core::ChromaStore;
use ax_local::{ChunkerConfig, IndexState, IndexingPipeline, PipelineConfig, FileInfo, BulkIndexResult};
use ax_remote::{ChromaHttpBackend, Vfs};

pub async fn run(
    vfs: &Vfs,
    path: Option<String>,
    chroma_endpoint: Option<String>,
    collection: Option<String>,
    recursive: bool,
    chunker: Option<String>,
    chunk_size: Option<usize>,
    incremental: bool,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.unwrap_or_else(|| "/".to_string());

    // Guard: only index from local (fs/memory) backends
    let config = vfs.effective_config();
    for mount in &config.mounts {
        if path.starts_with(&mount.path) || mount.path.starts_with(&path) {
            if let Some(backend_name) = &mount.backend {
                if let Some(backend_config) = config.backends.get(backend_name) {
                    match backend_config {
                        BackendConfig::Fs(_) | BackendConfig::Memory(_) => {}
                        _ => {
                            eprintln!(
                                "Warning: skipping mount '{}' — indexing is only supported for local (fs) backends, not '{}'",
                                mount.path, backend_name
                            );
                            eprintln!("Remote backends (for example S3) should use their own indexing systems.");
                            continue;
                        }
                    }
                }
            }
        }
    }

    // Set up pipeline config
    let mut config = PipelineConfig::default();

    if let Some(strategy) = chunker {
        config.chunker_strategy = strategy;
    }

    if let Some(size) = chunk_size {
        config.chunker = ChunkerConfig {
            chunk_size: size,
            chunk_overlap: size / 8,
            min_chunk_size: size / 10,
        };
    }

    let pipeline = IndexingPipeline::new(config)?;

    // Set up Chroma backend if specified
    let pipeline = if let Some(endpoint) = chroma_endpoint {
        let collection_name = collection.unwrap_or_else(|| "ax_index".to_string());
        println!("Connecting to Chroma at {} (collection: {})", endpoint, collection_name);
        let chroma = ChromaHttpBackend::new(&endpoint, &collection_name).await
            .map_err(|e| format!("Failed to connect to Chroma: {}", e))?;
        pipeline.with_chroma(Arc::new(chroma) as Arc<dyn ChromaStore>)
    } else {
        println!("No Chroma endpoint specified, indexing to memory only");
        pipeline
    };

    // Check if path is a file or directory
    let entry = vfs.stat(&path).await?;

    if entry.is_dir {
        if incremental && !force {
            println!("Incremental indexing directory: {} (recursive: {})", path, recursive);
            let result = index_directory_incremental(vfs, &pipeline, &path, recursive).await?;

            println!("\nIncremental indexing complete:");
            println!("  New files: {}", result.new_files);
            println!("  Modified files: {}", result.modified_files);
            println!("  Deleted files: {}", result.deleted_files);
            println!("  Unchanged files (skipped): {}", result.unchanged_files);
            println!("  Total chunks: {}", result.total_chunks);
            println!("  Duration: {}ms", result.duration_ms);

            if !result.errors.is_empty() {
                println!("\nErrors:");
                for (path, error) in result.errors {
                    println!("  {}: {}", path, error);
                }
            }
        } else {
            if force {
                println!("Force re-indexing directory: {} (recursive: {})", path, recursive);
                // Delete existing state file
                let state_path = IndexState::default_path(std::path::Path::new("."));
                if state_path.exists() {
                    std::fs::remove_file(&state_path)?;
                    println!("Removed existing index state");
                }
            } else {
                println!("Indexing directory: {} (recursive: {})", path, recursive);
            }

            // Index using VFS as the backend wrapper
            let result = index_directory_via_vfs(vfs, &pipeline, &path, recursive).await?;

            println!("\nIndexing complete:");
            println!("  Files processed: {}", result.files_processed);
            println!("  Files skipped: {}", result.files_skipped);
            println!("  Total chunks: {}", result.total_chunks);
            println!("  Duration: {}ms", result.duration_ms);

            if !result.errors.is_empty() {
                println!("\nErrors:");
                for (path, error) in result.errors {
                    println!("  {}: {}", path, error);
                }
            }
        }
    } else {
        println!("Indexing file: {}", path);

        let content = vfs.read(&path).await?;
        let result = pipeline.index_file(&path, &content).await?;

        println!("\nIndexing complete:");
        println!("  Chunks created: {}", result.chunks_created);
        println!("  Duration: {}ms", result.duration_ms);
    }

    Ok(())
}

/// Result of an incremental indexing run via the CLI.
struct IncrementalRunResult {
    new_files: usize,
    modified_files: usize,
    deleted_files: usize,
    unchanged_files: usize,
    total_chunks: usize,
    duration_ms: u64,
    errors: Vec<(String, String)>,
}

/// Index a directory incrementally using VFS and IndexState.
async fn index_directory_incremental(
    vfs: &Vfs,
    pipeline: &IndexingPipeline,
    dir_path: &str,
    recursive: bool,
) -> Result<IncrementalRunResult, Box<dyn std::error::Error>> {
    use std::time::Instant;

    let start = Instant::now();
    let mut total_chunks = 0;
    let mut errors = Vec::new();

    // Load or create index state
    let state_path = IndexState::default_path(std::path::Path::new("."));
    let mut state = if state_path.exists() {
        IndexState::load(&state_path).unwrap_or_else(|e| {
            eprintln!("Warning: Failed to load index state, starting fresh: {}", e);
            IndexState::new()
        })
    } else {
        IndexState::new()
    };

    // Collect current file info via VFS
    let mut current_files = Vec::new();
    collect_file_info_via_vfs(vfs, dir_path, recursive, &mut current_files).await?;

    println!("Found {} files, computing delta...", current_files.len());

    // Compute delta
    let delta = state.compute_delta(&current_files);

    println!(
        "Delta: {} new, {} modified, {} deleted, {} unchanged",
        delta.new_files.len(),
        delta.modified_files.len(),
        delta.deleted_files.len(),
        delta.unchanged_files.len()
    );

    // Build a map of file info for recording
    let file_info_map: std::collections::HashMap<&str, &FileInfo> = current_files
        .iter()
        .map(|f| (f.path.as_str(), f))
        .collect();

    // Index new and modified files
    let files_to_index: Vec<String> = delta
        .new_files
        .iter()
        .chain(delta.modified_files.iter())
        .cloned()
        .collect();

    let total_to_index = files_to_index.len();

    for (i, path) in files_to_index.iter().enumerate() {
        if (i + 1) % 10 == 0 || i + 1 == total_to_index {
            print!("\rProcessing {}/{}", i + 1, total_to_index);
        }

        match vfs.read(path).await {
            Ok(content) => match pipeline.index_file(path, &content).await {
                Ok(result) => {
                    total_chunks += result.chunks_created;
                    if let Some(info) = file_info_map.get(path.as_str()) {
                        state.record_indexed(
                            path,
                            info.size,
                            info.mtime,
                            result.chunks_created,
                        );
                    }
                }
                Err(e) => {
                    errors.push((path.clone(), e.to_string()));
                }
            },
            Err(e) => {
                errors.push((path.clone(), e.to_string()));
            }
        }
    }

    if total_to_index > 0 {
        println!();
    }

    // Clean up deleted files from state
    for path in &delta.deleted_files {
        if let Err(e) = pipeline.delete_file(path).await {
            eprintln!("Warning: Failed to clean up index for {}: {}", path, e);
        }
        state.remove_file(path);
    }

    // Persist state
    state.save(&state_path).map_err(|e| format!("Failed to save index state: {}", e))?;
    println!("Index state saved to {}", state_path.display());

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(IncrementalRunResult {
        new_files: delta.new_files.len(),
        modified_files: delta.modified_files.len(),
        deleted_files: delta.deleted_files.len(),
        unchanged_files: delta.unchanged_files.len(),
        total_chunks,
        duration_ms,
        errors,
    })
}

/// Recursively collect file info (path, size, mtime) via VFS.
#[async_recursion::async_recursion]
async fn collect_file_info_via_vfs(
    vfs: &Vfs,
    dir_path: &str,
    recursive: bool,
    files: &mut Vec<FileInfo>,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = vfs.list(dir_path).await?;

    for entry in entries {
        if entry.is_dir {
            if recursive {
                collect_file_info_via_vfs(vfs, &entry.path, recursive, files).await?;
            }
        } else if is_indexable(&entry.path) {
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

/// Index a directory using VFS for file access.
async fn index_directory_via_vfs(
    vfs: &Vfs,
    pipeline: &IndexingPipeline,
    dir_path: &str,
    recursive: bool,
) -> Result<BulkIndexResult, Box<dyn std::error::Error>> {
    use std::time::Instant;

    let start = Instant::now();
    let mut files_processed = 0;
    let mut files_skipped = 0;
    let mut total_chunks = 0;
    let mut errors = Vec::new();

    // Collect files to index
    let mut paths_to_index = Vec::new();
    collect_files_via_vfs(vfs, dir_path, recursive, &mut paths_to_index).await?;

    println!("Found {} files to index", paths_to_index.len());

    for (i, path) in paths_to_index.iter().enumerate() {
        // Simple progress indicator
        if (i + 1) % 10 == 0 || i + 1 == paths_to_index.len() {
            print!("\rProcessing {}/{}", i + 1, paths_to_index.len());
        }

        match vfs.read(path).await {
            Ok(content) => {
                match pipeline.index_file(path, &content).await {
                    Ok(result) => {
                        files_processed += 1;
                        total_chunks += result.chunks_created;
                    }
                    Err(e) => {
                        errors.push((path.clone(), e.to_string()));
                        files_skipped += 1;
                    }
                }
            }
            Err(e) => {
                errors.push((path.clone(), e.to_string()));
                files_skipped += 1;
            }
        }
    }
    println!(); // newline after progress

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(BulkIndexResult {
        files_processed,
        files_skipped,
        total_chunks,
        duration_ms,
        errors,
    })
}

/// Recursively collect file paths from a directory via VFS.
#[async_recursion::async_recursion]
async fn collect_files_via_vfs(
    vfs: &Vfs,
    dir_path: &str,
    recursive: bool,
    paths: &mut Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = vfs.list(dir_path).await?;

    for entry in entries {
        if entry.is_dir {
            if recursive {
                collect_files_via_vfs(vfs, &entry.path, recursive, paths).await?;
            }
        } else {
            // Only index text files (simple extension check)
            if is_indexable(&entry.path) {
                paths.push(entry.path);
            }
        }
    }

    Ok(())
}

/// Check if a file should be indexed (simple extension-based check).
fn is_indexable(path: &str) -> bool {
    let extensions = [
        "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp",
        "cs", "rb", "php", "swift", "kt", "scala", "clj", "ex", "exs", "erl", "hs",
        "lua", "r", "jl", "pl", "pm", "sh", "bash", "zsh", "fish", "ps1", "bat",
        "html", "htm", "css", "scss", "sass", "less", "vue", "svelte",
        "json", "yaml", "yml", "toml", "ini", "cfg", "conf",
        "md", "markdown", "txt", "rst", "adoc", "org",
        "csv", "tsv", "xml", "sql",
    ];

    if let Some(ext) = path.rsplit('.').next() {
        extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    } else {
        false
    }
}

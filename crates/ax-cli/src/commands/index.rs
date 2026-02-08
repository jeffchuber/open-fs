use ax_backends::ChromaBackend;
use ax_core::{IndexingPipeline, PipelineConfig, Vfs};
use ax_indexing::ChunkerConfig;

pub async fn run(
    vfs: &Vfs,
    path: Option<String>,
    chroma_endpoint: Option<String>,
    collection: Option<String>,
    recursive: bool,
    chunker: Option<String>,
    chunk_size: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.unwrap_or_else(|| "/".to_string());

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
        let chroma = ChromaBackend::new(&endpoint, &collection_name).await
            .map_err(|e| format!("Failed to connect to Chroma: {}", e))?;
        pipeline.with_chroma(chroma)
    } else {
        println!("No Chroma endpoint specified, indexing to memory only");
        pipeline
    };

    // Check if path is a file or directory
    let entry = vfs.stat(&path).await?;

    if entry.is_dir {
        println!("Indexing directory: {} (recursive: {})", path, recursive);

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

/// Index a directory using VFS for file access.
async fn index_directory_via_vfs(
    vfs: &Vfs,
    pipeline: &IndexingPipeline,
    dir_path: &str,
    recursive: bool,
) -> Result<ax_indexing::BulkIndexResult, Box<dyn std::error::Error>> {
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

    Ok(ax_indexing::BulkIndexResult {
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

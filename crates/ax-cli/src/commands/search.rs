use std::sync::Arc;

use ax_core::ChromaStore;
use ax_local::{IndexingPipeline, PipelineConfig, SearchConfig, SearchEngine, SearchMode};
use ax_remote::{ChromaHttpBackend, Vfs};

pub async fn run(
    _vfs: &Vfs,
    query: &str,
    chroma_endpoint: Option<String>,
    collection: Option<String>,
    limit: Option<usize>,
    mode: Option<String>,
    context_lines: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Search requires a Chroma backend for dense search
    let chroma_endpoint = chroma_endpoint.ok_or(
        "Search requires --chroma-endpoint to be specified"
    )?;

    let collection_name = collection.unwrap_or_else(|| "ax_index".to_string());

    // Connect to Chroma
    let chroma = ChromaHttpBackend::new(&chroma_endpoint, &collection_name).await
        .map_err(|e| format!("Failed to connect to Chroma: {}", e))?;

    // Create pipeline and search engine
    let config = PipelineConfig::default();
    let pipeline = Arc::new(IndexingPipeline::new(config)?);
    let engine = SearchEngine::new(pipeline).with_chroma(Arc::new(chroma) as Arc<dyn ChromaStore>);

    // Parse search mode
    let search_mode = match mode.as_deref() {
        Some("dense") => SearchMode::Dense,
        Some("sparse") => SearchMode::Sparse,
        Some("hybrid") => SearchMode::Hybrid,
        None => SearchMode::Dense, // Default to dense for Chroma-based search
        Some(m) => return Err(format!("Unknown search mode: {}. Use 'dense', 'sparse', or 'hybrid'", m).into()),
    };

    // Configure search
    let search_config = SearchConfig {
        mode: search_mode,
        limit: limit.unwrap_or(10),
        min_score: 0.0,
        ..Default::default()
    };

    println!("Searching for: \"{}\"", query);
    println!("Mode: {:?}, Limit: {}\n", search_config.mode, search_config.limit);

    // Perform search
    let results = engine.search(query, &search_config).await?;

    if results.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    println!("Found {} results:\n", results.len());

    let context = context_lines.unwrap_or(2);

    for (i, result) in results.iter().enumerate() {
        println!("{}. {} (score: {:.4})", i + 1, result.chunk.source_path, result.score);

        if let (Some(dense), Some(sparse)) = (result.dense_score, result.sparse_score) {
            println!("   [dense: {:.4}, sparse: {:.4}]", dense, sparse);
        }

        println!("   Lines {}-{}, chunk {}/{}",
            result.chunk.start_line,
            result.chunk.end_line,
            result.chunk.chunk_index + 1,
            result.chunk.total_chunks
        );

        // Show snippet
        let content = &result.chunk.content;
        let lines: Vec<&str> = content.lines().collect();
        let preview_lines = if lines.len() > context * 2 + 1 {
            let start = &lines[..context];
            let end = &lines[lines.len() - context..];
            format!("{}\n   ...\n{}",
                start.join("\n").lines().map(|l| format!("   {}", l)).collect::<Vec<_>>().join("\n"),
                end.join("\n").lines().map(|l| format!("   {}", l)).collect::<Vec<_>>().join("\n")
            )
        } else {
            lines.iter().map(|l| format!("   {}", l)).collect::<Vec<_>>().join("\n")
        };

        println!("{}\n", preview_lines);
    }

    Ok(())
}

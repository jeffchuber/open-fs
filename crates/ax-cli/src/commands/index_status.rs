use std::path::PathBuf;

use ax_local::IndexState;

pub async fn run(state_file: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let state_path = if let Some(path) = state_file {
        PathBuf::from(path)
    } else {
        IndexState::default_path(std::path::Path::new("."))
    };

    if !state_path.exists() {
        println!("No index state found at {}", state_path.display());
        println!("Run `ax index --incremental` to create an index.");
        return Ok(());
    }

    let state = IndexState::load(&state_path)
        .map_err(|e| format!("Failed to load index state from {}: {}", state_path.display(), e))?;

    println!("Index State: {}", state_path.display());
    println!("  Version: {}", state.version);
    println!("  Last updated: {}", state.last_updated.format("%Y-%m-%d %H:%M:%S UTC"));
    println!("  Files indexed: {}", state.file_count());
    println!("  Total chunks: {}", state.total_chunks());

    if state.file_count() > 0 {
        // Show some statistics
        let sizes: Vec<u64> = state.files.values().map(|f| f.size).collect();
        let total_size: u64 = sizes.iter().sum();
        let avg_size = total_size / sizes.len() as u64;

        println!("  Total file size: {}", format_size(total_size));
        println!("  Average file size: {}", format_size(avg_size));

        // Show the 5 most recently indexed files
        let mut recent: Vec<_> = state.files.iter().collect();
        recent.sort_by(|a, b| b.1.indexed_at.cmp(&a.1.indexed_at));

        println!("\n  Recently indexed files:");
        for (path, file_state) in recent.iter().take(5) {
            println!(
                "    {} ({}, {} chunks, indexed {})",
                path,
                format_size(file_state.size),
                file_state.chunks,
                file_state.indexed_at.format("%Y-%m-%d %H:%M:%S")
            );
        }

        if state.file_count() > 5 {
            println!("    ... and {} more files", state.file_count() - 5);
        }
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

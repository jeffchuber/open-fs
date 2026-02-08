use std::path::PathBuf;

use ax_config::VfsConfig;
use ax_core::Vfs;
use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser)]
#[command(name = "ax", version, about = "Agentic Files - Virtual Filesystem")]
struct Cli {
    /// Path to the configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List directory contents
    Ls {
        /// Path to list (defaults to /)
        path: Option<String>,
    },
    /// Display file contents
    Cat {
        /// Path to the file
        path: String,
    },
    /// Write content to a file
    Write {
        /// Path to the file
        path: String,
        /// Content to write (reads from stdin if not provided)
        content: Option<String>,
    },
    /// Append content to a file
    Append {
        /// Path to the file
        path: String,
        /// Content to append (reads from stdin if not provided)
        content: Option<String>,
    },
    /// Remove a file or directory
    Rm {
        /// Path to remove
        path: String,
    },
    /// Show file or directory metadata
    Stat {
        /// Path to inspect
        path: String,
    },
    /// Check if a path exists (exit code 0 if exists, 1 if not)
    Exists {
        /// Path to check
        path: String,
    },
    /// Copy a file
    Cp {
        /// Source path
        src: String,
        /// Destination path
        dst: String,
    },
    /// Move (rename) a file
    Mv {
        /// Source path
        src: String,
        /// Destination path
        dst: String,
    },
    /// Show directory tree
    Tree {
        /// Path to show tree for (defaults to /)
        path: Option<String>,
        /// Maximum depth to recurse
        #[arg(short, long)]
        depth: Option<usize>,
    },
    /// Show effective configuration
    Config,
    /// Find files by name pattern (regex)
    Find {
        /// Regex pattern to match file names
        pattern: String,
        /// Path to search in (defaults to /)
        #[arg(short, long)]
        path: Option<String>,
        /// Filter by type: 'f' for files, 'd' for directories
        #[arg(short = 't', long = "type")]
        file_type: Option<String>,
    },
    /// Search file contents (regex)
    Grep {
        /// Regex pattern to search for
        pattern: String,
        /// Path to search (file or directory)
        path: Option<String>,
        /// Search recursively in directories
        #[arg(short, long)]
        recursive: bool,
    },
    /// Index files for semantic search
    Index {
        /// Path to index (file or directory)
        path: Option<String>,
        /// Chroma endpoint URL (e.g., http://localhost:8000)
        #[arg(long)]
        chroma_endpoint: Option<String>,
        /// Collection name for storing vectors
        #[arg(long)]
        collection: Option<String>,
        /// Index recursively for directories
        #[arg(short, long, default_value = "true")]
        recursive: bool,
        /// Chunking strategy (fixed, recursive, semantic)
        #[arg(long)]
        chunker: Option<String>,
        /// Chunk size in characters
        #[arg(long)]
        chunk_size: Option<usize>,
    },
    /// Semantic search in indexed files
    Search {
        /// Search query
        query: String,
        /// Chroma endpoint URL (e.g., http://localhost:8000)
        #[arg(long)]
        chroma_endpoint: Option<String>,
        /// Collection name to search
        #[arg(long)]
        collection: Option<String>,
        /// Maximum number of results
        #[arg(short, long, default_value = "10")]
        limit: Option<usize>,
        /// Search mode (dense, sparse, hybrid)
        #[arg(short, long)]
        mode: Option<String>,
        /// Number of context lines to show
        #[arg(short, long, default_value = "2")]
        context: Option<usize>,
    },
    /// Show VFS status (mounts, backends, cache stats)
    Status,
    /// Watch for file changes
    Watch {
        /// Path to watch (defaults to /)
        path: Option<String>,
        /// Polling interval in seconds
        #[arg(short, long, default_value = "2")]
        interval: u64,
    },
    /// Generate tool definitions for AI agents
    Tools {
        /// Output format (json, mcp, openai)
        #[arg(short, long, default_value = "json")]
        format: Option<String>,
        /// Pretty-print output
        #[arg(short, long)]
        pretty: bool,
    },
}

fn find_config() -> Option<PathBuf> {
    // 1. AX_CONFIG environment variable
    if let Ok(path) = std::env::var("AX_CONFIG") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    // 2. ax.yaml in current directory
    let cwd_config = PathBuf::from("ax.yaml");
    if cwd_config.exists() {
        return Some(cwd_config);
    }

    // 3. ~/.config/ax/config.yaml
    if let Some(home) = dirs_next::home_dir() {
        let home_config = home.join(".config/ax/config.yaml");
        if home_config.exists() {
            return Some(home_config);
        }
    }

    None
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Find config file
    let config_path = cli.config.or_else(find_config).ok_or(
        "No configuration file found. Use --config, set AX_CONFIG, or create ax.yaml",
    )?;

    // Load and parse config
    let config = VfsConfig::from_file(&config_path)?;

    // Create VFS
    let vfs = Vfs::from_config(config).await?;

    // Execute command
    match cli.command {
        Commands::Ls { path } => {
            commands::ls::run(&vfs, path).await?;
        }
        Commands::Cat { path } => {
            commands::cat::run(&vfs, &path).await?;
        }
        Commands::Write { path, content } => {
            commands::write::run(&vfs, &path, content).await?;
        }
        Commands::Append { path, content } => {
            commands::append::run(&vfs, &path, content).await?;
        }
        Commands::Rm { path } => {
            commands::rm::run(&vfs, &path).await?;
        }
        Commands::Stat { path } => {
            commands::stat::run(&vfs, &path).await?;
        }
        Commands::Exists { path } => {
            commands::exists::run(&vfs, &path).await?;
        }
        Commands::Cp { src, dst } => {
            commands::cp::run(&vfs, &src, &dst).await?;
        }
        Commands::Mv { src, dst } => {
            commands::mv::run(&vfs, &src, &dst).await?;
        }
        Commands::Tree { path, depth } => {
            commands::tree::run(&vfs, path, depth).await?;
        }
        Commands::Config => {
            commands::config::run(&vfs).await?;
        }
        Commands::Find {
            pattern,
            path,
            file_type,
        } => {
            commands::find::run(&vfs, path, &pattern, file_type).await?;
        }
        Commands::Grep {
            pattern,
            path,
            recursive,
        } => {
            commands::grep::run(&vfs, &pattern, path, recursive).await?;
        }
        Commands::Index {
            path,
            chroma_endpoint,
            collection,
            recursive,
            chunker,
            chunk_size,
        } => {
            commands::index::run(&vfs, path, chroma_endpoint, collection, recursive, chunker, chunk_size).await?;
        }
        Commands::Search {
            query,
            chroma_endpoint,
            collection,
            limit,
            mode,
            context,
        } => {
            commands::search::run(&vfs, &query, chroma_endpoint, collection, limit, mode, context).await?;
        }
        Commands::Status => {
            commands::status::run(&vfs).await?;
        }
        Commands::Watch { path, interval } => {
            commands::watch::run(&vfs, path, interval).await?;
        }
        Commands::Tools { format, pretty } => {
            commands::tools::run(&vfs, format, pretty).await?;
        }
    }

    Ok(())
}

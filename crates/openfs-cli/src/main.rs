use std::path::PathBuf;
use std::process::ExitCode;

use openfs_config::VfsConfig;
use openfs_remote::Vfs;
use clap::{Parser, Subcommand};

mod commands;
mod errors;

#[derive(Parser)]
#[command(name = "openfs", version, about = "OpenFS - Virtual Filesystem")]
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
        /// Enable incremental indexing (only re-index changed files)
        #[arg(long)]
        incremental: bool,
        /// Force full re-index, ignoring incremental state
        #[arg(long)]
        force: bool,
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
        /// Polling interval in seconds (defaults to config or 2s)
        #[arg(short, long)]
        interval: Option<u64>,
        /// Use polling mode instead of native notifications
        #[arg(long)]
        poll: bool,
        /// Automatically index changed files
        #[arg(long)]
        auto_index: bool,
        /// Webhook URL to POST change notifications to
        #[arg(long)]
        webhook: Option<String>,
        /// Debounce interval in milliseconds (defaults to config or 500ms)
        #[arg(long)]
        debounce: Option<u64>,
    },
    /// Manage sync behavior for write-back mounts
    Sync {
        #[command(subcommand)]
        action: SyncAction,
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
    /// Mount OpenFS as a FUSE filesystem
    #[cfg_attr(not(feature = "fuse"), command(hide = true))]
    Mount {
        /// Directory to mount the VFS at
        mountpoint: std::path::PathBuf,
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
    },
    /// Unmount OpenFS FUSE filesystem
    Unmount {
        /// Mount point to unmount
        mountpoint: std::path::PathBuf,
        /// Force unmount even if busy
        #[arg(short, long)]
        force: bool,
    },
    /// Show index status (files indexed, total chunks, last updated)
    IndexStatus {
        /// Path to index state file (defaults to .openfs-index-state.json)
        #[arg(long)]
        state_file: Option<String>,
    },
    /// Validate configuration file
    Validate,
    /// Migrate configuration to current version
    Migrate,
    /// Run as an MCP (Model Context Protocol) server over stdio
    Mcp,
    /// Manage the Write-Ahead Log (WAL)
    Wal {
        #[command(subcommand)]
        action: WalAction,
    },
}

#[derive(Subcommand)]
enum WalAction {
    /// Checkpoint: prune old applied WAL entries
    Checkpoint {
        /// Path to the directory containing the WAL database
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Show WAL and outbox status
    Status {
        /// Path to the directory containing the WAL database
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SyncAction {
    /// Show sync and durable outbox status per mount
    Status,
    /// Flush write-back queues and replay durable outbox entries
    Flush,
}

fn find_config() -> Option<PathBuf> {
    // 1. OPENFS_CONFIG environment variable
    if let Ok(path) = std::env::var("OPENFS_CONFIG") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    // 2. openfs.yaml in current directory
    let cwd_config = PathBuf::from("openfs.yaml");
    if cwd_config.exists() {
        return Some(cwd_config);
    }

    // 3. ~/.config/ax/config.yaml
    if let Some(home) = dirs_next::home_dir() {
        let home_config = home.join(".config/openfs/config.yaml");
        if home_config.exists() {
            return Some(home_config);
        }
    }

    None
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    // Find config file
    let config_path = cli
        .config
        .or_else(find_config)
        .ok_or("No configuration file found. Use --config, set OPENFS_CONFIG, or create openfs.yaml")?;

    // Commands that don't need a VFS (or create their own)
    match &cli.command {
        Commands::Validate => {
            return commands::validate::run(&config_path).await;
        }
        Commands::Migrate => {
            return commands::migrate::run(&config_path).await;
        }
        Commands::IndexStatus { state_file } => {
            return commands::index_status::run(state_file.clone()).await;
        }
        Commands::Mcp => {
            return commands::mcp::run(&config_path).await;
        }
        Commands::Wal {
            action: WalAction::Checkpoint { dir },
        } => {
            return commands::wal::run_checkpoint(dir.clone()).await;
        }
        Commands::Wal {
            action: WalAction::Status { dir },
        } => {
            return commands::wal::run_status(dir.clone()).await;
        }
        _ => {}
    }

    // Load and parse config
    let config = VfsConfig::from_file(&config_path)?;

    // Create VFS
    let vfs = Vfs::from_config(config).await?;

    // Execute command
    let mut should_flush = false;
    match cli.command {
        Commands::Ls { path } => {
            commands::ls::run(&vfs, path).await?;
        }
        Commands::Cat { path } => {
            commands::cat::run(&vfs, &path).await?;
        }
        Commands::Write { path, content } => {
            commands::write::run(&vfs, &path, content).await?;
            should_flush = true;
        }
        Commands::Append { path, content } => {
            commands::append::run(&vfs, &path, content).await?;
            should_flush = true;
        }
        Commands::Rm { path } => {
            commands::rm::run(&vfs, &path).await?;
            should_flush = true;
        }
        Commands::Stat { path } => {
            commands::stat::run(&vfs, &path).await?;
        }
        Commands::Exists { path } => {
            commands::exists::run(&vfs, &path).await?;
        }
        Commands::Cp { src, dst } => {
            commands::cp::run(&vfs, &src, &dst).await?;
            should_flush = true;
        }
        Commands::Mv { src, dst } => {
            commands::mv::run(&vfs, &src, &dst).await?;
            should_flush = true;
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
            incremental,
            force,
        } => {
            commands::index::run(
                &vfs,
                path,
                chroma_endpoint,
                collection,
                recursive,
                chunker,
                chunk_size,
                incremental,
                force,
            )
            .await?;
        }
        Commands::Search {
            query,
            chroma_endpoint,
            collection,
            limit,
            mode,
            context,
        } => {
            commands::search::run(
                &vfs,
                &query,
                chroma_endpoint,
                collection,
                limit,
                mode,
                context,
            )
            .await?;
        }
        Commands::Status => {
            commands::status::run(&vfs).await?;
        }
        Commands::Watch {
            path,
            interval,
            poll,
            auto_index,
            webhook,
            debounce,
        } => {
            commands::watch::run(&vfs, path, interval, poll, auto_index, webhook, debounce).await?;
        }
        Commands::Sync { action } => match action {
            SyncAction::Status => {
                commands::sync::run_status(&vfs).await?;
            }
            SyncAction::Flush => {
                commands::sync::run_flush(&vfs).await?;
            }
        },
        Commands::Tools { format, pretty } => {
            commands::tools::run(&vfs, format, pretty).await?;
        }
        Commands::Mount {
            mountpoint,
            foreground,
        } => {
            // Mount doesn't use the already-created VFS instance; it creates its own.
            let config = VfsConfig::from_file(&config_path)?;
            let args = commands::mount::MountArgs {
                mountpoint,
                foreground,
            };
            commands::mount::run(config, args)?;
        }
        Commands::Unmount { mountpoint, force } => {
            let args = commands::unmount::UnmountArgs { mountpoint, force };
            commands::unmount::run(args)?;
        }
        Commands::IndexStatus { state_file } => {
            commands::index_status::run(state_file).await?;
        }
        Commands::Mcp => {
            commands::mcp::run(&config_path).await?;
        }
        // These are handled above before VFS creation; this path is logically
        // unreachable due to the early return, but we return an error instead of
        // panicking if it's ever reached due to a code change.
        Commands::Validate | Commands::Migrate | Commands::Wal { .. } => {
            return Err("Internal error: command should have been handled earlier".into());
        }
    }

    if should_flush {
        vfs.flush_write_back().await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            let code = err.exit_code();
            let code = if code < 0 {
                1u8
            } else if code > 255 {
                255u8
            } else {
                code as u8
            };
            return ExitCode::from(code);
        }
    };

    if let Err(e) = run(cli).await {
        errors::print_error(e.as_ref());
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

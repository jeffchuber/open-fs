use openfs_remote::{WalConfig, WriteAheadLog};
use std::path::PathBuf;

/// Run the WAL checkpoint command.
pub async fn run_checkpoint(config_dir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let wal_paths = resolve_wal_paths(config_dir)?;
    if wal_paths.is_empty() {
        eprintln!("No WAL databases found.");
        return Ok(());
    }

    let mut total_pruned = 0usize;
    for wal_path in wal_paths {
        let wal = WriteAheadLog::new(&wal_path, WalConfig::default())?;
        let pruned = wal.checkpoint()?;
        total_pruned += pruned;
        eprintln!("{}: {} entries pruned", wal_path.display(), pruned);
    }
    eprintln!(
        "WAL checkpoint complete: {} total entries pruned",
        total_pruned
    );
    Ok(())
}

/// Run the WAL status command.
pub async fn run_status(config_dir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let wal_paths = resolve_wal_paths(config_dir)?;
    if wal_paths.is_empty() {
        println!("WAL Status:\n  No WAL databases found.");
        return Ok(());
    }

    let mut total_unapplied = 0usize;
    let mut total_pending = 0usize;
    let mut total_processing = 0usize;
    let mut total_failed = 0usize;

    println!("WAL Status:");
    for wal_path in wal_paths {
        let wal = WriteAheadLog::new(
            &wal_path,
            WalConfig {
                recover_on_startup: false,
                ..Default::default()
            },
        )?;

        let stats = wal.outbox_stats()?;
        let unapplied = wal.get_unapplied()?;
        let failed = wal.get_failed()?;

        total_unapplied += unapplied.len();
        total_pending += stats.pending;
        total_processing += stats.processing;
        total_failed += stats.failed;

        println!(
            "  {}: unapplied {}, pending {}, processing {}, failed {}",
            wal_path.display(),
            unapplied.len(),
            stats.pending,
            stats.processing,
            stats.failed
        );

        if !failed.is_empty() {
            println!("    failed entries:");
            for entry in &failed {
                println!(
                    "      [{}] {} {} (attempts: {}, error: {})",
                    entry.id,
                    entry.op_type.as_str(),
                    entry.path,
                    entry.attempts,
                    entry.error.as_deref().unwrap_or("none")
                );
            }
        }
    }

    println!();
    println!("Totals:");
    println!("  Unapplied entries: {}", total_unapplied);
    println!("  Outbox pending:    {}", total_pending);
    println!("  Outbox processing: {}", total_processing);
    println!("  Outbox failed:     {}", total_failed);

    Ok(())
}

fn resolve_wal_dir(config_dir: Option<PathBuf>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(dir) = config_dir {
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }
        return Ok(dir);
    }

    if let Ok(dir) = std::env::var("OPENFS_WAL_DIR") {
        let path = PathBuf::from(dir);
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
        }
        return Ok(path);
    }

    // Default: .ax in current directory
    let cwd = std::env::current_dir()?;
    let wal_dir = cwd.join(".ax");
    if !wal_dir.exists() {
        std::fs::create_dir_all(&wal_dir)?;
    }
    Ok(wal_dir)
}

fn resolve_wal_paths(
    config_dir: Option<PathBuf>,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let wal_dir = resolve_wal_dir(config_dir)?;
    let mut paths = Vec::new();

    // Legacy single-file layout.
    let legacy = wal_dir.join("wal.db");
    if legacy.exists() {
        paths.push(legacy);
    }

    // Current per-mount layout.
    for entry in std::fs::read_dir(&wal_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("wal_") && name.ends_with(".db") {
            paths.push(path);
        }
    }

    paths.sort();
    Ok(paths)
}

use ax_remote::{WriteAheadLog, WalConfig};
use std::path::PathBuf;

/// Run the WAL checkpoint command.
pub async fn run_checkpoint(config_dir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let wal_path = resolve_wal_path(config_dir)?;
    let wal = WriteAheadLog::new(&wal_path, WalConfig::default())?;
    let pruned = wal.checkpoint()?;
    eprintln!("WAL checkpoint complete: {} entries pruned", pruned);
    Ok(())
}

/// Run the WAL status command.
pub async fn run_status(config_dir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let wal_path = resolve_wal_path(config_dir)?;
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

    println!("WAL Status:");
    println!("  Unapplied entries: {}", unapplied.len());
    println!("  Outbox pending:    {}", stats.pending);
    println!("  Outbox processing: {}", stats.processing);
    println!("  Outbox failed:     {}", stats.failed);

    if !failed.is_empty() {
        println!("\nFailed entries:");
        for entry in &failed {
            println!(
                "  [{}] {} {} (attempts: {}, error: {})",
                entry.id,
                entry.op_type.as_str(),
                entry.path,
                entry.attempts,
                entry.error.as_deref().unwrap_or("none")
            );
        }
    }

    Ok(())
}

fn resolve_wal_path(config_dir: Option<PathBuf>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(dir) = config_dir {
        return Ok(dir.join("ax-wal.db"));
    }
    // Default: .ax/wal.db in current directory
    let cwd = std::env::current_dir()?;
    let wal_dir = cwd.join(".ax");
    if !wal_dir.exists() {
        std::fs::create_dir_all(&wal_dir)?;
    }
    Ok(wal_dir.join("wal.db"))
}

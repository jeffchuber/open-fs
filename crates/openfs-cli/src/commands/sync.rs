use openfs_remote::{SyncMode, Vfs};

fn sync_mode_label(mode: SyncMode) -> &'static str {
    match mode {
        SyncMode::None => "none",
        SyncMode::WriteThrough => "write-through",
        SyncMode::WriteBack => "write-back",
        SyncMode::PullMirror => "pull-mirror",
    }
}

pub async fn run_status(vfs: &Vfs) -> Result<(), Box<dyn std::error::Error>> {
    let statuses = vfs.sync_statuses().await?;

    println!("OpenFS Sync Status");
    println!("==============");
    println!();

    for status in statuses {
        println!(
            "{} -> {} (mode: {}, read_only: {})",
            status.mount_path,
            status.backend_name,
            sync_mode_label(status.sync_mode),
            status.read_only
        );
        println!(
            "  pending: {}, synced: {}, failed: {}, retries: {}",
            status.pending, status.synced, status.failed, status.retries
        );
        if let (Some(pending), Some(processing), Some(failed), Some(unapplied)) = (
            status.outbox_pending,
            status.outbox_processing,
            status.outbox_failed,
            status.outbox_wal_unapplied,
        ) {
            println!(
                "  outbox: pending {}, processing {}, failed {}, wal_unapplied {}",
                pending, processing, failed, unapplied
            );
        }
        println!();
    }

    Ok(())
}

pub async fn run_flush(vfs: &Vfs) -> Result<(), Box<dyn std::error::Error>> {
    let mounts = vfs.flush_write_back().await?;
    println!("Flushed write-back sync state for {} mount(s).", mounts);
    Ok(())
}

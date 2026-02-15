use ax_remote::Vfs;

fn sync_mode_label(mode: ax_remote::SyncMode) -> &'static str {
    match mode {
        ax_remote::SyncMode::None => "none",
        ax_remote::SyncMode::WriteThrough => "write-through",
        ax_remote::SyncMode::WriteBack => "write-back",
        ax_remote::SyncMode::PullMirror => "pull-mirror",
    }
}

pub async fn run(vfs: &Vfs) -> Result<(), Box<dyn std::error::Error>> {
    let config = vfs.effective_config();
    let sync_statuses = vfs.sync_statuses().await?;

    println!("AX Status");
    println!("=========");
    println!();

    // VFS Info
    println!("VFS: {}", config.name.as_deref().unwrap_or("unnamed"));
    if let Some(version) = &config.version {
        println!("Version: {}", version);
    }
    println!();

    // Backends
    println!("Backends:");
    for (name, backend) in &config.backends {
        let backend_type = match backend {
            ax_config::BackendConfig::Fs(_) => "fs",
            ax_config::BackendConfig::Memory(_) => "memory",
            ax_config::BackendConfig::Chroma(_) => "chroma",
            ax_config::BackendConfig::S3(_) => "s3",
            ax_config::BackendConfig::Postgres(_) => "postgres",
            _ => "unknown",
        };
        println!("  {} ({})", name, backend_type);
    }
    println!();

    // Mounts
    println!("Mounts:");
    for mount in &config.mounts {
        let mode = mount.mode.as_ref().map_or("default", |m| match m {
            ax_config::MountMode::Local => "local",
            ax_config::MountMode::LocalIndexed => "local-indexed",
            ax_config::MountMode::WriteThrough => "write-through",
            ax_config::MountMode::WriteBack => "write-back",
            ax_config::MountMode::Remote => "remote",
            ax_config::MountMode::RemoteCached => "remote-cached",
            ax_config::MountMode::PullMirror => "pull-mirror",
            _ => "unknown",
        });

        let read_only = if mount.read_only {
            " [read-only]"
        } else {
            ""
        };

        let backend = mount.backend.as_deref().unwrap_or("(implicit)");

        println!(
            "  {} -> {} (mode: {}){}",
            mount.path, backend, mode, read_only
        );
    }
    println!();

    println!("Sync:");
    for status in sync_statuses {
        println!(
            "  {} -> {} (mode: {}, read_only: {})",
            status.mount_path,
            status.backend_name,
            sync_mode_label(status.sync_mode),
            status.read_only
        );
        println!(
            "    pending: {}, synced: {}, failed: {}, retries: {}",
            status.pending, status.synced, status.failed, status.retries
        );
        if let (Some(pending), Some(processing), Some(failed), Some(unapplied)) = (
            status.outbox_pending,
            status.outbox_processing,
            status.outbox_failed,
            status.outbox_wal_unapplied,
        ) {
            println!(
                "    outbox: pending {}, processing {}, failed {}, wal_unapplied {}",
                pending, processing, failed, unapplied
            );
        }
    }
    println!();

    println!("Status: OK");

    Ok(())
}

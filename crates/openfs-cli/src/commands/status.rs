use openfs_remote::Vfs;

fn sync_mode_label(mode: openfs_remote::SyncMode) -> &'static str {
    match mode {
        openfs_remote::SyncMode::None => "none",
        openfs_remote::SyncMode::WriteThrough => "write-through",
        openfs_remote::SyncMode::WriteBack => "write-back",
        openfs_remote::SyncMode::PullMirror => "pull-mirror",
    }
}

pub async fn run(vfs: &Vfs) -> Result<(), Box<dyn std::error::Error>> {
    let config = vfs.effective_config();
    let sync_statuses = vfs.sync_statuses().await?;

    println!("OpenFS Status");
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
            openfs_config::BackendConfig::Fs(_) => "fs",
            openfs_config::BackendConfig::Memory(_) => "memory",
            openfs_config::BackendConfig::Chroma(_) => "chroma",
            openfs_config::BackendConfig::S3(_) => "s3",
            openfs_config::BackendConfig::Postgres(_) => "postgres",
            _ => "unknown",
        };
        println!("  {} ({})", name, backend_type);
    }
    println!();

    // Mounts
    println!("Mounts:");
    for mount in &config.mounts {
        let mode = mount.mode.as_ref().map_or("default", |m| match m {
            openfs_config::MountMode::Local => "local",
            openfs_config::MountMode::LocalIndexed => "local-indexed",
            openfs_config::MountMode::WriteThrough => "write-through",
            openfs_config::MountMode::WriteBack => "write-back",
            openfs_config::MountMode::Remote => "remote",
            openfs_config::MountMode::RemoteCached => "remote-cached",
            openfs_config::MountMode::PullMirror => "pull-mirror",
            _ => "unknown",
        });

        let read_only = if mount.read_only { " [read-only]" } else { "" };

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

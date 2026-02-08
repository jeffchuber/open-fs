use ax_core::Vfs;

pub async fn run(vfs: &Vfs) -> Result<(), Box<dyn std::error::Error>> {
    let config = vfs.effective_config();

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
            ax_config::BackendConfig::Chroma(_) => "chroma",
            ax_config::BackendConfig::S3(_) => "s3",
            ax_config::BackendConfig::Postgres(_) => "postgres",
            ax_config::BackendConfig::Api(_) => "api",
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

    // Cache/Sync would go here if we had access to those stats
    // For now, just show basic info

    println!("Status: OK");

    Ok(())
}

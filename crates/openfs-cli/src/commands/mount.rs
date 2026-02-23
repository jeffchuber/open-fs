//! Mount command for AX FUSE filesystem.

use std::path::PathBuf;

use openfs_config::VfsConfig;
use openfs_fuse::OpenFsFuse;

/// Mount arguments.
pub struct MountArgs {
    /// Mount point path.
    pub mountpoint: PathBuf,
    /// Run in foreground (don't daemonize).
    pub foreground: bool,
}

/// Run the mount command.
///
/// Note: This function does not take a Vfs reference because it needs
/// to create and own the FUSE filesystem. The config is loaded separately.
pub fn run(config: VfsConfig, args: MountArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Ensure mount point exists
    if !args.mountpoint.exists() {
        std::fs::create_dir_all(&args.mountpoint)?;
    }

    // Create FUSE filesystem
    let ax = OpenFsFuse::from_config(config)?;

    // Mount (this blocks until unmount)
    if args.foreground {
        ax.mount_foreground(&args.mountpoint)?;
    } else {
        ax.mount(&args.mountpoint)?;
    }

    Ok(())
}

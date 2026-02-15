//! Unmount command for AX FUSE filesystem.

use std::path::PathBuf;
use std::process::Command;

/// Unmount arguments.
pub struct UnmountArgs {
    /// Mount point path to unmount.
    pub mountpoint: PathBuf,
    /// Force unmount even if busy.
    pub force: bool,
}

/// Run the unmount command.
pub fn run(args: UnmountArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mountpoint = args.mountpoint.canonicalize().unwrap_or(args.mountpoint.clone());

    // Platform-specific unmount
    #[cfg(target_os = "macos")]
    {
        unmount_macos(&mountpoint, args.force)?;
    }

    #[cfg(target_os = "linux")]
    {
        unmount_linux(&mountpoint, args.force)?;
    }

    #[cfg(target_os = "windows")]
    {
        unmount_windows(&mountpoint, args.force)?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        return Err("Unmount not supported on this platform".into());
    }

    println!("Unmounted {}", mountpoint.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn unmount_macos(mountpoint: &PathBuf, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new("umount");

    if force {
        cmd.arg("-f");
    }

    cmd.arg(mountpoint);

    let output = cmd.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("umount failed: {}", stderr).into());
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn unmount_linux(mountpoint: &PathBuf, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Try fusermount first (preferred for FUSE)
    let mut cmd = Command::new("fusermount");
    cmd.arg("-u");

    if force {
        cmd.arg("-z"); // Lazy unmount
    }

    cmd.arg(mountpoint);

    let output = cmd.output();

    match output {
        Ok(out) if out.status.success() => return Ok(()),
        _ => {
            // Fall back to umount
            let mut cmd = Command::new("umount");

            if force {
                cmd.arg("-l"); // Lazy unmount
            }

            cmd.arg(mountpoint);

            let output = cmd.output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("umount failed: {}", stderr).into());
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn unmount_windows(mountpoint: &PathBuf, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mountpoint_str = mountpoint
        .to_str()
        .ok_or("Invalid mount point path")?;

    // For drive letter mounts (e.g., X:), use "net use X: /delete"
    if mountpoint_str.len() == 2 && mountpoint_str.ends_with(':') {
        let mut cmd = Command::new("net");
        cmd.args(["use", mountpoint_str, "/delete"]);

        if force {
            cmd.arg("/yes");
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("net use /delete failed: {}", stderr).into());
        }
    } else {
        // For directory mount points, use WinFsp's launchctl or direct removal
        let mut cmd = Command::new("launchctl-x64");
        cmd.args(["stop", mountpoint_str]);

        let output = cmd.output();

        match output {
            Ok(out) if out.status.success() => {}
            _ => {
                let mut cmd = Command::new("net");
                cmd.args(["use", mountpoint_str, "/delete"]);
                if force {
                    cmd.arg("/yes");
                }

                let output = cmd.output()?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("Failed to unmount: {}", stderr).into());
                }
            }
        }
    }

    Ok(())
}

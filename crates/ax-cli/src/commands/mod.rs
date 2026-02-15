pub mod append;
pub mod cat;
pub mod config;
pub mod cp;
pub mod exists;
pub mod find;
pub mod grep;
pub mod index;
pub mod index_status;
pub mod ls;
pub mod mcp;
pub mod migrate;
#[cfg(feature = "fuse")]
pub mod mount;
#[cfg(not(feature = "fuse"))]
pub mod mount {
    use std::path::PathBuf;

    use ax_config::VfsConfig;

    /// Mount arguments.
    #[allow(dead_code)]
    pub struct MountArgs {
        /// Mount point path.
        pub mountpoint: PathBuf,
        /// Run in foreground (don't daemonize).
        pub foreground: bool,
    }

    /// Run the mount command when FUSE support is disabled.
    pub fn run(_config: VfsConfig, _args: MountArgs) -> Result<(), Box<dyn std::error::Error>> {
        Err("FUSE support is disabled in this build. Rebuild ax-cli with --features fuse.".into())
    }
}
pub mod mv;
pub mod rm;
pub mod search;
pub mod sync;
pub mod stat;
pub mod status;
pub mod tools;
pub mod tree;
pub mod unmount;
pub mod validate;
pub mod wal;
pub mod watch;
pub mod write;

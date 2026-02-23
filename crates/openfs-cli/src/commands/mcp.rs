use std::path::Path;
use std::sync::Arc;

use openfs_config::VfsConfig;
use openfs_mcp::{McpHandler, McpServer};
use openfs_remote::Vfs;

pub async fn run(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let config = VfsConfig::from_file(config_path)?;
    let vfs = Arc::new(Vfs::from_config(config).await?);
    let handler = McpHandler::new(vfs);
    let server = McpServer::new(handler);
    server.run().await
}

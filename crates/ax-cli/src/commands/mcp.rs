use std::path::Path;
use std::sync::Arc;

use ax_config::VfsConfig;
use ax_remote::Vfs;
use ax_mcp::{McpHandler, McpServer};

pub async fn run(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let config = VfsConfig::from_file(config_path)?;
    let vfs = Arc::new(Vfs::from_config(config).await?);
    let handler = McpHandler::new(vfs);
    let server = McpServer::new(handler);
    server.run().await
}

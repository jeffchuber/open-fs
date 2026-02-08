#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use tokio::runtime::Runtime;

use ax_config::VfsConfig;
use ax_core::{format_tools, generate_tools, ToolFormat, Vfs, VfsError};

/// Convert VfsError to napi Error
fn vfs_error_to_napi(err: VfsError) -> Error {
    Error::from_reason(err.to_string())
}

/// File entry returned from list operations.
#[napi(object)]
pub struct JsEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<i64>,
}

/// AX Virtual Filesystem for JavaScript/TypeScript.
#[napi]
pub struct JsVfs {
    vfs: Arc<Vfs>,
    runtime: Arc<Runtime>,
}

#[napi]
impl JsVfs {
    /// Create a new VFS from a YAML configuration string.
    #[napi(factory)]
    pub fn from_yaml(yaml: String) -> Result<Self> {
        let config = VfsConfig::from_yaml(&yaml)
            .map_err(|e| Error::from_reason(format!("Config error: {}", e)))?;

        let runtime =
            Runtime::new().map_err(|e| Error::from_reason(format!("Runtime error: {}", e)))?;

        let vfs = runtime
            .block_on(async { Vfs::from_config(config).await })
            .map_err(vfs_error_to_napi)?;

        Ok(JsVfs {
            vfs: Arc::new(vfs),
            runtime: Arc::new(runtime),
        })
    }

    /// Create a new VFS from a YAML configuration file.
    #[napi(factory)]
    pub fn from_file(path: String) -> Result<Self> {
        let path = std::path::Path::new(&path);
        let config = VfsConfig::from_file(path)
            .map_err(|e| Error::from_reason(format!("Config error: {}", e)))?;

        let runtime =
            Runtime::new().map_err(|e| Error::from_reason(format!("Runtime error: {}", e)))?;

        let vfs = runtime
            .block_on(async { Vfs::from_config(config).await })
            .map_err(vfs_error_to_napi)?;

        Ok(JsVfs {
            vfs: Arc::new(vfs),
            runtime: Arc::new(runtime),
        })
    }

    /// Read the contents of a file as a Buffer.
    #[napi]
    pub fn read(&self, path: String) -> Result<Buffer> {
        let vfs = Arc::clone(&self.vfs);

        let data = self
            .runtime
            .block_on(async move { vfs.read(&path).await })
            .map_err(vfs_error_to_napi)?;

        Ok(data.into())
    }

    /// Read the contents of a file as a string.
    #[napi]
    pub fn read_text(&self, path: String) -> Result<String> {
        let vfs = Arc::clone(&self.vfs);

        let data = self
            .runtime
            .block_on(async move { vfs.read(&path).await })
            .map_err(vfs_error_to_napi)?;

        String::from_utf8(data).map_err(|e| Error::from_reason(format!("UTF-8 decode error: {}", e)))
    }

    /// Write content to a file.
    #[napi]
    pub fn write(&self, path: String, content: Buffer) -> Result<()> {
        let vfs = Arc::clone(&self.vfs);
        let content = content.to_vec();

        self.runtime
            .block_on(async move { vfs.write(&path, &content).await })
            .map_err(vfs_error_to_napi)
    }

    /// Write a string to a file.
    #[napi]
    pub fn write_text(&self, path: String, content: String) -> Result<()> {
        let vfs = Arc::clone(&self.vfs);
        let content = content.into_bytes();

        self.runtime
            .block_on(async move { vfs.write(&path, &content).await })
            .map_err(vfs_error_to_napi)
    }

    /// Append content to a file.
    #[napi]
    pub fn append(&self, path: String, content: Buffer) -> Result<()> {
        let vfs = Arc::clone(&self.vfs);
        let content = content.to_vec();

        self.runtime
            .block_on(async move { vfs.append(&path, &content).await })
            .map_err(vfs_error_to_napi)
    }

    /// Append a string to a file.
    #[napi]
    pub fn append_text(&self, path: String, content: String) -> Result<()> {
        let vfs = Arc::clone(&self.vfs);
        let content = content.into_bytes();

        self.runtime
            .block_on(async move { vfs.append(&path, &content).await })
            .map_err(vfs_error_to_napi)
    }

    /// Delete a file.
    #[napi]
    pub fn delete(&self, path: String) -> Result<()> {
        let vfs = Arc::clone(&self.vfs);

        self.runtime
            .block_on(async move { vfs.delete(&path).await })
            .map_err(vfs_error_to_napi)
    }

    /// List files in a directory.
    #[napi]
    pub fn list(&self, path: String) -> Result<Vec<JsEntry>> {
        let vfs = Arc::clone(&self.vfs);

        let entries = self
            .runtime
            .block_on(async move { vfs.list(&path).await })
            .map_err(vfs_error_to_napi)?;

        Ok(entries
            .into_iter()
            .map(|e| JsEntry {
                path: e.path,
                name: e.name,
                is_dir: e.is_dir,
                size: e.size.map(|s| s as i64),
            })
            .collect())
    }

    /// Check if a path exists.
    #[napi]
    pub fn exists(&self, path: String) -> Result<bool> {
        let vfs = Arc::clone(&self.vfs);

        self.runtime
            .block_on(async move { vfs.exists(&path).await })
            .map_err(vfs_error_to_napi)
    }

    /// Get metadata for a path.
    #[napi]
    pub fn stat(&self, path: String) -> Result<JsEntry> {
        let vfs = Arc::clone(&self.vfs);

        let entry = self
            .runtime
            .block_on(async move { vfs.stat(&path).await })
            .map_err(vfs_error_to_napi)?;

        Ok(JsEntry {
            path: entry.path,
            name: entry.name,
            is_dir: entry.is_dir,
            size: entry.size.map(|s| s as i64),
        })
    }

    /// Generate tool definitions in JSON format.
    ///
    /// @param format - Output format: 'json', 'mcp', or 'openai'
    #[napi]
    pub fn tools(&self, format: Option<String>) -> Result<String> {
        let config = self.vfs.effective_config();
        let tools = generate_tools(config);

        let tool_format: ToolFormat = format
            .as_deref()
            .unwrap_or("json")
            .parse()
            .map_err(|e: String| Error::from_reason(e))?;

        let output = format_tools(&tools, tool_format);
        serde_json::to_string_pretty(&output)
            .map_err(|e| Error::from_reason(format!("JSON error: {}", e)))
    }

    /// Get the VFS name.
    #[napi]
    pub fn name(&self) -> Option<String> {
        self.vfs.effective_config().name.clone()
    }

    /// Get mount paths.
    #[napi]
    pub fn mounts(&self) -> Vec<String> {
        self.vfs
            .effective_config()
            .mounts
            .iter()
            .map(|m| m.path.clone())
            .collect()
    }
}

/// Parse a YAML configuration string and return a VFS.
#[napi]
pub fn load_config(yaml: String) -> Result<JsVfs> {
    JsVfs::from_yaml(yaml)
}

/// Load a VFS from a configuration file.
#[napi]
pub fn load_config_file(path: String) -> Result<JsVfs> {
    JsVfs::from_file(path)
}

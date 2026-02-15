use pyo3::prelude::*;
use pyo3::exceptions::{PyIOError, PyValueError};
use std::sync::Arc;
use tokio::runtime::Runtime;

use ax_config::VfsConfig;
use ax_core::{Vfs, VfsError, GrepOptions, generate_tools, format_tools, ToolFormat};

/// Python wrapper for VfsError
fn vfs_error_to_py(err: VfsError) -> PyErr {
    PyIOError::new_err(err.to_string())
}

/// File entry returned from list operations.
#[pyclass]
#[derive(Clone)]
pub struct PyEntry {
    #[pyo3(get)]
    pub path: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub is_dir: bool,
    #[pyo3(get)]
    pub size: Option<u64>,
}

#[pymethods]
impl PyEntry {
    fn __repr__(&self) -> String {
        if self.is_dir {
            format!("Entry(path='{}', is_dir=True)", self.path)
        } else {
            format!("Entry(path='{}', size={})", self.path, self.size.unwrap_or(0))
        }
    }
}

/// A single grep match.
#[pyclass]
#[derive(Clone)]
pub struct PyGrepMatch {
    #[pyo3(get)]
    pub path: String,
    #[pyo3(get)]
    pub line_number: usize,
    #[pyo3(get)]
    pub line: String,
}

#[pymethods]
impl PyGrepMatch {
    fn __repr__(&self) -> String {
        format!("GrepMatch(path='{}', line_number={}, line='{}')", self.path, self.line_number, self.line)
    }
}

/// Python wrapper for the AX Virtual Filesystem.
#[pyclass]
pub struct PyVfs {
    vfs: Arc<Vfs>,
    runtime: Arc<Runtime>,
}

#[pymethods]
impl PyVfs {
    /// Create a new VFS from a YAML configuration string.
    #[staticmethod]
    fn from_yaml(yaml: &str) -> PyResult<Self> {
        let config = VfsConfig::from_yaml(yaml)
            .map_err(|e| PyValueError::new_err(format!("Config error: {}", e)))?;

        let runtime = Runtime::new()
            .map_err(|e| PyIOError::new_err(format!("Failed to create runtime: {}", e)))?;

        let vfs = runtime.block_on(async {
            Vfs::from_config(config).await
        }).map_err(vfs_error_to_py)?;

        Ok(PyVfs {
            vfs: Arc::new(vfs),
            runtime: Arc::new(runtime),
        })
    }

    /// Create a new VFS from a YAML configuration file.
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Self> {
        let path = std::path::Path::new(path);
        let config = VfsConfig::from_file(path)
            .map_err(|e| PyValueError::new_err(format!("Config error: {}", e)))?;

        let runtime = Runtime::new()
            .map_err(|e| PyIOError::new_err(format!("Failed to create runtime: {}", e)))?;

        let vfs = runtime.block_on(async {
            Vfs::from_config(config).await
        }).map_err(vfs_error_to_py)?;

        Ok(PyVfs {
            vfs: Arc::new(vfs),
            runtime: Arc::new(runtime),
        })
    }

    /// Read the contents of a file.
    fn read(&self, path: &str) -> PyResult<Vec<u8>> {
        let vfs = Arc::clone(&self.vfs);
        let path = path.to_string();

        self.runtime.block_on(async move {
            vfs.read(&path).await
        }).map_err(vfs_error_to_py)
    }

    /// Read the contents of a file as a string.
    fn read_text(&self, path: &str) -> PyResult<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes)
            .map_err(|e| PyValueError::new_err(format!("UTF-8 decode error: {}", e)))
    }

    /// Write content to a file.
    fn write(&self, path: &str, content: &[u8]) -> PyResult<()> {
        let vfs = Arc::clone(&self.vfs);
        let path = path.to_string();
        let content = content.to_vec();

        self.runtime.block_on(async move {
            vfs.write(&path, &content).await
        }).map_err(vfs_error_to_py)
    }

    /// Write a string to a file.
    fn write_text(&self, path: &str, content: &str) -> PyResult<()> {
        self.write(path, content.as_bytes())
    }

    /// Append content to a file.
    fn append(&self, path: &str, content: &[u8]) -> PyResult<()> {
        let vfs = Arc::clone(&self.vfs);
        let path = path.to_string();
        let content = content.to_vec();

        self.runtime.block_on(async move {
            vfs.append(&path, &content).await
        }).map_err(vfs_error_to_py)
    }

    /// Append a string to a file.
    fn append_text(&self, path: &str, content: &str) -> PyResult<()> {
        self.append(path, content.as_bytes())
    }

    /// Delete a file.
    fn delete(&self, path: &str) -> PyResult<()> {
        let vfs = Arc::clone(&self.vfs);
        let path = path.to_string();

        self.runtime.block_on(async move {
            vfs.delete(&path).await
        }).map_err(vfs_error_to_py)
    }

    /// List files in a directory.
    fn list(&self, path: &str) -> PyResult<Vec<PyEntry>> {
        let vfs = Arc::clone(&self.vfs);
        let path = path.to_string();

        let entries = self.runtime.block_on(async move {
            vfs.list(&path).await
        }).map_err(vfs_error_to_py)?;

        Ok(entries
            .into_iter()
            .map(|e| PyEntry {
                path: e.path,
                name: e.name,
                is_dir: e.is_dir,
                size: e.size,
            })
            .collect())
    }

    /// Check if a path exists.
    fn exists(&self, path: &str) -> PyResult<bool> {
        let vfs = Arc::clone(&self.vfs);
        let path = path.to_string();

        self.runtime.block_on(async move {
            vfs.exists(&path).await
        }).map_err(vfs_error_to_py)
    }

    /// Get metadata for a path.
    fn stat(&self, path: &str) -> PyResult<PyEntry> {
        let vfs = Arc::clone(&self.vfs);
        let path = path.to_string();

        let entry = self.runtime.block_on(async move {
            vfs.stat(&path).await
        }).map_err(vfs_error_to_py)?;

        Ok(PyEntry {
            path: entry.path,
            name: entry.name,
            is_dir: entry.is_dir,
            size: entry.size,
        })
    }

    /// Generate tool definitions in JSON format.
    fn tools(&self, format: Option<&str>) -> PyResult<String> {
        let config = self.vfs.effective_config();
        let tools = generate_tools(config);

        let tool_format: ToolFormat = format
            .unwrap_or("json")
            .parse()
            .map_err(|e: String| PyValueError::new_err(e))?;

        let output = format_tools(&tools, tool_format);
        serde_json::to_string_pretty(&output)
            .map_err(|e| PyValueError::new_err(format!("JSON error: {}", e)))
    }

    /// Get the VFS name.
    fn name(&self) -> Option<String> {
        self.vfs.effective_config().name.clone()
    }

    /// Get mount paths.
    fn mounts(&self) -> Vec<String> {
        self.vfs
            .effective_config()
            .mounts
            .iter()
            .map(|m| m.path.clone())
            .collect()
    }

    /// Rename/move a file.
    fn rename(&self, from_path: &str, to_path: &str) -> PyResult<()> {
        let vfs = Arc::clone(&self.vfs);
        let from = from_path.to_string();
        let to = to_path.to_string();

        self.runtime.block_on(async move {
            vfs.rename(&from, &to).await
        }).map_err(vfs_error_to_py)
    }

    /// Copy a file. Returns the number of bytes copied.
    fn copy(&self, src: &str, dst: &str) -> PyResult<usize> {
        let vfs = Arc::clone(&self.vfs);
        let src = src.to_string();
        let dst = dst.to_string();

        self.runtime.block_on(async move {
            let content = vfs.read(&src).await?;
            let len = content.len();
            vfs.write(&dst, &content).await?;
            Ok::<_, VfsError>(len)
        }).map_err(vfs_error_to_py)
    }

    /// Search files for lines matching a regex pattern.
    #[pyo3(signature = (pattern, path=None, recursive=None))]
    fn grep(&self, pattern: &str, path: Option<&str>, recursive: Option<bool>) -> PyResult<Vec<PyGrepMatch>> {
        let vfs = Arc::clone(&self.vfs);
        let pattern = pattern.to_string();
        let path = path.unwrap_or("/").to_string();
        let recursive = recursive.unwrap_or(false);

        let matches = self.runtime.block_on(async move {
            let opts = GrepOptions {
                recursive,
                ..Default::default()
            };
            ax_core::grep(&vfs, &pattern, &path, &opts).await
        }).map_err(vfs_error_to_py)?;

        Ok(matches.into_iter().map(|m| PyGrepMatch {
            path: m.path,
            line_number: m.line_number,
            line: m.line,
        }).collect())
    }

    fn __repr__(&self) -> String {
        let name = self.name().unwrap_or_else(|| "unnamed".to_string());
        let mounts = self.mounts();
        format!("Vfs(name='{}', mounts={:?})", name, mounts)
    }
}

/// Parse a YAML configuration string and return a VFS.
#[pyfunction]
fn load_config(yaml: &str) -> PyResult<PyVfs> {
    PyVfs::from_yaml(yaml)
}

/// Load a VFS from a configuration file.
#[pyfunction]
fn load_config_file(path: &str) -> PyResult<PyVfs> {
    PyVfs::from_file(path)
}

/// AX Python module.
#[pymodule]
fn ax(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyVfs>()?;
    m.add_class::<PyEntry>()?;
    m.add_class::<PyGrepMatch>()?;
    m.add_function(wrap_pyfunction!(load_config, m)?)?;
    m.add_function(wrap_pyfunction!(load_config_file, m)?)?;
    Ok(())
}

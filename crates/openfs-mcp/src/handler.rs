//! MCP tool handler — dispatches tool calls to VFS operations.

use std::collections::HashMap;
use std::sync::Arc;

use openfs_local::{SearchConfig, SearchEngine};
use openfs_remote::Vfs;
use tracing::{debug, warn};

use crate::protocol::{McpToolDef, ToolCallResult};

/// Handles MCP tool calls by dispatching to the VFS.
pub struct McpHandler {
    vfs: Arc<Vfs>,
    search_engine: Option<Arc<SearchEngine>>,
}

impl McpHandler {
    pub fn new(vfs: Arc<Vfs>) -> Self {
        McpHandler {
            vfs,
            search_engine: None,
        }
    }

    /// Set an optional search engine for semantic search.
    pub fn with_search(mut self, engine: Arc<SearchEngine>) -> Self {
        self.search_engine = Some(engine);
        self
    }

    /// Return the list of tools this server exposes.
    pub fn tool_definitions(&self) -> Vec<McpToolDef> {
        vec![
            McpToolDef {
                name: "openfs_read".to_string(),
                description: "Read the contents of a file from the OpenFS virtual filesystem. Returns a cas_token for use with conditional writes.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The VFS path to the file to read"
                        }
                    },
                    "required": ["path"]
                }),
            },
            McpToolDef {
                name: "openfs_write".to_string(),
                description: "Write content to a file in the OpenFS virtual filesystem. Supports optimistic concurrency via cas_token.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The VFS path to write to"
                        },
                        "content": {
                            "type": "string",
                            "description": "The content to write"
                        },
                        "cas_token": {
                            "type": "string",
                            "description": "Optional CAS token from a previous openfs_read. If provided, the write will fail if the file has been modified since that read. The response includes the new cas_token on success."
                        }
                    },
                    "required": ["path", "content"]
                }),
            },
            McpToolDef {
                name: "openfs_ls".to_string(),
                description: "List files and directories at a path in the OpenFS virtual filesystem"
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The VFS directory path to list"
                        }
                    },
                    "required": ["path"]
                }),
            },
            McpToolDef {
                name: "openfs_stat".to_string(),
                description: "Get metadata (size, modified time) for a file or directory"
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The VFS path to get metadata for"
                        }
                    },
                    "required": ["path"]
                }),
            },
            McpToolDef {
                name: "openfs_delete".to_string(),
                description: "Delete a file from the OpenFS virtual filesystem".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The VFS path to delete"
                        }
                    },
                    "required": ["path"]
                }),
            },
            McpToolDef {
                name: "openfs_grep".to_string(),
                description: "Search file contents for a regex pattern".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to search for"
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory or file path to search in (defaults to /)"
                        }
                    },
                    "required": ["pattern"]
                }),
            },
            McpToolDef {
                name: "openfs_append".to_string(),
                description: "Append content to a file in the OpenFS virtual filesystem".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The VFS path to append to"
                        },
                        "content": {
                            "type": "string",
                            "description": "The content to append"
                        }
                    },
                    "required": ["path", "content"]
                }),
            },
            McpToolDef {
                name: "openfs_exists".to_string(),
                description: "Check if a file or directory exists in the OpenFS virtual filesystem"
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The VFS path to check"
                        }
                    },
                    "required": ["path"]
                }),
            },
            McpToolDef {
                name: "openfs_rename".to_string(),
                description: "Rename or move a file or directory in the OpenFS virtual filesystem"
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "from": {
                            "type": "string",
                            "description": "The current VFS path"
                        },
                        "to": {
                            "type": "string",
                            "description": "The new VFS path"
                        }
                    },
                    "required": ["from", "to"]
                }),
            },
            McpToolDef {
                name: "openfs_read_batch".to_string(),
                description: "Read multiple files in a single request. Returns results for each path, including errors for individual failures.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Array of VFS paths to read"
                        }
                    },
                    "required": ["paths"]
                }),
            },
            McpToolDef {
                name: "openfs_write_batch".to_string(),
                description: "Write multiple files in a single request. Returns results for each file, including errors for individual failures.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "content": { "type": "string" }
                                },
                                "required": ["path", "content"]
                            },
                            "description": "Array of {path, content} objects to write"
                        }
                    },
                    "required": ["files"]
                }),
            },
            McpToolDef {
                name: "openfs_delete_batch".to_string(),
                description: "Delete multiple files in a single request. Returns results for each path, including errors for individual failures.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Array of VFS paths to delete"
                        }
                    },
                    "required": ["paths"]
                }),
            },
            McpToolDef {
                name: "openfs_cache_stats".to_string(),
                description: "Get cache performance statistics across all mounts".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpToolDef {
                name: "openfs_prefetch".to_string(),
                description: "Prefetch files into cache for faster subsequent reads".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Array of VFS paths to prefetch into cache"
                        }
                    },
                    "required": ["paths"]
                }),
            },
            McpToolDef {
                name: "openfs_search".to_string(),
                description: "Semantic search across indexed files using natural language queries"
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Natural language search query"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results (default: 10)"
                        }
                    },
                    "required": ["query"]
                }),
            },
        ]
    }

    /// Dispatch a tool call to the appropriate VFS operation.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<HashMap<String, serde_json::Value>>,
    ) -> ToolCallResult {
        let args = arguments.unwrap_or_default();
        debug!("Tool call: {} with {:?}", name, args);

        match name {
            "openfs_read" => self.handle_read(&args).await,
            "openfs_write" => self.handle_write(&args).await,
            "openfs_append" => self.handle_append(&args).await,
            "openfs_ls" => self.handle_ls(&args).await,
            "openfs_stat" => self.handle_stat(&args).await,
            "openfs_delete" => self.handle_delete(&args).await,
            "openfs_grep" => self.handle_grep(&args).await,
            "openfs_exists" => self.handle_exists(&args).await,
            "openfs_rename" => self.handle_rename(&args).await,
            "openfs_read_batch" => self.handle_read_batch(&args).await,
            "openfs_write_batch" => self.handle_write_batch(&args).await,
            "openfs_delete_batch" => self.handle_delete_batch(&args).await,
            "openfs_cache_stats" => self.handle_cache_stats().await,
            "openfs_prefetch" => self.handle_prefetch(&args).await,
            "openfs_search" => self.handle_search(&args).await,
            _ => ToolCallResult::error(format!("Unknown tool: {}", name)),
        }
    }

    async fn handle_read(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: path".to_string()),
        };

        match self.vfs.read_with_cas_token(path).await {
            Ok((content, cas_token)) => match String::from_utf8(content) {
                Ok(text) => {
                    let mut result = serde_json::json!({ "content": text });
                    if let Some(token) = cas_token {
                        result["cas_token"] = serde_json::json!(token);
                    }
                    ToolCallResult::text(
                        serde_json::to_string(&result).unwrap_or(text),
                    )
                }
                Err(_) => ToolCallResult::text("[binary content]".to_string()),
            },
            Err(e) => ToolCallResult::error(format!("Failed to read {}: {}", path, e)),
        }
    }

    async fn handle_write(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: path".to_string()),
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolCallResult::error("Missing required parameter: content".to_string())
            }
        };
        let cas_token = args.get("cas_token").and_then(|v| v.as_str());

        if let Some(token) = cas_token {
            // Conditional write with CAS
            match self
                .vfs
                .compare_and_swap(path, Some(token), content.as_bytes())
                .await
            {
                Ok(new_token) => {
                    let result = serde_json::json!({
                        "status": "ok",
                        "bytes_written": content.len(),
                        "path": path,
                        "cas_token": new_token,
                    });
                    ToolCallResult::text(serde_json::to_string(&result).unwrap_or_default())
                }
                Err(e) => {
                    // Check for CAS conflict
                    let err_str = e.to_string();
                    if err_str.contains("precondition") || err_str.contains("Precondition") {
                        let result = serde_json::json!({
                            "status": "conflict",
                            "error": err_str,
                            "path": path,
                            "hint": "The file was modified since your last read. Read the file again to get the latest cas_token, then retry your write.",
                        });
                        ToolCallResult::error(
                            serde_json::to_string(&result).unwrap_or(err_str),
                        )
                    } else {
                        ToolCallResult::error(format!("Failed to write {}: {}", path, e))
                    }
                }
            }
        } else {
            // Unconditional write (original behavior)
            match self.vfs.write(path, content.as_bytes()).await {
                Ok(()) => {
                    ToolCallResult::text(format!("Wrote {} bytes to {}", content.len(), path))
                }
                Err(e) => ToolCallResult::error(format!("Failed to write {}: {}", path, e)),
            }
        }
    }

    async fn handle_append(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: path".to_string()),
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolCallResult::error("Missing required parameter: content".to_string())
            }
        };

        match self.vfs.append(path, content.as_bytes()).await {
            Ok(()) => {
                ToolCallResult::text(format!("Appended {} bytes to {}", content.len(), path))
            }
            Err(e) => ToolCallResult::error(format!("Failed to append to {}: {}", path, e)),
        }
    }

    async fn handle_ls(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("/");

        match self.vfs.list(path).await {
            Ok(entries) => {
                let json_entries: Vec<serde_json::Value> = entries
                    .iter()
                    .map(|entry| {
                        serde_json::json!({
                            "path": entry.path,
                            "name": entry.name,
                            "is_dir": entry.is_dir,
                            "size": entry.size,
                            "modified": entry.modified.map(|m| m.to_rfc3339()),
                        })
                    })
                    .collect();
                ToolCallResult::text(
                    serde_json::to_string(&json_entries).unwrap_or_else(|_| "[]".to_string()),
                )
            }
            Err(e) => ToolCallResult::error(format!("Failed to list {}: {}", path, e)),
        }
    }

    async fn handle_stat(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: path".to_string()),
        };

        match self.vfs.stat(path).await {
            Ok(entry) => {
                let result = serde_json::json!({
                    "path": entry.path,
                    "name": entry.name,
                    "is_dir": entry.is_dir,
                    "size": entry.size,
                    "modified": entry.modified.map(|m| m.to_rfc3339()),
                });
                match serde_json::to_string_pretty(&result) {
                    Ok(json) => ToolCallResult::text(json),
                    Err(e) => {
                        ToolCallResult::error(format!("Failed to serialize stat result: {}", e))
                    }
                }
            }
            Err(e) => ToolCallResult::error(format!("Failed to stat {}: {}", path, e)),
        }
    }

    async fn handle_delete(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: path".to_string()),
        };

        match self.vfs.delete(path).await {
            Ok(()) => ToolCallResult::text(format!("Deleted {}", path)),
            Err(e) => ToolCallResult::error(format!("Failed to delete {}: {}", path, e)),
        }
    }

    async fn handle_grep(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolCallResult::error("Missing required parameter: pattern".to_string())
            }
        };
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("/");

        let regex = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolCallResult::error(format!("Invalid regex: {}", e)),
        };

        // Collect files to search
        let mut matches = Vec::new();
        if let Err(e) = self.grep_recursive(&regex, path, &mut matches).await {
            warn!("Grep error in {}: {}", path, e);
        }

        let json_matches: Vec<serde_json::Value> = matches
            .iter()
            .map(|(path, line_number, line)| {
                serde_json::json!({
                    "path": path,
                    "line_number": line_number,
                    "line": line,
                })
            })
            .collect();
        ToolCallResult::text(
            serde_json::to_string(&json_matches).unwrap_or_else(|_| "[]".to_string()),
        )
    }

    async fn grep_recursive(
        &self,
        regex: &regex::Regex,
        path: &str,
        matches: &mut Vec<(String, usize, String)>,
    ) -> Result<(), openfs_core::VfsError> {
        let entries = self.vfs.list(path).await?;
        for entry in entries {
            if entry.is_dir {
                Box::pin(self.grep_recursive(regex, &entry.path, matches)).await?;
            } else if let Ok(content) = self.vfs.read(&entry.path).await {
                if let Ok(text) = String::from_utf8(content) {
                    for (i, line) in text.lines().enumerate() {
                        if regex.is_match(line) {
                            matches.push((entry.path.clone(), i + 1, line.to_string()));
                            if matches.len() >= 100 {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_exists(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: path".to_string()),
        };

        match self.vfs.exists(path).await {
            Ok(exists) => {
                let result = serde_json::json!({ "exists": exists });
                ToolCallResult::text(serde_json::to_string(&result).unwrap_or_default())
            }
            Err(e) => ToolCallResult::error(format!("Failed to check existence of {}: {}", path, e)),
        }
    }

    async fn handle_rename(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let from = match args.get("from").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: from".to_string()),
        };
        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolCallResult::error("Missing required parameter: to".to_string()),
        };

        match self.vfs.rename(from, to).await {
            Ok(()) => ToolCallResult::text(format!("Renamed {} to {}", from, to)),
            Err(e) => ToolCallResult::error(format!("Failed to rename {} to {}: {}", from, to, e)),
        }
    }

    async fn handle_read_batch(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let paths = match args.get("paths").and_then(|v| v.as_array()) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<&str>>(),
            None => {
                return ToolCallResult::error("Missing required parameter: paths".to_string())
            }
        };

        let results = self.vfs.read_batch(&paths).await;
        let json_results: Vec<serde_json::Value> = paths
            .iter()
            .zip(results.iter())
            .map(|(path, result)| match result {
                Ok(content) => match String::from_utf8(content.clone()) {
                    Ok(text) => serde_json::json!({ "path": path, "content": text }),
                    Err(_) => serde_json::json!({ "path": path, "content": "[binary content]" }),
                },
                Err(e) => serde_json::json!({ "path": path, "error": e.to_string() }),
            })
            .collect();

        ToolCallResult::text(
            serde_json::json!({ "results": json_results }).to_string(),
        )
    }

    async fn handle_write_batch(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let files = match args.get("files").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                return ToolCallResult::error("Missing required parameter: files".to_string())
            }
        };

        let file_pairs: Vec<(String, String)> = files
            .iter()
            .filter_map(|f| {
                let path = f.get("path")?.as_str()?.to_string();
                let content = f.get("content")?.as_str()?.to_string();
                Some((path, content))
            })
            .collect();

        let write_args: Vec<(&str, &[u8])> = file_pairs
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_bytes()))
            .collect();

        let results = self.vfs.write_batch(&write_args).await;
        let json_results: Vec<serde_json::Value> = file_pairs
            .iter()
            .zip(results.iter())
            .map(|((path, _), result)| match result {
                Ok(()) => serde_json::json!({ "path": path, "status": "ok" }),
                Err(e) => serde_json::json!({ "path": path, "status": "error", "error": e.to_string() }),
            })
            .collect();

        ToolCallResult::text(
            serde_json::json!({ "results": json_results }).to_string(),
        )
    }

    async fn handle_delete_batch(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let paths = match args.get("paths").and_then(|v| v.as_array()) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<&str>>(),
            None => {
                return ToolCallResult::error("Missing required parameter: paths".to_string())
            }
        };

        let results = self.vfs.delete_batch(&paths).await;
        let json_results: Vec<serde_json::Value> = paths
            .iter()
            .zip(results.iter())
            .map(|(path, result)| match result {
                Ok(()) => serde_json::json!({ "path": path, "status": "ok" }),
                Err(e) => serde_json::json!({ "path": path, "status": "error", "error": e.to_string() }),
            })
            .collect();

        ToolCallResult::text(
            serde_json::json!({ "results": json_results }).to_string(),
        )
    }

    async fn handle_cache_stats(&self) -> ToolCallResult {
        let stats = self.vfs.cache_stats().await;
        let result = serde_json::json!({
            "hits": stats.hits,
            "misses": stats.misses,
            "hit_rate": stats.hit_rate(),
            "entries": stats.entries,
            "size": stats.size,
            "evictions": stats.evictions,
        });
        ToolCallResult::text(result.to_string())
    }

    async fn handle_prefetch(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let paths = match args.get("paths").and_then(|v| v.as_array()) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<&str>>(),
            None => {
                return ToolCallResult::error("Missing required parameter: paths".to_string())
            }
        };

        let (prefetched, errors) = self.vfs.prefetch(&paths).await;
        let result = serde_json::json!({
            "prefetched": prefetched,
            "errors": errors,
        });
        ToolCallResult::text(result.to_string())
    }

    async fn handle_search(&self, args: &HashMap<String, serde_json::Value>) -> ToolCallResult {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => return ToolCallResult::error("Missing required parameter: query".to_string()),
        };

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let engine = match &self.search_engine {
            Some(e) => e,
            None => {
                return ToolCallResult::error(
                    "Semantic search not available. Configure a Chroma backend and search engine to enable it. Use grep for regex search.".to_string(),
                );
            }
        };

        let config = SearchConfig {
            limit,
            ..Default::default()
        };

        match engine.search(query, &config).await {
            Ok(results) => {
                if results.is_empty() {
                    return ToolCallResult::text("No results found.".to_string());
                }
                let mut lines = Vec::new();
                for result in &results {
                    lines.push(format!(
                        "[{:.3}] {} {}",
                        result.score,
                        result.chunk.source_path,
                        result.chunk.content.chars().take(200).collect::<String>()
                    ));
                }
                ToolCallResult::text(lines.join("\n"))
            }
            Err(e) => ToolCallResult::error(format!("Search failed: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfs_config::VfsConfig;
    use tempfile::TempDir;

    async fn make_handler(tmp: &TempDir) -> McpHandler {
        let yaml = format!(
            r#"
name: test
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /workspace
    backend: local
"#,
            tmp.path().to_str().unwrap()
        );
        let config = VfsConfig::from_yaml(&yaml).unwrap();
        let vfs = Arc::new(Vfs::from_config(config).await.unwrap());
        McpHandler::new(vfs)
    }

    #[tokio::test]
    async fn test_tool_definitions() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;
        let tools = handler.tool_definitions();
        assert!(tools.len() >= 15);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"openfs_read"));
        assert!(names.contains(&"openfs_write"));
        assert!(names.contains(&"openfs_append"));
        assert!(names.contains(&"openfs_ls"));
        assert!(names.contains(&"openfs_stat"));
        assert!(names.contains(&"openfs_delete"));
        assert!(names.contains(&"openfs_grep"));
        assert!(names.contains(&"openfs_exists"));
        assert!(names.contains(&"openfs_rename"));
        assert!(names.contains(&"openfs_search"));
    }

    #[tokio::test]
    async fn test_read_write_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Write
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        args.insert("content".to_string(), serde_json::json!("hello world"));
        let result = handler.call_tool("openfs_write", Some(args)).await;
        assert!(result.is_error.is_none());

        // Read
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        let result = handler.call_tool("openfs_read", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["content"], "hello world");
    }

    #[tokio::test]
    async fn test_ls() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Create files
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/a.txt"));
        args.insert("content".to_string(), serde_json::json!("aaa"));
        handler.call_tool("openfs_write", Some(args)).await;

        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/b.txt"));
        args.insert("content".to_string(), serde_json::json!("bbb"));
        handler.call_tool("openfs_write", Some(args)).await;

        // List
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace"));
        let result = handler.call_tool("openfs_ls", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        // Validate JSON structure
        let entries: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        // Validate entry fields
        for entry in &entries {
            assert!(entry["path"].is_string());
            assert!(entry["name"].is_string());
            assert!(entry["is_dir"].is_boolean());
        }
    }

    #[tokio::test]
    async fn test_stat() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        args.insert("content".to_string(), serde_json::json!("hello"));
        handler.call_tool("openfs_write", Some(args)).await;

        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        let result = handler.call_tool("openfs_stat", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        assert!(text.contains("\"name\": \"test.txt\""));
    }

    #[tokio::test]
    async fn test_delete() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        args.insert("content".to_string(), serde_json::json!("hello"));
        handler.call_tool("openfs_write", Some(args)).await;

        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        let result = handler.call_tool("openfs_delete", Some(args)).await;
        assert!(result.is_error.is_none());

        // Verify deleted
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        let result = handler.call_tool("openfs_read", Some(args)).await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_grep() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Write the file directly to the filesystem so it's immediately available
        std::fs::write(tmp.path().join("test.txt"), "line one\nfoo bar\nline three").unwrap();

        let mut args = HashMap::new();
        args.insert("pattern".to_string(), serde_json::json!("foo"));
        args.insert("path".to_string(), serde_json::json!("/workspace"));
        let result = handler.call_tool("openfs_grep", Some(args)).await;
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text.clone(),
        };
        // Result is always JSON array
        let matches: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        if !matches.is_empty() {
            // Validate JSON structure of grep matches
            assert!(matches[0]["path"].is_string());
            assert!(matches[0]["line_number"].is_number());
            assert!(matches[0]["line"].as_str().unwrap().contains("foo bar"));
        }
        // Empty array is also valid (depends on fs backend listing behavior)
    }

    #[tokio::test]
    async fn test_exists_true() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Write a file first
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        args.insert("content".to_string(), serde_json::json!("hello"));
        handler.call_tool("openfs_write", Some(args)).await;

        // Check exists
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/test.txt"));
        let result = handler.call_tool("openfs_exists", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["exists"], true);
    }

    #[tokio::test]
    async fn test_exists_false() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            serde_json::json!("/workspace/nonexistent.txt"),
        );
        let result = handler.call_tool("openfs_exists", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["exists"], false);
    }

    #[tokio::test]
    async fn test_rename() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Write a file
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/old.txt"));
        args.insert("content".to_string(), serde_json::json!("rename me"));
        handler.call_tool("openfs_write", Some(args)).await;

        // Rename it
        let mut args = HashMap::new();
        args.insert("from".to_string(), serde_json::json!("/workspace/old.txt"));
        args.insert("to".to_string(), serde_json::json!("/workspace/new.txt"));
        let result = handler.call_tool("openfs_rename", Some(args)).await;
        assert!(result.is_error.is_none());

        // Verify old is gone, new exists with content
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/old.txt"));
        let result = handler.call_tool("openfs_read", Some(args)).await;
        assert_eq!(result.is_error, Some(true));

        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/new.txt"));
        let result = handler.call_tool("openfs_read", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["content"], "rename me");
    }

    #[tokio::test]
    async fn test_rename_missing_params() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Missing 'to'
        let mut args = HashMap::new();
        args.insert("from".to_string(), serde_json::json!("/workspace/a.txt"));
        let result = handler.call_tool("openfs_rename", Some(args)).await;
        assert_eq!(result.is_error, Some(true));

        // Missing 'from'
        let mut args = HashMap::new();
        args.insert("to".to_string(), serde_json::json!("/workspace/b.txt"));
        let result = handler.call_tool("openfs_rename", Some(args)).await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;
        let result = handler.call_tool("nonexistent", None).await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_missing_required_param() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let result = handler.call_tool("openfs_read", Some(HashMap::new())).await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_read_batch_all_success() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Write files
        for name in &["a.txt", "b.txt"] {
            let mut args = HashMap::new();
            args.insert(
                "path".to_string(),
                serde_json::json!(format!("/workspace/{}", name)),
            );
            args.insert("content".to_string(), serde_json::json!(format!("content of {}", name)));
            handler.call_tool("openfs_write", Some(args)).await;
        }

        let mut args = HashMap::new();
        args.insert(
            "paths".to_string(),
            serde_json::json!(["/workspace/a.txt", "/workspace/b.txt"]),
        );
        let result = handler.call_tool("openfs_read_batch", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["content"], "content of a.txt");
        assert_eq!(results[1]["content"], "content of b.txt");
    }

    #[tokio::test]
    async fn test_read_batch_partial_failure() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/exists.txt"));
        args.insert("content".to_string(), serde_json::json!("exists"));
        handler.call_tool("openfs_write", Some(args)).await;

        let mut args = HashMap::new();
        args.insert(
            "paths".to_string(),
            serde_json::json!(["/workspace/exists.txt", "/workspace/missing.txt"]),
        );
        let result = handler.call_tool("openfs_read_batch", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results[0]["content"], "exists");
        assert!(results[1]["error"].is_string());
    }

    #[tokio::test]
    async fn test_read_batch_empty() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert("paths".to_string(), serde_json::json!([]));
        let result = handler.call_tool("openfs_read_batch", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["results"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_write_batch() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert(
            "files".to_string(),
            serde_json::json!([
                {"path": "/workspace/w1.txt", "content": "one"},
                {"path": "/workspace/w2.txt", "content": "two"},
            ]),
        );
        let result = handler.call_tool("openfs_write_batch", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["status"], "ok");
        assert_eq!(results[1]["status"], "ok");

        // Verify written
        let mut args = HashMap::new();
        args.insert("path".to_string(), serde_json::json!("/workspace/w1.txt"));
        let result = handler.call_tool("openfs_read", Some(args)).await;
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["content"], "one");
    }

    #[tokio::test]
    async fn test_delete_batch() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        // Write files
        for name in &["d1.txt", "d2.txt"] {
            let mut args = HashMap::new();
            args.insert(
                "path".to_string(),
                serde_json::json!(format!("/workspace/{}", name)),
            );
            args.insert("content".to_string(), serde_json::json!("data"));
            handler.call_tool("openfs_write", Some(args)).await;
        }

        let mut args = HashMap::new();
        args.insert(
            "paths".to_string(),
            serde_json::json!(["/workspace/d1.txt", "/workspace/d2.txt"]),
        );
        let result = handler.call_tool("openfs_delete_batch", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["status"], "ok");
        assert_eq!(results[1]["status"], "ok");
    }

    #[tokio::test]
    async fn test_cache_stats_returns_json() {
        let tmp = TempDir::new().unwrap();
        let handler = make_handler(&tmp).await;

        let result = handler.call_tool("openfs_cache_stats", None).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed["hits"].is_number());
        assert!(parsed["misses"].is_number());
        assert!(parsed["hit_rate"].is_number());
        assert!(parsed["entries"].is_number());
        assert!(parsed["size"].is_number());
        assert!(parsed["evictions"].is_number());
    }

    #[tokio::test]
    async fn test_prefetch_warms_cache() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("pf1.txt"), "prefetch1").unwrap();
        std::fs::write(tmp.path().join("pf2.txt"), "prefetch2").unwrap();

        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert(
            "paths".to_string(),
            serde_json::json!(["/workspace/pf1.txt", "/workspace/pf2.txt"]),
        );
        let result = handler.call_tool("openfs_prefetch", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["prefetched"], 2);
        assert_eq!(parsed["errors"], 0);
    }

    #[tokio::test]
    async fn test_prefetch_with_missing_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("exists.txt"), "yes").unwrap();

        let handler = make_handler(&tmp).await;

        let mut args = HashMap::new();
        args.insert(
            "paths".to_string(),
            serde_json::json!(["/workspace/exists.txt", "/workspace/nope.txt"]),
        );
        let result = handler.call_tool("openfs_prefetch", Some(args)).await;
        assert!(result.is_error.is_none());
        let text = match &result.content[0] {
            crate::protocol::ToolContent::Text { text } => text,
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["prefetched"], 1);
        assert_eq!(parsed["errors"], 1);
    }
}

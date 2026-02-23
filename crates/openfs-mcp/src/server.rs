//! MCP server — reads JSON-RPC from stdin, writes to stdout.

#[cfg(test)]
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info};

use crate::handler::McpHandler;
use crate::protocol::*;

/// MCP server that communicates over stdio.
pub struct McpServer {
    handler: McpHandler,
}

impl McpServer {
    pub fn new(handler: McpHandler) -> Self {
        McpServer { handler }
    }

    /// Run the server, reading JSON-RPC messages from stdin and writing responses to stdout.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        info!("OpenFS MCP server started (stdio transport)");

        while let Some(line) = lines.next_line().await? {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            debug!("Received: {}", line);

            let response = self.handle_message(&line).await;

            if let Some(resp) = response {
                let json = serde_json::to_string(&resp)?;
                debug!("Sending: {}", json);
                stdout.write_all(json.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
            }
        }

        info!("OpenFS MCP server shutting down");
        Ok(())
    }

    /// Process a single JSON-RPC message and return an optional response.
    /// Returns None for notifications (no id).
    pub async fn handle_message(&self, line: &str) -> Option<JsonRpcResponse> {
        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(e) => {
                error!("Failed to parse JSON-RPC: {}", e);
                return Some(JsonRpcResponse::error(
                    None,
                    PARSE_ERROR,
                    format!("Parse error: {}", e),
                ));
            }
        };

        // Notifications (no id) don't get responses
        if request.id.is_none() {
            debug!("Notification: {}", request.method);
            return None;
        }

        let id = request.id.clone();

        match request.method.as_str() {
            "initialize" => {
                let result = InitializeResult {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability {
                            list_changed: Some(false),
                        }),
                    },
                    server_info: ServerInfo {
                        name: "openfs-mcp".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                };
                match serde_json::to_value(result) {
                    Ok(v) => Some(JsonRpcResponse::success(id, v)),
                    Err(e) => Some(JsonRpcResponse::error(
                        id,
                        INTERNAL_ERROR,
                        format!("Serialization error: {}", e),
                    )),
                }
            }
            "tools/list" => {
                let tools = self.handler.tool_definitions();
                let result = ToolListResult { tools };
                match serde_json::to_value(result) {
                    Ok(v) => Some(JsonRpcResponse::success(id, v)),
                    Err(e) => Some(JsonRpcResponse::error(
                        id,
                        INTERNAL_ERROR,
                        format!("Serialization error: {}", e),
                    )),
                }
            }
            "tools/call" => {
                let params: ToolCallParams = match request.params {
                    Some(p) => match serde_json::from_value(p) {
                        Ok(params) => params,
                        Err(e) => {
                            return Some(JsonRpcResponse::error(
                                id,
                                INVALID_PARAMS,
                                format!("Invalid params: {}", e),
                            ))
                        }
                    },
                    None => {
                        return Some(JsonRpcResponse::error(
                            id,
                            INVALID_PARAMS,
                            "Missing params".to_string(),
                        ))
                    }
                };

                let tool_timeout = std::time::Duration::from_secs(30);
                let result = match tokio::time::timeout(
                    tool_timeout,
                    self.handler.call_tool(&params.name, params.arguments),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        return Some(JsonRpcResponse::error(
                            id,
                            INTERNAL_ERROR,
                            format!(
                                "Tool '{}' timed out after {}s",
                                params.name,
                                tool_timeout.as_secs()
                            ),
                        ));
                    }
                };
                match serde_json::to_value(result) {
                    Ok(v) => Some(JsonRpcResponse::success(id, v)),
                    Err(e) => Some(JsonRpcResponse::error(
                        id,
                        INTERNAL_ERROR,
                        format!("Serialization error: {}", e),
                    )),
                }
            }
            "ping" => Some(JsonRpcResponse::success(id, serde_json::json!({}))),
            _ => Some(JsonRpcResponse::error(
                id,
                METHOD_NOT_FOUND,
                format!("Unknown method: {}", request.method),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfs_config::VfsConfig;
    use openfs_remote::Vfs;
    use tempfile::TempDir;

    async fn make_server(tmp: &TempDir) -> McpServer {
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
        let handler = McpHandler::new(vfs);
        McpServer::new(handler)
    }

    #[tokio::test]
    async fn test_initialize() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}"#;
        let resp = server.handle_message(msg).await.unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn test_tools_list() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let resp = server.handle_message(msg).await.unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(tools.len() >= 7);
    }

    #[tokio::test]
    async fn test_tools_call_read() {
        let tmp = TempDir::new().unwrap();
        // Write a file first
        std::fs::write(tmp.path().join("hello.txt"), "hello from file").unwrap();

        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"openfs_read","arguments":{"path":"/workspace/hello.txt"}}}"#;
        let resp = server.handle_message(msg).await.unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        let read_text = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(read_text).unwrap();
        assert_eq!(parsed["content"], "hello from file");
    }

    #[tokio::test]
    async fn test_tools_call_write_and_read() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        // Write
        let msg = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"openfs_write","arguments":{"path":"/workspace/new.txt","content":"written via mcp"}}}"#;
        let resp = server.handle_message(msg).await.unwrap();
        assert!(resp.error.is_none());

        // Read back
        let msg = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"openfs_read","arguments":{"path":"/workspace/new.txt"}}}"#;
        let resp = server.handle_message(msg).await.unwrap();
        let result = resp.result.unwrap();
        let read_text = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(read_text).unwrap();
        assert_eq!(parsed["content"], "written via mcp");
    }

    #[tokio::test]
    async fn test_notification_no_response() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let resp = server.handle_message(msg).await;
        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","id":6,"method":"unknown/method"}"#;
        let resp = server.handle_message(msg).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn test_parse_error() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        let resp = server.handle_message("not json").await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, PARSE_ERROR);
    }

    #[tokio::test]
    async fn test_ping() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#;
        let resp = server.handle_message(msg).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_tools_call_ls() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("file1.txt"), "content").unwrap();
        std::fs::write(tmp.path().join("file2.txt"), "content").unwrap();

        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"openfs_ls","arguments":{"path":"/workspace"}}}"#;
        let resp = server.handle_message(msg).await.unwrap();
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("file1.txt"));
        assert!(text.contains("file2.txt"));
    }

    #[tokio::test]
    async fn test_missing_params() {
        let tmp = TempDir::new().unwrap();
        let server = make_server(&tmp).await;

        let msg = r#"{"jsonrpc":"2.0","id":9,"method":"tools/call"}"#;
        let resp = server.handle_message(msg).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }
}

//! MCP (Model Context Protocol) JSON-RPC types.
//!
//! Implements the subset of MCP needed for tool serving over stdio transport.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JSON-RPC request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<serde_json::Value>, code: i32, message: String) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

/// JSON-RPC error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// MCP error codes.
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

/// MCP server capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// MCP server info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// MCP initialize result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

/// MCP tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// MCP tool list result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolListResult {
    pub tools: Vec<McpToolDef>,
}

/// MCP tool call params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<HashMap<String, serde_json::Value>>,
}

/// Content types returned from tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
}

/// MCP tool call result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl ToolCallResult {
    pub fn text(text: String) -> Self {
        ToolCallResult {
            content: vec![ToolContent::Text { text }],
            is_error: None,
        }
    }

    pub fn error(message: String) -> Self {
        ToolCallResult {
            content: vec![ToolContent::Text { text: message }],
            is_error: Some(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_rpc_request_deserialize() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn test_json_rpc_response_success() {
        let resp =
            JsonRpcResponse::success(Some(serde_json::json!(1)), serde_json::json!({"ok": true}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_json_rpc_response_error() {
        let resp = JsonRpcResponse::error(
            Some(serde_json::json!(1)),
            METHOD_NOT_FOUND,
            "not found".to_string(),
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"result\""));
        assert!(json.contains("\"error\""));
        assert!(json.contains("-32601"));
    }

    #[test]
    fn test_tool_call_result_text() {
        let result = ToolCallResult::text("hello".to_string());
        assert_eq!(result.content.len(), 1);
        assert!(result.is_error.is_none());
    }

    #[test]
    fn test_tool_call_result_error() {
        let result = ToolCallResult::error("something failed".to_string());
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn test_initialize_result_serialize() {
        let result = InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
            },
            server_info: ServerInfo {
                name: "openfs".to_string(),
                version: "0.3.0".to_string(),
            },
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("protocolVersion"));
        assert!(json.contains("serverInfo"));
    }

    #[test]
    fn test_tool_call_params_deserialize() {
        let json = r#"{"name":"openfs_read","arguments":{"path":"/workspace/test.txt"}}"#;
        let params: ToolCallParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "openfs_read");
        let args = params.arguments.unwrap();
        assert_eq!(args.get("path").unwrap(), "/workspace/test.txt");
    }

    #[test]
    fn test_mcp_tool_def_serialize() {
        let tool = McpToolDef {
            name: "openfs_read".to_string(),
            description: "Read a file".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path"}
                },
                "required": ["path"]
            }),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("inputSchema"));
        assert!(json.contains("openfs_read"));
    }

    #[test]
    fn test_request_without_params() {
        let json = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert!(req.params.is_none());
    }

    #[test]
    fn test_request_with_null_id() {
        let json = r#"{"jsonrpc":"2.0","id":null,"method":"notifications/initialized"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        // serde deserializes JSON null for Option<Value> as None
        // This is correct - null id is treated same as missing id
        assert!(req.id.is_none() || req.id == Some(serde_json::Value::Null));
    }

    #[test]
    fn test_request_without_id_is_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert!(req.id.is_none());
    }
}

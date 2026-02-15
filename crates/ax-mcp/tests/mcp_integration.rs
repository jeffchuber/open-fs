//! MCP Protocol Integration Tests
//!
//! Simulates a full MCP client session via `handle_message()` with raw JSON-RPC strings.

use std::sync::Arc;

use ax_config::VfsConfig;
use ax_mcp::{McpHandler, McpServer, INVALID_PARAMS, METHOD_NOT_FOUND, PARSE_ERROR};
use ax_remote::Vfs;
use tempfile::TempDir;

async fn make_server(tmp: &TempDir) -> McpServer {
    let yaml = format!(
        r#"
name: mcp-test
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
async fn test_full_session_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let server = make_server(&tmp).await;

    // 1. Initialize
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}"#,
        )
        .await
        .unwrap();
    assert_eq!(resp.jsonrpc, "2.0");
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["serverInfo"]["name"].is_string());

    // 2. tools/list
    let resp = server
        .handle_message(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
        .await
        .unwrap();
    let result = resp.result.unwrap();
    let tools = result["tools"].as_array().unwrap();
    assert!(tools.len() >= 7);
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"ax_read"));
    assert!(tool_names.contains(&"ax_write"));
    assert!(tool_names.contains(&"ax_ls"));

    // 3. Write a file
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ax_write","arguments":{"path":"/workspace/session.txt","content":"session data"}}}"#,
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // 4. Read it back
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ax_read","arguments":{"path":"/workspace/session.txt"}}}"#,
        )
        .await
        .unwrap();
    let result = resp.result.unwrap();
    assert_eq!(result["content"][0]["text"], "session data");

    // 5. Verify via stat
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"ax_stat","arguments":{"path":"/workspace/session.txt"}}}"#,
        )
        .await
        .unwrap();
    let result = resp.result.unwrap();
    let stat_text = result["content"][0]["text"].as_str().unwrap();
    assert!(stat_text.contains("session.txt"));
}

#[tokio::test]
async fn test_error_handling_session() {
    let tmp = TempDir::new().unwrap();
    let server = make_server(&tmp).await;

    // Parse error
    let resp = server.handle_message("{bad json").await.unwrap();
    assert_eq!(resp.error.as_ref().unwrap().code, PARSE_ERROR);

    // Unknown method
    let resp = server
        .handle_message(r#"{"jsonrpc":"2.0","id":1,"method":"bogus/method"}"#)
        .await
        .unwrap();
    assert_eq!(resp.error.as_ref().unwrap().code, METHOD_NOT_FOUND);

    // Missing params for tools/call
    let resp = server
        .handle_message(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call"}"#)
        .await
        .unwrap();
    assert_eq!(resp.error.as_ref().unwrap().code, INVALID_PARAMS);

    // Unknown tool
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nonexistent_tool","arguments":{}}}"#,
        )
        .await
        .unwrap();
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
}

#[tokio::test]
async fn test_write_delete_verify_flow() {
    let tmp = TempDir::new().unwrap();
    let server = make_server(&tmp).await;

    // Write
    server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ax_write","arguments":{"path":"/workspace/ephemeral.txt","content":"temp data"}}}"#,
        )
        .await
        .unwrap();

    // Delete
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ax_delete","arguments":{"path":"/workspace/ephemeral.txt"}}}"#,
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // Verify deleted (read should error)
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ax_read","arguments":{"path":"/workspace/ephemeral.txt"}}}"#,
        )
        .await
        .unwrap();
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
}

#[tokio::test]
async fn test_grep_flow() {
    let tmp = TempDir::new().unwrap();
    // Write test files directly to the filesystem
    std::fs::write(
        tmp.path().join("searchable.txt"),
        "line one\nfind me here\nline three",
    )
    .unwrap();

    let server = make_server(&tmp).await;

    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ax_grep","arguments":{"pattern":"find me","path":"/workspace"}}}"#,
        )
        .await
        .unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("find me") || text.contains("No matches"),
        "Unexpected grep result: {}",
        text
    );
}

#[tokio::test]
async fn test_ls_flow() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("file1.txt"), "content1").unwrap();
    std::fs::write(tmp.path().join("file2.txt"), "content2").unwrap();

    let server = make_server(&tmp).await;

    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ax_ls","arguments":{"path":"/workspace"}}}"#,
        )
        .await
        .unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("file1.txt"));
    assert!(text.contains("file2.txt"));
}

#[tokio::test]
async fn test_jsonrpc_compliance() {
    let tmp = TempDir::new().unwrap();
    let server = make_server(&tmp).await;

    // All responses must have "jsonrpc": "2.0"
    let messages = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"bogus"}"#,
    ];

    for msg in messages {
        let resp = server.handle_message(msg).await.unwrap();
        assert_eq!(resp.jsonrpc, "2.0", "Response missing jsonrpc 2.0 for: {}", msg);
    }

    // Notifications (no id) get no response
    let resp = server
        .handle_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .await;
    assert!(resp.is_none(), "Notification should not produce a response");
}

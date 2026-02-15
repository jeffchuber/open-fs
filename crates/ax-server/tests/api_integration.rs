//! Server API Integration Tests
//!
//! Spins up a real Axum server on an ephemeral port and tests with reqwest over HTTP.
//! Exercises the full middleware stack (tracing, timeout, body limit, concurrency limit).

use ax_config::{Secret, VfsConfig};
use ax_remote::Vfs;
use ax_server::{build_router, AppState};
use tempfile::TempDir;

/// Start an Axum server on an ephemeral port and return the base URL.
async fn start_server(tmp: &TempDir, api_key: Option<&str>) -> String {
    let yaml = format!(
        r#"
name: test
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /
    backend: local
"#,
        tmp.path().to_str().unwrap()
    );
    let config = VfsConfig::from_yaml(&yaml).unwrap();
    let vfs = Vfs::from_config(config).await.unwrap();
    let state = AppState::new(vfs, api_key.map(|k| Secret::new(k)));
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://127.0.0.1:{}", port)
}

#[tokio::test]
async fn test_health_endpoint() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("{}/health", base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
}

#[tokio::test]
async fn test_health_live_and_ready() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    // Liveness
    let resp = client
        .get(format!("{}/health/live", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Readiness
    let resp = client
        .get(format!("{}/health/ready", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_write_read_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    // Write
    let resp = client
        .post(format!("{}/write", base))
        .json(&serde_json::json!({
            "path": "/integration-test.txt",
            "content": "hello from integration test"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Read
    let resp = client
        .get(format!("{}/read?path=%2Fintegration-test.txt", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["content"], "hello from integration test");
    assert_eq!(json["size"], 27);
}

#[tokio::test]
async fn test_versioned_v1_routes() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    // Write via /v1/write
    let resp = client
        .post(format!("{}/v1/write", base))
        .json(&serde_json::json!({
            "path": "/v1-test.txt",
            "content": "versioned"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Read via /v1/read
    let resp = client
        .get(format!("{}/v1/read?path=%2Fv1-test.txt", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["content"], "versioned");
}

#[tokio::test]
async fn test_delete_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    // Write
    client
        .post(format!("{}/write", base))
        .json(&serde_json::json!({
            "path": "/to-delete.txt",
            "content": "temporary"
        }))
        .send()
        .await
        .unwrap();

    // Delete
    let resp = client
        .delete(format!("{}/delete?path=%2Fto-delete.txt", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["deleted"], true);

    // Verify gone
    let resp = client
        .get(format!("{}/read?path=%2Fto-delete.txt", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_ls_endpoint() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    // Write files
    for name in &["a.txt", "b.txt"] {
        client
            .post(format!("{}/write", base))
            .json(&serde_json::json!({
                "path": format!("/{}", name),
                "content": "data"
            }))
            .send()
            .await
            .unwrap();
    }

    let resp = client
        .get(format!("{}/ls?path=%2F", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json.is_array());
    assert!(json.as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn test_stat_endpoint() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    client
        .post(format!("{}/write", base))
        .json(&serde_json::json!({
            "path": "/stat-test.txt",
            "content": "twelve chars"
        }))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(format!("{}/stat?path=%2Fstat-test.txt", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["is_dir"], false);
    assert_eq!(json["size"], 12);
}

#[tokio::test]
async fn test_auth_required_401() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, Some("test-secret-key")).await;
    let client = reqwest::Client::new();

    // No auth header → 401
    let resp = client
        .get(format!("{}/status", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn test_auth_bypass_on_health() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, Some("test-secret-key")).await;
    let client = reqwest::Client::new();

    // Health should work without auth
    let resp = client.get(format!("{}/health", base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_404_on_nonexistent_file() {
    let tmp = TempDir::new().unwrap();
    let base = start_server(&tmp, None).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/read?path=%2Fno-such-file.txt", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

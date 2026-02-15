//! Route definitions for the AX REST API.

use std::time::Duration;

use axum::routing::{delete, get, post};
use axum::Router;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::handlers;
use crate::state::AppState;

/// Default request timeout (60 seconds).
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Default maximum request body size (50 MB).
const DEFAULT_BODY_LIMIT: usize = 50 * 1024 * 1024;
/// Default max concurrent requests.
const DEFAULT_CONCURRENCY_LIMIT: usize = 256;

/// Build the Axum router with all AX API routes.
///
/// Routes are available both at `/` (legacy) and `/v1/` (versioned).
/// Health endpoints are always at the root level.
pub fn build_router(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/status", get(handlers::status))
        .route("/read", get(handlers::read))
        .route("/write", post(handlers::write))
        .route("/delete", delete(handlers::delete))
        .route("/stat", get(handlers::stat))
        .route("/ls", get(handlers::ls))
        .route("/search", post(handlers::search))
        .route("/grep", get(handlers::grep))
        .route("/append", post(handlers::append))
        .route("/exists", get(handlers::exists))
        .route("/rename", post(handlers::rename))
        .route("/copy", post(handlers::copy))
        .route("/openapi", get(handlers::openapi));

    Router::new()
        .route("/health", get(handlers::health))
        .route("/health/live", get(handlers::health_live))
        .route("/health/ready", get(handlers::health_ready))
        .merge(api_routes.clone())
        .nest("/v1", api_routes)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(DEFAULT_REQUEST_TIMEOUT))
        .layer(RequestBodyLimitLayer::new(DEFAULT_BODY_LIMIT))
        .layer(ConcurrencyLimitLayer::new(DEFAULT_CONCURRENCY_LIMIT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ax_config::VfsConfig;
    use ax_remote::Vfs;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn make_config(tmp: &TempDir) -> VfsConfig {
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
        VfsConfig::from_yaml(&yaml).unwrap()
    }

    async fn make_app_with_tmp(tmp: &TempDir) -> Router {
        let config = make_config(tmp);
        let vfs = Vfs::from_config(config).await.unwrap();
        let state = AppState::new(vfs, None);
        build_router(state)
    }

    async fn make_app_with_key_and_tmp(tmp: &TempDir, key: &str) -> Router {
        let config = make_config(tmp);
        let vfs = Vfs::from_config(config).await.unwrap();
        let state = AppState::new(vfs, Some(ax_config::Secret::new(key)));
        build_router(state)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["uptime_secs"].is_number());
        assert!(json["mounts"].is_array());
    }

    #[tokio::test]
    async fn test_auth_required() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_key_and_tmp(&tmp, "secret").await;
        // No auth header -> 401
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_with_valid_key() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_key_and_tmp(&tmp, "secret").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        // Write
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/test-server.txt","content":"hello from server"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Read
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/read?path=%2Ftest-server.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "hello from server");
        assert_eq!(json["size"], 17);
    }

    #[tokio::test]
    async fn test_read_not_found() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/read?path=%2Fnonexistent-file-xyz.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_ls_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/ls?path=%2F")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.is_array());
    }

    #[tokio::test]
    async fn test_stat_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        // Write a file first
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/stat-test.txt","content":"data"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Stat it
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/stat?path=%2Fstat-test.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["is_dir"], false);
    }

    #[tokio::test]
    async fn test_delete_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        // Write a file
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/delete-test.txt","content":"temp"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Delete it
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/delete?path=%2Fdelete-test.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["deleted"], true);
    }

    #[tokio::test]
    async fn test_openapi_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/openapi")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["openapi"], "3.0.3");
        assert!(json["info"]["title"].as_str().unwrap().contains("AX"));
        assert!(json["paths"]["/search"].is_object());
        assert!(json["paths"]["/health"].is_object());
        assert!(json["paths"]["/grep"].is_object());
        assert!(json["components"]["schemas"]["SearchRequest"].is_object());
    }

    #[tokio::test]
    async fn test_search_no_engine() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Should return 503 since no search engine is configured
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_health_no_auth_needed() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_key_and_tmp(&tmp, "secret").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Health should work without auth
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_append_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        // Write initial content
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path":"/append-test.txt","content":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Append
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/append")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/append-test.txt","content":" world"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["bytes_appended"], 6);

        // Read and verify
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/read?path=%2Fappend-test.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "hello world");
    }

    #[tokio::test]
    async fn test_exists_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        // Check non-existent
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/exists?path=%2Fnope.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["exists"], false);

        // Write, then check
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/exists-test.txt","content":"data"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/exists?path=%2Fexists-test.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["exists"], true);
    }

    #[tokio::test]
    async fn test_rename_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        // Write file
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/rename-src.txt","content":"data"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Rename
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rename")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"from":"/rename-src.txt","to":"/rename-dst.txt"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["renamed"], true);

        // Verify old gone, new exists
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/exists?path=%2Frename-src.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["exists"], false);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/read?path=%2Frename-dst.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_copy_endpoint() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        // Write source
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/copy-src.txt","content":"copy me"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Copy
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/copy")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"src":"/copy-src.txt","dst":"/copy-dst.txt"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["bytes_copied"], 7);

        // Both should exist with same content
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/read?path=%2Fcopy-src.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "copy me");

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/read?path=%2Fcopy-dst.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "copy me");
    }

    #[tokio::test]
    async fn test_copy_not_found() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/copy")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"src":"/nonexistent.txt","dst":"/dst.txt"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_new_endpoints_in_openapi() {
        let tmp = TempDir::new().unwrap();
        let app = make_app_with_tmp(&tmp).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/openapi")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["paths"]["/append"].is_object());
        assert!(json["paths"]["/exists"].is_object());
        assert!(json["paths"]["/rename"].is_object());
        assert!(json["paths"]["/copy"].is_object());
    }
}

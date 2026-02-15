//! Request handlers for the REST API.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

// --- Auth helper ---

fn extract_api_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if !state.check_auth(extract_api_key(headers)) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Unauthorized".to_string(),
                detail: Some("Invalid or missing API key".to_string()),
            }),
        ));
    }
    Ok(())
}

// --- Response types ---

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub detail: Option<String>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub uptime_secs: u64,
    pub mounts: Vec<MountInfo>,
}

#[derive(Serialize)]
pub struct MountInfo {
    pub path: String,
    pub backend: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentEncoding {
    Utf8,
    Base64,
}

#[derive(Serialize)]
pub struct ReadResponse {
    pub path: String,
    pub content: String,
    pub size: usize,
    pub encoding: ContentEncoding,
}

#[derive(Serialize)]
pub struct WriteResponse {
    pub path: String,
    pub bytes_written: usize,
}

#[derive(Serialize)]
pub struct DeleteResponse {
    pub path: String,
    pub deleted: bool,
}

#[derive(Serialize)]
pub struct StatResponse {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<String>,
}

#[derive(Serialize)]
pub struct LsEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Serialize)]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

// --- Search response ---

#[derive(Serialize)]
pub struct SearchHit {
    pub path: String,
    pub content: String,
    pub score: f32,
    pub dense_score: Option<f32>,
    pub sparse_score: Option<f32>,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub hits: Vec<SearchHit>,
}

// --- Request types ---

#[derive(Deserialize)]
pub struct PathQuery {
    pub path: String,
}

#[derive(Deserialize)]
pub struct ReadQuery {
    pub path: String,
    #[serde(default)]
    pub encoding: Option<ContentEncoding>,
}

#[derive(Deserialize)]
pub struct WriteRequest {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub encoding: Option<ContentEncoding>,
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    #[serde(default)]
    pub min_score: Option<f32>,
}

fn default_search_limit() -> usize {
    10
}

#[derive(Deserialize)]
pub struct GrepQuery {
    pub pattern: String,
    #[serde(default = "default_grep_path")]
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

fn default_grep_path() -> String {
    "/".to_string()
}

// --- Append types ---

#[derive(Deserialize)]
pub struct AppendRequest {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub encoding: Option<ContentEncoding>,
}

#[derive(Serialize)]
pub struct AppendResponse {
    pub path: String,
    pub bytes_appended: usize,
}

// --- Exists types ---

#[derive(Serialize)]
pub struct ExistsResponse {
    pub path: String,
    pub exists: bool,
}

// --- Rename types ---

#[derive(Deserialize)]
pub struct RenameRequest {
    pub from: String,
    pub to: String,
}

#[derive(Serialize)]
pub struct RenameResponse {
    pub from: String,
    pub to: String,
    pub renamed: bool,
}

// --- Copy types ---

#[derive(Deserialize)]
pub struct CopyRequest {
    pub src: String,
    pub dst: String,
}

#[derive(Serialize)]
pub struct CopyResponse {
    pub src: String,
    pub dst: String,
    pub bytes_copied: usize,
}

// --- Handlers ---

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Lightweight liveness check — confirms the process is running.
pub async fn health_live() -> StatusCode {
    StatusCode::OK
}

/// Readiness check — confirms backends are accessible and the VFS is operational.
pub async fn health_ready(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    // Verify VFS can list root (proves at least one backend is working)
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state.vfs().list("/"),
    ).await {
        Ok(Ok(_)) => Ok(Json(serde_json::json!({
            "status": "ready",
            "search_available": state.search_engine().is_some(),
        }))),
        Ok(Err(e)) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not ready".to_string(),
                detail: Some(format!("VFS check failed: {}", e)),
            }),
        )),
        Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not ready".to_string(),
                detail: Some("Health check timed out after 5s".to_string()),
            }),
        )),
    }
}

pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let config = state.vfs().effective_config();
    let mounts: Vec<MountInfo> = config
        .mounts
        .iter()
        .map(|m| MountInfo {
            path: m.path.clone(),
            backend: m.backend.clone().unwrap_or_default(),
        })
        .collect();

    Ok(Json(StatusResponse {
        uptime_secs: state.uptime_secs(),
        mounts,
    }))
}

pub async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ReadQuery>,
) -> Result<Json<ReadResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    match state.vfs().read(&params.path).await {
        Ok(content) => {
            let size = content.len();
            let (text, encoding) = match (params.encoding, content) {
                (Some(ContentEncoding::Base64), bytes) => (BASE64.encode(&bytes), ContentEncoding::Base64),
                (Some(ContentEncoding::Utf8), bytes) => {
                    let text = String::from_utf8(bytes).map_err(|e| {
                        (
                            StatusCode::UNPROCESSABLE_ENTITY,
                            Json(ErrorResponse {
                                error: "Invalid UTF-8".to_string(),
                                detail: Some(e.to_string()),
                            }),
                        )
                    })?;
                    (text, ContentEncoding::Utf8)
                }
                (None, bytes) => match String::from_utf8(bytes) {
                    Ok(text) => (text, ContentEncoding::Utf8),
                    Err(err) => (BASE64.encode(err.into_bytes()), ContentEncoding::Base64),
                },
            };
            Ok(Json(ReadResponse {
                path: params.path,
                content: text,
                size,
                encoding,
            }))
        }
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Read failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn write(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WriteRequest>,
) -> Result<Json<WriteResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let encoding = req.encoding.unwrap_or(ContentEncoding::Utf8);
    let bytes = match encoding {
        ContentEncoding::Utf8 => req.content.into_bytes(),
        ContentEncoding::Base64 => BASE64.decode(req.content.as_bytes()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid base64 content".to_string(),
                    detail: Some(e.to_string()),
                }),
            )
        })?,
    };
    let len = bytes.len();

    match state.vfs().write(&req.path, &bytes).await {
        Ok(()) => Ok(Json(WriteResponse {
            path: req.path,
            bytes_written: len,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Write failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<PathQuery>,
) -> Result<Json<DeleteResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    match state.vfs().delete(&params.path).await {
        Ok(()) => Ok(Json(DeleteResponse {
            path: params.path,
            deleted: true,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Delete failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn stat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<PathQuery>,
) -> Result<Json<StatResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    match state.vfs().stat(&params.path).await {
        Ok(metadata) => Ok(Json(StatResponse {
            path: params.path,
            size: metadata.size.unwrap_or(0),
            is_dir: metadata.is_dir,
            modified: metadata.modified.map(|t| {
                t
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string()
            }),
        })),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Stat failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn ls(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<PathQuery>,
) -> Result<Json<Vec<LsEntry>>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    match state.vfs().list(&params.path).await {
        Ok(entries) => Ok(Json(
            entries
                .into_iter()
                .map(|e| LsEntry {
                    path: e.path,
                    is_dir: e.is_dir,
                    size: e.size.unwrap_or(0),
                })
                .collect(),
        )),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "List failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let engine = state.search_engine().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Search not available".to_string(),
                detail: Some("Semantic search engine not configured".to_string()),
            }),
        )
    })?;

    let config = ax_local::SearchConfig {
        limit: req.limit,
        min_score: req.min_score.unwrap_or(0.0),
        ..Default::default()
    };

    match engine.search(&req.query, &config).await {
        Ok(results) => {
            let hits: Vec<SearchHit> = results
                .into_iter()
                .map(|r| SearchHit {
                    path: r.chunk.source_path,
                    content: r.chunk.content,
                    score: r.score,
                    dense_score: r.dense_score,
                    sparse_score: r.sparse_score,
                })
                .collect();
            Ok(Json(SearchResponse {
                query: req.query,
                hits,
            }))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Search failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn append(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AppendRequest>,
) -> Result<Json<AppendResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let encoding = req.encoding.unwrap_or(ContentEncoding::Utf8);
    let bytes = match encoding {
        ContentEncoding::Utf8 => req.content.into_bytes(),
        ContentEncoding::Base64 => BASE64.decode(req.content.as_bytes()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid base64 content".to_string(),
                    detail: Some(e.to_string()),
                }),
            )
        })?,
    };
    let len = bytes.len();

    match state.vfs().append(&req.path, &bytes).await {
        Ok(()) => Ok(Json(AppendResponse {
            path: req.path,
            bytes_appended: len,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Append failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn exists(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<PathQuery>,
) -> Result<Json<ExistsResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    match state.vfs().exists(&params.path).await {
        Ok(exists_val) => Ok(Json(ExistsResponse {
            path: params.path,
            exists: exists_val,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Exists check failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RenameRequest>,
) -> Result<Json<RenameResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    match state.vfs().rename(&req.from, &req.to).await {
        Ok(()) => Ok(Json(RenameResponse {
            from: req.from,
            to: req.to,
            renamed: true,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Rename failed".to_string(),
                detail: Some(e.to_string()),
            }),
        )),
    }
}

pub async fn copy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CopyRequest>,
) -> Result<Json<CopyResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    // Read source, then write to destination
    let content = state.vfs().read(&req.src).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Copy failed: source not found".to_string(),
                detail: Some(e.to_string()),
            }),
        )
    })?;

    let len = content.len();
    state.vfs().write(&req.dst, &content).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Copy failed: write error".to_string(),
                detail: Some(e.to_string()),
            }),
        )
    })?;

    Ok(Json(CopyResponse {
        src: req.src,
        dst: req.dst,
        bytes_copied: len,
    }))
}

pub async fn openapi() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": "AX Virtual Filesystem API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "REST API for AX — a virtual filesystem for AI agents with semantic search, caching, and multi-backend support."
        },
        "paths": {
            "/health": {
                "get": {
                    "summary": "Health check",
                    "operationId": "health",
                    "responses": {
                        "200": { "description": "Server is healthy", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/HealthResponse" } } } }
                    }
                }
            },
            "/status": {
                "get": {
                    "summary": "Server status and mount info",
                    "operationId": "status",
                    "security": [{ "bearerAuth": [] }],
                    "responses": {
                        "200": { "description": "Server status", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/StatusResponse" } } } }
                    }
                }
            },
            "/read": {
                "get": {
                    "summary": "Read a file",
                    "operationId": "readFile",
                    "security": [{ "bearerAuth": [] }],
                    "parameters": [
                        { "name": "path", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "encoding", "in": "query", "required": false, "schema": { "type": "string", "enum": ["utf8", "base64"] } }
                    ],
                    "responses": {
                        "200": { "description": "File content", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ReadResponse" } } } },
                        "404": { "description": "File not found" }
                    }
                }
            },
            "/write": {
                "post": {
                    "summary": "Write a file",
                    "operationId": "writeFile",
                    "security": [{ "bearerAuth": [] }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/WriteRequest" } } } },
                    "responses": {
                        "200": { "description": "Write successful", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/WriteResponse" } } } }
                    }
                }
            },
            "/delete": {
                "delete": {
                    "summary": "Delete a file",
                    "operationId": "deleteFile",
                    "security": [{ "bearerAuth": [] }],
                    "parameters": [{ "name": "path", "in": "query", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "Delete successful", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeleteResponse" } } } }
                    }
                }
            },
            "/stat": {
                "get": {
                    "summary": "Get file metadata",
                    "operationId": "statFile",
                    "security": [{ "bearerAuth": [] }],
                    "parameters": [{ "name": "path", "in": "query", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "File metadata", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/StatResponse" } } } },
                        "404": { "description": "File not found" }
                    }
                }
            },
            "/ls": {
                "get": {
                    "summary": "List directory contents",
                    "operationId": "listDirectory",
                    "security": [{ "bearerAuth": [] }],
                    "parameters": [{ "name": "path", "in": "query", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "Directory listing", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/LsEntry" } } } } }
                    }
                }
            },
            "/search": {
                "post": {
                    "summary": "Semantic search across indexed files",
                    "operationId": "search",
                    "security": [{ "bearerAuth": [] }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SearchRequest" } } } },
                    "responses": {
                        "200": { "description": "Search results", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SearchResponse" } } } },
                        "503": { "description": "Search engine not configured" }
                    }
                }
            },
            "/grep": {
                "get": {
                    "summary": "Regex search in files",
                    "operationId": "grep",
                    "security": [{ "bearerAuth": [] }],
                    "parameters": [
                        { "name": "pattern", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "path", "in": "query", "schema": { "type": "string", "default": "/" } },
                        { "name": "recursive", "in": "query", "schema": { "type": "boolean", "default": false } }
                    ],
                    "responses": {
                        "200": { "description": "Grep matches", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/GrepMatch" } } } } }
                    }
                }
            },
            "/append": {
                "post": {
                    "summary": "Append content to a file",
                    "operationId": "appendFile",
                    "security": [{ "bearerAuth": [] }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AppendRequest" } } } },
                    "responses": {
                        "200": { "description": "Append successful", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AppendResponse" } } } }
                    }
                }
            },
            "/exists": {
                "get": {
                    "summary": "Check if a path exists",
                    "operationId": "existsPath",
                    "security": [{ "bearerAuth": [] }],
                    "parameters": [{ "name": "path", "in": "query", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "Existence check result", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ExistsResponse" } } } }
                    }
                }
            },
            "/rename": {
                "post": {
                    "summary": "Rename/move a file",
                    "operationId": "renameFile",
                    "security": [{ "bearerAuth": [] }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RenameRequest" } } } },
                    "responses": {
                        "200": { "description": "Rename successful", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RenameResponse" } } } }
                    }
                }
            },
            "/copy": {
                "post": {
                    "summary": "Copy a file",
                    "operationId": "copyFile",
                    "security": [{ "bearerAuth": [] }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CopyRequest" } } } },
                    "responses": {
                        "200": { "description": "Copy successful", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CopyResponse" } } } },
                        "404": { "description": "Source not found" }
                    }
                }
            },
        },
        "components": {
            "securitySchemes": {
                "bearerAuth": { "type": "http", "scheme": "bearer" }
            },
            "schemas": {
                "HealthResponse": { "type": "object", "properties": { "status": { "type": "string" }, "version": { "type": "string" } } },
                "StatusResponse": { "type": "object", "properties": { "uptime_secs": { "type": "integer" }, "mounts": { "type": "array", "items": { "$ref": "#/components/schemas/MountInfo" } } } },
                "MountInfo": { "type": "object", "properties": { "path": { "type": "string" }, "backend": { "type": "string" } } },
                "ReadResponse": { "type": "object", "properties": { "path": { "type": "string" }, "content": { "type": "string" }, "size": { "type": "integer" }, "encoding": { "type": "string", "enum": ["utf8", "base64"] } } },
                "WriteRequest": { "type": "object", "required": ["path", "content"], "properties": { "path": { "type": "string" }, "content": { "type": "string" }, "encoding": { "type": "string", "enum": ["utf8", "base64"] } } },
                "WriteResponse": { "type": "object", "properties": { "path": { "type": "string" }, "bytes_written": { "type": "integer" } } },
                "DeleteResponse": { "type": "object", "properties": { "path": { "type": "string" }, "deleted": { "type": "boolean" } } },
                "StatResponse": { "type": "object", "properties": { "path": { "type": "string" }, "size": { "type": "integer" }, "is_dir": { "type": "boolean" }, "modified": { "type": "string", "nullable": true } } },
                "LsEntry": { "type": "object", "properties": { "path": { "type": "string" }, "is_dir": { "type": "boolean" }, "size": { "type": "integer" } } },
                "SearchRequest": { "type": "object", "required": ["query"], "properties": { "query": { "type": "string" }, "limit": { "type": "integer", "default": 10 }, "min_score": { "type": "number", "nullable": true } } },
                "SearchResponse": { "type": "object", "properties": { "query": { "type": "string" }, "hits": { "type": "array", "items": { "$ref": "#/components/schemas/SearchHit" } } } },
                "SearchHit": { "type": "object", "properties": { "path": { "type": "string" }, "content": { "type": "string" }, "score": { "type": "number" }, "dense_score": { "type": "number", "nullable": true }, "sparse_score": { "type": "number", "nullable": true } } },
                "GrepMatch": { "type": "object", "properties": { "path": { "type": "string" }, "line_number": { "type": "integer" }, "line": { "type": "string" } } },
                "AppendRequest": { "type": "object", "required": ["path", "content"], "properties": { "path": { "type": "string" }, "content": { "type": "string" }, "encoding": { "type": "string", "enum": ["utf8", "base64"] } } },
                "AppendResponse": { "type": "object", "properties": { "path": { "type": "string" }, "bytes_appended": { "type": "integer" } } },
                "ExistsResponse": { "type": "object", "properties": { "path": { "type": "string" }, "exists": { "type": "boolean" } } },
                "RenameRequest": { "type": "object", "required": ["from", "to"], "properties": { "from": { "type": "string" }, "to": { "type": "string" } } },
                "RenameResponse": { "type": "object", "properties": { "from": { "type": "string" }, "to": { "type": "string" }, "renamed": { "type": "boolean" } } },
                "CopyRequest": { "type": "object", "required": ["src", "dst"], "properties": { "src": { "type": "string" }, "dst": { "type": "string" } } },
                "CopyResponse": { "type": "object", "properties": { "src": { "type": "string" }, "dst": { "type": "string" }, "bytes_copied": { "type": "integer" } } }
            }
        }
    }))
}

pub async fn grep(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<GrepQuery>,
) -> Result<Json<Vec<GrepMatch>>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let re = match regex::Regex::new(&params.pattern) {
        Ok(r) => r,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid pattern".to_string(),
                    detail: Some(e.to_string()),
                }),
            ));
        }
    };

    let mut matches = Vec::new();

    // If path points to a file, grep it directly
    if let Ok(content) = state.vfs().read(&params.path).await {
        let text = String::from_utf8_lossy(&content);
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                matches.push(GrepMatch {
                    path: params.path.clone(),
                    line_number: i + 1,
                    line: line.to_string(),
                });
            }
        }
        return Ok(Json(matches));
    }

    // Otherwise, list directory and grep files
    if params.recursive {
        grep_recursive(state.vfs(), &params.path, &re, &mut matches, 10).await;
    } else if let Ok(entries) = state.vfs().list(&params.path).await {
        for entry in entries {
            if !entry.is_dir {
                if let Ok(content) = state.vfs().read(&entry.path).await {
                    let text = String::from_utf8_lossy(&content);
                    for (i, line) in text.lines().enumerate() {
                        if re.is_match(line) {
                            matches.push(GrepMatch {
                                path: entry.path.clone(),
                                line_number: i + 1,
                                line: line.to_string(),
                            });
                            if matches.len() >= 100 {
                                return Ok(Json(matches));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Json(matches))
}

#[async_recursion::async_recursion]
async fn grep_recursive(
    vfs: &ax_remote::Vfs,
    path: &str,
    re: &regex::Regex,
    matches: &mut Vec<GrepMatch>,
    max_depth: usize,
) {
    if max_depth == 0 || matches.len() >= 100 {
        return;
    }
    if let Ok(entries) = vfs.list(path).await {
        for entry in entries {
            if entry.is_dir {
                grep_recursive(vfs, &entry.path, re, matches, max_depth - 1).await;
            } else if let Ok(content) = vfs.read(&entry.path).await {
                let text = String::from_utf8_lossy(&content);
                for (i, line) in text.lines().enumerate() {
                    if re.is_match(line) {
                        matches.push(GrepMatch {
                            path: entry.path.clone(),
                            line_number: i + 1,
                            line: line.to_string(),
                        });
                        if matches.len() >= 100 {
                            return;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_serialization() {
        let resp = HealthResponse {
            status: "ok".to_string(),
            version: "0.3.0".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse {
            error: "Not found".to_string(),
            detail: Some("File not found".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\":\"Not found\""));
        assert!(json.contains("File not found"));
    }

    #[test]
    fn test_path_query_deserialization() {
        let json = r#"{"path":"/docs/readme.md"}"#;
        let q: PathQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.path, "/docs/readme.md");
    }

    #[test]
    fn test_read_query_deserialization_with_encoding() {
        let json = r#"{"path":"/docs/readme.md","encoding":"base64"}"#;
        let q: ReadQuery = serde_json::from_str(json).unwrap();
        assert!(matches!(q.encoding, Some(ContentEncoding::Base64)));
    }

    #[test]
    fn test_write_request_deserialization() {
        let json = r#"{"path":"/test.txt","content":"hello world"}"#;
        let req: WriteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.path, "/test.txt");
        assert_eq!(req.content, "hello world");
    }

    #[test]
    fn test_write_request_deserialization_with_encoding() {
        let json = r#"{"path":"/test.txt","content":"aGVsbG8=","encoding":"base64"}"#;
        let req: WriteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.path, "/test.txt");
        assert_eq!(req.content, "aGVsbG8=");
        assert!(matches!(req.encoding, Some(ContentEncoding::Base64)));
    }

    #[test]
    fn test_grep_query_defaults() {
        let json = r#"{"pattern":"foo"}"#;
        let q: GrepQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.pattern, "foo");
        assert_eq!(q.path, "/");
        assert!(!q.recursive);
    }

    #[test]
    fn test_search_request_deserialization() {
        let json = r#"{"query":"hello world"}"#;
        let req: SearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "hello world");
        assert_eq!(req.limit, 10);
        assert!(req.min_score.is_none());
    }

    #[test]
    fn test_search_request_with_options() {
        let json = r#"{"query":"test","limit":5,"min_score":0.5}"#;
        let req: SearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "test");
        assert_eq!(req.limit, 5);
        assert_eq!(req.min_score, Some(0.5));
    }

    #[test]
    fn test_search_response_serialization() {
        let resp = SearchResponse {
            query: "test".to_string(),
            hits: vec![SearchHit {
                path: "/doc.txt".to_string(),
                content: "test content".to_string(),
                score: 0.95,
                dense_score: Some(0.9),
                sparse_score: Some(0.3),
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"query\":\"test\""));
        assert!(json.contains("\"score\":0.95"));
    }

    #[test]
    fn test_append_request_deserialization() {
        let json = r#"{"path":"/test.txt","content":"appended"}"#;
        let req: AppendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.path, "/test.txt");
        assert_eq!(req.content, "appended");
    }

    #[test]
    fn test_append_request_deserialization_with_encoding() {
        let json = r#"{"path":"/test.txt","content":"d29ybGQ=","encoding":"base64"}"#;
        let req: AppendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.path, "/test.txt");
        assert_eq!(req.content, "d29ybGQ=");
        assert!(matches!(req.encoding, Some(ContentEncoding::Base64)));
    }

    #[test]
    fn test_rename_request_deserialization() {
        let json = r#"{"from":"/a.txt","to":"/b.txt"}"#;
        let req: RenameRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.from, "/a.txt");
        assert_eq!(req.to, "/b.txt");
    }

    #[test]
    fn test_copy_request_deserialization() {
        let json = r#"{"src":"/a.txt","dst":"/b.txt"}"#;
        let req: CopyRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.src, "/a.txt");
        assert_eq!(req.dst, "/b.txt");
    }

}

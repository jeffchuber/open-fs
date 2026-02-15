# AX Architecture Guide

A comprehensive guide to the AX virtual filesystem architecture.

## Table of Contents

- [AX Architecture Guide](#ax-architecture-guide)
  - [Table of Contents](#table-of-contents)
  - [Overview](#overview)
  - [System Architecture](#system-architecture)
  - [Crate Dependency Graph](#crate-dependency-graph)
  - [Crate Structure](#crate-structure)
  - [Request Flow](#request-flow)
    - [Read Operation](#read-operation)
    - [Write Operation (with WAL)](#write-operation-with-wal)
    - [REST API Request Flow](#rest-api-request-flow)
  - [Backend Abstraction](#backend-abstraction)
  - [Mount-Based Routing](#mount-based-routing)
  - [Caching Layer](#caching-layer)
  - [Sync Engine \& WAL](#sync-engine--wal)
  - [FUSE Integration](#fuse-integration)
  - [MCP Server](#mcp-server)
  - [REST API Server](#rest-api-server)
  - [Indexing Pipeline](#indexing-pipeline)
  - [Search Architecture](#search-architecture)
  - [Configuration System](#configuration-system)
  - [Security \& Credential Handling](#security--credential-handling)
  - [Error Handling \& Resilience](#error-handling--resilience)
  - [Observability](#observability)
  - [AI Tool Generation](#ai-tool-generation)
  - [Language Bindings](#language-bindings)
  - [CLI Reference](#cli-reference)
  - [Design Principles](#design-principles)

---

## Overview

AX (Agentic Files) is a virtual filesystem designed for AI agents and automation.
It provides a unified interface to multiple storage backends with support for
caching, syncing, semantic search, and tool generation for AI assistants.

```
+=====================================================================+
|                        AX VIRTUAL FILESYSTEM                        |
|                                                                     |
|  "One API to rule them all — local files, S3, databases, vectors"   |
+=====================================================================+
|                                                                     |
|   +---------+ +----+ +-----+ +------+ +-----+ +------+ +------+    |
|   | Python  | | TS | | CLI | | FUSE | | MCP | | REST | |WebUI |    |
|   |  PyO3   | |napi| |clap | |fuser | |stdio| | Axum | |(any) |    |
|   +----+----+ +-+--+ +--+--+ +--+---+ +--+--+ +--+---+ +--+---+    |
|        |        |       |       |        |        |        |        |
|        +--------+---+---+-------+--------+--------+--------+        |
|                     |                                               |
|              +------v------+                                        |
|              |   ax-core   |                                        |
|              |     VFS     |                                        |
|              +------+------+                                        |
|                     |                                               |
|   +------+------+--+---+------+------+------+------+------+        |
|   |      |      |      |      |      |      |      |      |        |
|   v      v      v      v      v      v      v      v      v        |
| +----+ +----+ +--+ +------+ +------+ +----+ +----+ +---+ +-----+   |
| +----+ +----+ +--+ +------+ +------+ +----+ +----+ +---+ +-----+   |
|                                                                     |
+=====================================================================+
```

**Key stats:**
- 10 crates, 100+ source files
- 617+ tests across all crates (including integration test suites)
- 27 CLI subcommands
- 7 MCP tools, 14 REST endpoints
- 4 sync modes, 4 sync profiles
- Hybrid semantic + keyword search

---

## System Architecture

```
                               +------------------+
                               |    YAML Config   |
                               |    (ax.yaml)     |
                               +--------+---------+
                                        |
                                        v
+=========================================================================+
|                                ax-config                                |
|  +-----------+  +--------------+  +------------+  +----------+          |
|  | Parsing & |  | Environment  |  | Validation |  | Secret   |          |
|  |   Types   |  | Interpolation|  |  & Defaults|  | (redact) |          |
|  +-----------+  +--------------+  +------------+  +----------+          |
+=========================================================================+
                                        |
                                        v
+=========================================================================+
|                                 ax-core                                 |
|                                                                         |
|  +----------+    +----------+    +-----------+    +----------+          |
|  |   VFS    |--->|  Router  |--->|CachedBack-|--->|   Sync   |          |
|  |          |    |          |    |   end     |    |  Engine  |          |
|  +----------+    +----------+    +-----------+    +----------+          |
|       |            (longest        (moka,        (WriteThrough/         |
|       |             prefix)        lock-free)     WriteBack/Mirror)     |
|       v                                            + WAL + Outbox       |
|  +----------+    +----------+    +----------+    +----------+           |
|  | Metrics  |    |  Tools   |    | Pipeline |    |WorkQueue |           |
|  |(counters)|    |(JSON/MCP)|    | (chunk + |    |(SQLite,  |           |
|  +----------+    +----------+    |  embed)  |    | retry,   |           |
|  +----------+    +----------+    +----------+    |  DLQ)    |           |
|  |  Search  |    |  Watcher |                    +----------+           |
|  | (hybrid) |    | (notify) |                                           |
|  +----------+    +----------+                                           |
+=========================================================================+yeah 
          |              |               |              |           |
          v              v               v              v           v
  +----------+   +----------+   +--------+   +----------+   +----------+
  | ax-fuse  |   |ax-back-  |   | ax-mcp |   | ax-      |   | ax-      |
  |          |   |  ends    |   |        |   | server   |   | indexing |
  | FUSE     |   |          |   | MCP    |   | REST     |   | Chunk   |
  | mount    |   | FS  Mem  |   | stdio  |   | Axum     |   | Embed   |
  | .search/ |   | S3  Pg   |   | JSON-  |   | /v1/...  |   | BM25    |
  | virtual  |   | Chroma   |   | RPC    |   | /health  |   | Sparse  |
  +----------+   +----------+   +--------+   +----------+   +----------+
```

---

## Crate Dependency Graph

```
ax-config ─────────────────────────────────────────────────────────┐
    │                                                              │
    ├──> ax-backends ──────────────────────────────────────┐       │
    │        │                                             │       │
    │        │                                             │       │
    ├──> ax-indexing ──────────────────────────┐           │       │
    │        │                                 │           │       │
    │        │                                 │           │       │
    ├──> ax-core ─────────────────────────┐    │           │       │
    │        │  (depends on ax-backends,  │    │           │       │
    │        │   ax-indexing, ax-config)  │    │           │       │
    │        │                            │    │           │       │
    │        ├──> ax-fuse                 │    │           │       │
    │        │    (depends on ax-core,    │    │           │       │
    │        │     ax-config)             │    │           │       │
    │        │                            │    │           │       │
    │        ├──> ax-mcp                  │    │           │       │
    │        │    (depends on ax-core,    │    │           │       │
    │        │     ax-config, ax-backends)│    │           │       │
    │        │                            │    │           │       │
    │        └──> ax-server               │    │           │       │
    │             (depends on ax-core,    │    │           │       │
    │              ax-config, ax-backends,│    │           │       │
    │              ax-indexing)            │    │           │       │
    │                                     │    │           │       │
    └──> ax-cli ──────────────────────────┘    │           │       │
              (depends on ALL crates)          │           │       │
                                               │           │       │
    ax-ffi (excluded, PyO3) ───────────────────┴───────────┘       │
    ax-js  (excluded, napi-rs) ────────────────────────────────────┘
```

**Workspace configuration:**

| Field       | Value                                    |
|-------------|------------------------------------------|
| Version     | 0.3.0                                    |
| Edition     | 2021                                     |
| License     | MIT                                      |
| Repository  | https://github.com/ax-vfs/ax             |
| Members     | 8 crates (ax-config through ax-cli)      |
| Excluded    | ax-ffi (needs Python headers), ax-js (needs Node.js) |

---

## Crate Structure

```
ax/
│
├── Cargo.toml                    # Workspace: version 0.3.0, edition 2021, MIT
│
├── crates/
│   │
│   ├── ax-config/                # Configuration parsing, validation & types
│   │   └── src/
│   │       ├── lib.rs            # VfsConfig::from_yaml/from_file, ConfigError
│   │       ├── types.rs          # Secret, 10 #[non_exhaustive] enums, all config structs
│   │       ├── env.rs            # ${VAR_NAME} environment interpolation
│   │       ├── validation.rs     # Backend-specific validation rules
│   │       ├── defaults.rs       # Smart defaults (infer backend, collection, mode)
│   │       └── migration.rs      # Config version migration
│   │
│   ├── ax-core/                  # Core VFS engine
│   │   ├── src/
│   │   │   ├── lib.rs            # 38 public re-exports
│   │   │   ├── vfs.rs            # Vfs struct: from_config, read, write, list, delete, ...
│   │   │   ├── router.rs         # Mount-based longest-prefix path routing
│   │   │   ├── cache.rs          # moka-based LRU cache (lock-free reads, TinyLFU)
│   │   │   ├── cached_backend.rs # Cache wrapper for any Backend impl
│   │   │   ├── sync.rs           # SyncEngine (WriteThrough/WriteBack/PullMirror)
│   │   │   │                     #   + compute_backoff(Fixed/Linear/Exponential)
│   │   │   ├── wal.rs            # WriteAheadLog (SQLite): WAL log, outbox, sync profiles
│   │   │   │                     #   + checkpoint(), auto_checkpoint_threshold
│   │   │   ├── work_queue.rs     # Persistent SQLite work queue (retry, DLQ, debounce)
│   │   │   ├── persistent_worker.rs # Crash-resilient background index worker
│   │   │   ├── pipeline.rs       # IndexingPipeline orchestration
│   │   │   ├── search.rs         # SearchEngine: hybrid dense + sparse + RRF fusion
│   │   │   ├── index_state.rs    # IndexState: BLAKE3 content dedup, cold boot reconcile
│   │   │   ├── incremental.rs    # Delta-based incremental indexer
│   │   │   ├── index_worker.rs   # [deprecated] Background indexing via mpsc
│   │   │   ├── watcher.rs        # FileChange notifications via notify crate
│   │   │   ├── tools.rs          # AI tool definition generation (JSON/MCP/OpenAI)
│   │   │   ├── traits.rs         # Backend trait definition + Entry struct
│   │   │   ├── error.rs          # VfsError (#[non_exhaustive], 8 variants)
│   │   │   └── metrics.rs        # VfsMetrics: counters, latency, RAII guards
│   │   └── benches/
│   │       ├── cache_bench.rs    # Cache performance benchmarks
│   │       └── vfs_bench.rs      # VFS operation benchmarks
│   │
│   ├── ax-backends/              # Storage backend implementations
│   │   └── src/
│   │       ├── lib.rs            # Public re-exports, feature-gated backends
│   │       ├── traits.rs         # Backend trait: read, write, append, delete, list, ...
│   │       ├── error.rs          # BackendError (#[non_exhaustive], 8 variants, is_transient)
│   │       ├── fs.rs             # Local filesystem (std::fs)
│   │       ├── memory.rs         # In-memory HashMap (testing)
│   │       ├── s3.rs             # S3-compatible (AWS, R2, MinIO) — Secret credentials
│   │       ├── postgres.rs       # PostgreSQL blob store — SQL injection protected, Secret
│   │       ├── chroma.rs         # Chroma vector database
│   │
│   ├── ax-indexing/              # Text processing & search indexing
│   │   └── src/
│   │       ├── lib.rs            # IndexingError, pipeline types
│   │       ├── types.rs          # Chunk, SearchResult, SparseVector
│   │       ├── content_hash.rs   # BLAKE3 content hashing
│   │       ├── sparse.rs         # BM25 sparse encoding
│   │       ├── chunkers/
│   │       │   ├── mod.rs        # Strategy dispatch
│   │       │   ├── fixed.rs      # Fixed-size chunks
│   │       │   ├── recursive.rs  # Recursive splitting (paragraphs→lines→sentences)
│   │       │   ├── semantic.rs   # Semantic boundary detection
│   │       │   └── ast.rs        # AST-aware code chunking (tree-sitter)
│   │       ├── embedders/
│   │       │   ├── mod.rs        # Embedder trait & dispatch
│   │       │   ├── ollama.rs     # Local Ollama embeddings
│   │       │   ├── openai.rs     # OpenAI API embeddings
│   │       │   └── stub.rs       # No-op testing stub
│   │       └── extractors/
│   │           ├── mod.rs        # Text extraction dispatch
│   │           ├── plaintext.rs  # .txt, .md, .rs, .py, .js, ...
│   │           └── pdf.rs        # PDF text extraction
│   │
│   ├── ax-fuse/                  # FUSE filesystem integration
│   │   ├── src/
│   │   │   ├── lib.rs            # AxFuse type alias, pub(crate) internal modules
│   │   │   ├── async_bridge.rs   # OnceLock<Result<Runtime,String>> sync↔async bridge
│   │   │   ├── common.rs         # AxFsCore: platform-neutral VFS logic [pub(crate)]
│   │   │   ├── unix_fuse.rs      # fuser::Filesystem impl [pub(crate), cfg(unix)]
│   │   │   ├── inode.rs          # Inode table: path↔u64 bidirectional mapping
│   │   │   └── search_dir.rs     # Virtual /.search/query/ directory
│   │   └── tests/
│   │       └── integration.rs    # FUSE integration tests
│   │
│   ├── ax-mcp/                   # MCP (Model Context Protocol) server
│   │   └── src/
│   │       ├── lib.rs            # McpServer entry point
│   │       ├── protocol.rs       # JSON-RPC 2.0 types, MCP messages
│   │       ├── handler.rs        # Tool dispatch: ax_read, ax_write, ax_ls, ...
│   │       └── server.rs         # Stdio transport, 30s per-tool timeout
│   │
│   ├── ax-server/                # REST API server (Axum)
│   │   └── src/
│   │       ├── lib.rs            # ServerConfig (Secret api_key), graceful shutdown
│   │       ├── state.rs          # AppState: Vfs + Secret + SearchEngine + uptime
│   │       ├── routes.rs         # Router: /v1/ prefix, middleware stack
│   │       └── handlers.rs       # Handlers: health_live, health_ready, CRUD, search
│   │
│   ├── ax-cli/                   # Command-line interface (27 subcommands)
│   │   └── src/
│   │       ├── main.rs           # Cli struct, Commands enum, WalAction enum
│   │       ├── errors.rs         # User-friendly error formatting
│   │       └── commands/
│   │           ├── mod.rs         # Module declarations (29 submodules)
│   │           ├── cat.rs, ls.rs, write.rs, rm.rs, append.rs, ...
│   │           ├── search.rs, grep.rs, find.rs, index.rs, ...
│   │           ├── mount.rs, unmount.rs, watch.rs
│   │           ├── serve.rs, mcp.rs, tools.rs
│   │           ├── wal.rs         # ax wal checkpoint / ax wal status
│   │           └── validate.rs, migrate.rs, config.rs, status.rs
│   │
│   ├── ax-ffi/                   # Python bindings (PyO3, excluded from workspace)
│   │   └── src/lib.rs            # PyVfs, PyEntry classes
│   │
│   └── ax-js/                    # TypeScript bindings (napi-rs, excluded)
│       └── src/lib.rs            # JsVfs, JsEntry
│
├── configs/                      # Example YAML configurations
├── examples/                     # Usage examples (Python, TypeScript)
└── docs/
    ├── ARCHITECTURE.md           # This file
    ├── PROJECT_STATUS.md         # Current development status
    ├── GETTING_STARTED.md        # FUSE + Claude Code quick start
    ├── GUIDE.md                  # Gentle introduction to AX
    ├── USE_CASES.md              # AI agent use case patterns
    └── CLAUDE_CODE_INTEGRATION.md # FUSE integration guide
```

---

## Request Flow

### Read Operation

```
Client: vfs.read("/workspace/docs/readme.md")
         │
         v
┌──────────────────┐
│       VFS        │
│  1. Validate     │
│     path format  │
└────────┬─────────┘
         │
         v
┌──────────────────┐
│     Router       │
│  2. Longest-     │
│     prefix match │
│     /workspace   │
│  3. Strip prefix │
│   → docs/        │
│     readme.md    │
└────────┬─────────┘
         │
         v
┌──────────────────┐
│  CachedBackend   │  ◄── moka lock-free cache
│                  │
│  4. Check cache ─┼──── HIT → return cached content
│                  │
│  5. MISS:        │
│     forward to   │
│     backend      │
└────────┬─────────┘
         │
         v
┌──────────────────┐
│   FsBackend      │
│  6. Read file    │
│     from disk    │
│  7. Return bytes │
└────────┬─────────┘
         │
         v
┌──────────────────┐
│  CachedBackend   │
│  8. Store in     │
│     cache (async)│
└────────┬─────────┘
         │
         v
    Return to client
```

### Write Operation (with WAL)

```
Client: vfs.write("/data/file.txt", content)
         │
         v
┌──────────────────┐
│       VFS        │
│  1. Validate     │
│     path format  │
└────────┬─────────┘
         │
         v
┌──────────────────┐
│     Router       │
│  2. Find mount   │
│     /data        │
│  3. Read-only? ──┼──── YES → Return Error
│  4. Strip prefix │
│   → file.txt     │
└────────┬─────────┘
         │
         v
┌──────────────────────────────────────────────────┐
│               SyncEngine + WAL                    │
│                                                   │
│  ┌──────────────────────────────────────────────┐ │
│  │ WRITE-THROUGH path:                          │ │
│  │  5a. WAL: log_write(op, path, content)       │ │
│  │  5b. Backend: write(path, content)           │ │
│  │  5c. WAL: mark_applied(wal_id)               │ │
│  │  5d. Outbox: enqueue for remote sync         │ │
│  │  5e. Cache: invalidate(path)                 │ │
│  │  5f. Return success to client                │ │
│  └──────────────────────────────────────────────┘ │
│                                                   │
│  ┌──────────────────────────────────────────────┐ │
│  │ WRITE-BACK path:                             │ │
│  │  5a. WAL: log_write(op, path, content)       │ │
│  │  5b. Queue write for background flush        │ │
│  │  5c. Return success immediately              │ │
│  │  ... later (background task) ...             │ │
│  │  5d. Batch write to backend                  │ │
│  │  5e. WAL: mark_applied(wal_id)               │ │
│  │  5f. Outbox: complete on success / retry     │ │
│  └──────────────────────────────────────────────┘ │
│                                                   │
│  Auto-checkpoint: after 500 applied WAL entries,  │
│  prune entries older than 24h automatically       │
└───────────────────────────────────────────────────┘
```

### REST API Request Flow

```
HTTP Client
    │
    │  POST /v1/write  (JSON body, Bearer token)
    v
┌──────────────────────────────────────────────────────────────┐
│                      Axum Middleware Stack                     │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │  Layer 1: RequestBodyLimitLayer (50 MB max)             │  │
│  ├─────────────────────────────────────────────────────────┤  │
│  │  Layer 2: TimeoutLayer (60 seconds)                     │  │
│  ├─────────────────────────────────────────────────────────┤  │
│  │  Layer 3: TraceLayer (structured HTTP logging)          │  │
│  └─────────────────────────────────────────────────────────┘  │
│                          │                                    │
│                          v                                    │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │  Auth Check: Bearer token vs AppState.api_key           │  │
│  │  (health endpoints bypass auth)                         │  │
│  └───────────────────────┬─────────────────────────────────┘  │
│                          │                                    │
│                          v                                    │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │  Route Handler: handlers::write()                       │  │
│  │  → VFS.write(path, content)                             │  │
│  └───────────────────────┬─────────────────────────────────┘  │
│                          │                                    │
│                          v                                    │
│               JSON Response (200 OK)                          │
└──────────────────────────────────────────────────────────────┘
```

---

## Backend Abstraction

All backends implement a common async trait for uniform access:

```
+=====================================================================+
|                          Backend Trait                               |
+=====================================================================+
|                                                                     |
|   async fn read(&self, path: &str) -> Result<Vec<u8>>              |
|   async fn write(&self, path: &str, content: &[u8]) -> Result<()>  |
|   async fn append(&self, path: &str, content: &[u8]) -> Result<()> |
|   async fn delete(&self, path: &str) -> Result<()>                 |
|   async fn list(&self, path: &str) -> Result<Vec<Entry>>           |
|   async fn exists(&self, path: &str) -> Result<bool>               |
|   async fn stat(&self, path: &str) -> Result<Entry>                |
|   async fn rename(&self, from: &str, to: &str) -> Result<()>       |
|                                                                     |
+=====================================================================+
     ^       ^       ^       ^       ^       ^      ^      ^      ^
     |       |       |       |       |       |      |      |      |
  +------+ +----+ +------+ +------+ +------+ +----+ +----+ +---+ +-----+
  +------+ +----+ +------+ +------+ +------+ +----+ +----+ +---+ +-----+
  |Local | |Hash | |AWS,  | |blob  | |Vector| |HTTP| |SSH | |JSON| |REST|
  |files | |Map  | |MinIO,| |store | |  DB  | |DAV | |file| |API | |API |
  |std:: | |(for | |R2,DO | |upsert| |embed | |PROP| |xfer| |    | |    |
  |  fs  | |test)| |Spaces| |sqlx  | |reqwst| |FIND| |    | |    | |    |
  +------+ +----+ +------+ +------+ +------+ +----+ +----+ +---+ +-----+

Entry Structure:
┌──────────────────────┐
│        Entry         │
├──────────────────────┤
│ path: String         │  Full path to the entry
│ name: String         │  Filename or directory name
│ is_dir: bool         │  True if directory
│ size: Option<u64>    │  File size in bytes
│ modified: Option     │  Last modification time
│   <DateTime<Utc>>    │
│ content_hash:        │  BLAKE3 hash (for dedup)
│   Option<String>     │
└──────────────────────┘
```

**Feature flags** control which backends are compiled:

| Feature        | Backends enabled               | Dependencies                            |
|----------------|-------------------------------|-----------------------------------------|
| `s3`           | S3Backend                     | aws-sdk-s3, aws-config                  |
| `postgres`     | PostgresBackend               | sqlx (postgres, chrono)                 |
| `all-backends` | All of the above              | All of the above                        |

**BackendError** (`#[non_exhaustive]`):

```
BackendError
├── NotFound(path)              Path does not exist
├── NotADirectory(path)         Expected directory
├── PathTraversal(path)         Traversal attack blocked
├── PermissionDenied(path)      Access denied
├── ConnectionFailed { .. }     Backend connection failed  ──┐
├── Timeout { .. }              Operation timed out        ──┤ is_transient() = true
├── Io(std::io::Error)          I/O error (some transient) ──┘
└── Other(String)               Backend-specific error
```

---

## Mount-Based Routing

```
Configuration:
┌───────────────────────────────────────────┐
│  backends:                                │
│    local:                                 │
│      type: fs                             │
│      root: ./data                         │
│    remote:                                │
│      type: s3                             │
│      bucket: my-bucket                    │
│                                           │
│  mounts:                                  │
│    - path: /workspace                     │
│      backend: local                       │
│      mode: local_indexed                  │
│    - path: /archive                       │
│      backend: remote                      │
│      read_only: true                      │
│      mode: pull_mirror                    │
└───────────────────────────────────────────┘

Virtual Filesystem View:
/
├── workspace/              ──→ FsBackend (./data)
│   ├── src/
│   │   ├── main.rs
│   │   └── lib.rs
│   └── Cargo.toml
│
└── archive/                ──→ S3Backend (my-bucket) [READ-ONLY]
    ├── 2024/
    │   └── backup-01.tar
    └── 2023/
        └── backup-12.tar

Path Resolution (longest prefix match):
┌─────────────────────────────────────────────┐
│  Input: /workspace/src/main.rs              │
│                                             │
│  1. Match mount: /workspace (longest)       │
│  2. Backend: local (FsBackend)              │
│  3. Relative path: src/main.rs              │
│  4. Full path: ./data/src/main.rs           │
├─────────────────────────────────────────────┤
│  Input: /archive/2024/backup.tar            │
│                                             │
│  1. Match mount: /archive                   │
│  2. Backend: remote (S3Backend)             │
│  3. Relative path: 2024/backup.tar          │
│  4. S3 key: 2024/backup.tar                 │
├─────────────────────────────────────────────┤
│  Nested mounts (longest prefix wins):       │
│                                             │
│  Mounts: /data → BackendA                   │
│          /data/special → BackendB           │
│                                             │
│  /data/special/file.txt → BackendB          │
│  /data/other/file.txt → BackendA            │
└─────────────────────────────────────────────┘
```

---

## Caching Layer

High-performance concurrent LRU cache using `moka` for lock-free reads:

```
┌──────────────────────────────────────────────────────────────┐
│                      Cache Architecture                       │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│                  ┌────────────────────┐                       │
│                  │   CachedBackend    │                       │
│                  └────────┬───────────┘                       │
│                           │                                   │
│   read(path) ────────────>│                                   │
│                           v                                   │
│                  ┌────────────────────┐                       │
│                  │   moka::Cache      │  Lock-free reads!     │
│                  │   (TinyLFU + LRU)  │  No write-lock on     │
│                  └───────┬──┬─────────┘  read path            │
│                    HIT   │  │  MISS                           │
│                    │     │  │    │                             │
│                    v     │  │    v                             │
│              Return      │  │  Forward to                     │
│              cached      │  │  inner backend                  │
│              data        │  │    │                             │
│                          │  │    v                             │
│                          │  │  Insert into                    │
│                          │  │  cache (async)                  │
│                          │  │    │                             │
│                          │  └────┘                             │
│                          v                                    │
│                    Return data                                │
│                                                               │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  CacheConfig:                    CacheStats:                  │
│  ┌──────────────────────┐       ┌──────────────────────┐     │
│  │ max_entries: 1000    │       │ hits: u64             │     │
│  │ max_size: 100 MB     │       │ misses: u64           │     │
│  │ ttl: 300s (5 min)    │       │ entries: usize        │     │
│  │ enabled: true        │       │ size: usize (bytes)   │     │
│  └──────────────────────┘       │ evictions: u64        │     │
│                                 │ expirations: u64      │     │
│  Why moka over RwLock?          │ hit_rate() → f64      │     │
│  ┌──────────────────────┐       └──────────────────────┘     │
│  │ RwLock<HashMap>:     │                                    │
│  │  Write lock on EVERY │                                    │
│  │  read (LRU update)   │                                    │
│  │  Blocks all readers  │                                    │
│  │                      │                                    │
│  │ moka::Cache:         │                                    │
│  │  Lock-free reads     │                                    │
│  │  TinyLFU admission   │                                    │
│  │  Built-in TTL        │                                    │
│  │  Async eviction      │                                    │
│  └──────────────────────┘                                    │
└──────────────────────────────────────────────────────────────┘
```

---

## Sync Engine & WAL

Multiple sync modes for different consistency/performance trade-offs,
with WAL-backed durability and a durable outbox for crash-safe sync:

```
┌══════════════════════════════════════════════════════════════════════┐
│                       Sync Mode Comparison                          │
├──────────┬──────────────────┬──────────────────┬────────────────────┤
│ Mode     │ Read Path        │ Write Path       │ Use Case           │
├──────────┼──────────────────┼──────────────────┼────────────────────┤
│ None     │ Backend direct   │ Backend direct   │ Simple local       │
│ WriteThr │ Cache → Backend  │ Cache + Backend  │ Consistency        │
│ WriteBack│ Cache → Backend  │ Cache, async bg  │ Performance        │
│ PullMirr │ Cache → Backend  │ READ-ONLY        │ Remote docs        │
└──────────┴──────────────────┴──────────────────┴────────────────────┘

┌═══════════════════════════════════════════════════════════════┐
│                WAL + Outbox Architecture                      │
│                                                               │
│  All operations logged to SQLite WAL before being applied.    │
│  Pending remote sync stored in durable outbox that survives   │
│  process crashes.                                             │
│                                                               │
│  write(path, content)                                         │
│        │                                                      │
│        v                                                      │
│  ┌────────────┐                                               │
│  │  WAL Log   │  1. log_write(op, path, content, mount)      │
│  │  (SQLite)  │  2. Apply to local backend                   │
│  │            │  3. mark_applied(wal_id)                      │
│  └─────┬──────┘     │                                         │
│        │            │ auto-checkpoint triggers after           │
│        │            │ 500 applied entries (configurable)       │
│        v            v                                         │
│  ┌────────────┐  ┌────────────┐                               │
│  │   Outbox   │  │ Checkpoint │                               │
│  │  (SQLite)  │  │ prune old  │                               │
│  │  upsert:   │  │ applied    │                               │
│  │  latest    │  │ entries    │                               │
│  │  event     │  │ + VACUUM   │                               │
│  │  wins      │  └────────────┘                               │
│  └─────┬──────┘                                               │
│        │                                                      │
│        v    Background drain task:                            │
│  ┌────────────┐                                               │
│  │   Drain    │  fetch_ready_outbox(batch_size)               │
│  │   Worker   │  → mark_processing(id)                        │
│  └─────┬──────┘  → sync to remote backend                    │
│        │         → complete_outbox(id) on success             │
│   ┌────┴────┐    → fail_outbox(id, err) on failure            │
│   │         │    → retry with exponential backoff             │
│   v         v    → dead letter after max_retries              │
│ Success   Retry                                               │
│ (delete)  (backoff)──→ Dead Letter Queue (after max retries) │
│                                                               │
│  On startup (crash recovery):                                 │
│  1. Replay unapplied WAL entries                              │
│  2. recover_stuck() processing entries                        │
│  3. Resume outbox drain                                       │
│                                                               │
├───────────────────────────────────────────────────────────────┤
│                                                               │
│  Per-Mount Sync Profiles:                                     │
│  ┌──────────────┬──────────────────────────────────────────┐  │
│  │ LocalOnly    │ No syncing, local only                   │  │
│  │ LocalFirst   │ Write local, sync to remote later (dflt) │  │
│  │ RemoteFirst  │ Write to remote first, then cache local  │  │
│  │ RemoteOnly   │ Remote only, no local state              │  │
│  └──────────────┴──────────────────────────────────────────┘  │
│                                                               │
│  Backoff Strategies (compute_backoff):                        │
│  ┌──────────────┬──────────────────────────────────────────┐  │
│  │ Fixed        │ base                                     │  │
│  │ Linear       │ base × (attempt + 1)                     │  │
│  │ Exponential  │ base × 2^attempt  (default)              │  │
│  └──────────────┴──────────────────────────────────────────┘  │
│                                                               │
│  WalConfig:                                                   │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │ max_retries: 5                                           │ │
│  │ base_backoff_secs: 2                                     │ │
│  │ recover_on_startup: true                                 │ │
│  │ stuck_timeout_secs: 300                                  │ │
│  │ auto_checkpoint_threshold: 500                           │ │
│  │ checkpoint_max_age_secs: 86400 (24h)                     │ │
│  └──────────────────────────────────────────────────────────┘ │
└═══════════════════════════════════════════════════════════════┘
```

---

## FUSE Integration

Transparent filesystem integration for Claude Code and other tools:

```
┌═══════════════════════════════════════════════════════════════════┐
│                       FUSE Architecture                           │
│                                                                   │
│  Claude Code / any tool sees a normal filesystem:                 │
│    Read("/ax/workspace/auth.py")                                  │
│    Write("/ax/workspace/test.py", code)                           │
│    Glob("/.search/query/authentication/*")                        │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │                   Kernel / OS                                 │ │
│  └───────────────────────┬──────────────────────────────────────┘ │
│                          │ synchronous callbacks                  │
│                          v                                        │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │              Platform FUSE Driver                             │ │
│  │  UnixFuse (cfg(unix), fuser::Filesystem impl)                │ │
│  │  [pub(crate) — not exposed to downstream crates]             │ │
│  └───────────────────────┬──────────────────────────────────────┘ │
│                          │                                        │
│  ┌───────────────────────v──────────────────────────────────────┐ │
│  │  AxFsCore (platform-neutral, pub(crate) common.rs)           │ │
│  │  do_lookup, do_read, do_write, do_readdir,                   │ │
│  │  do_create, do_mkdir, do_unlink, do_rmdir, do_rename         │ │
│  └───────────────────────┬──────────────────────────────────────┘ │
│                          │                                        │
│  ┌───────────────────────v──────────────────────────────────────┐ │
│  │              Async Bridge (async_bridge.rs)                   │ │
│  │                                                               │ │
│  │  static RUNTIME: OnceLock<Result<Runtime, String>>            │ │
│  │                                                               │ │
│  │  FUSE callbacks are synchronous, AX VFS is async.             │ │
│  │  Bridge pattern:                                              │ │
│  │    init_runtime() → creates 4-worker tokio runtime            │ │
│  │    block_on(future) → Result<T, FuseError>                    │ │
│  │    spawn(future)    → fire-and-forget                         │ │
│  │                                                               │ │
│  │  Uses OnceLock<Result<Runtime, String>> (stable API)          │ │
│  │  instead of unstable get_or_try_init — stores the result      │ │
│  │  of runtime creation for error propagation without panics.    │ │
│  └───────────────────────┬──────────────────────────────────────┘ │
│                          │                                        │
│                          v                                        │
│                    ┌───────────┐                                  │
│                    │  AX VFS   │                                  │
│                    │  (Router) │                                  │
│                    └─────┬─────┘                                  │
│            ┌─────────────┼─────────────────┐                     │
│            v             v                 v                     │
│    ┌─────────────┐ ┌──────────┐  ┌──────────────┐               │
│    │ /workspace  │ │  /docs   │  │  /.search/   │               │
│    │WriteThrough │ │PullMirror│  │  (virtual)   │               │
│    └──────┬──────┘ └────┬─────┘  └──────┬───────┘               │
│           v             v               v                       │
│       Local FS      S3 bucket    Semantic search                │
│       + sync       (read-only)     via SearchEngine             │
│                                                                   │
├───────────────────────────────────────────────────────────────────┤
│                                                                   │
│  Inode Management:                                                │
│  ┌─────────────────────────────────────────────────────────────┐  │
│  │  inodes: HashMap<u64, String>       inode → path            │  │
│  │  path_to_ino: HashMap<String, u64>  path → inode            │  │
│  │  next_ino: AtomicU64                next available inode     │  │
│  │  Reserved: 1 = root directory (/)                           │  │
│  └─────────────────────────────────────────────────────────────┘  │
│                                                                   │
│  Virtual .search/ Directory:                                      │
│  ┌─────────────────────────────────────────────────────────────┐  │
│  │  /.search/                                                  │  │
│  │  └── query/                      Search interface           │  │
│  │      └── {url-encoded-query}/    Results as symlinks        │  │
│  │          ├── 01_auth.py → ../../workspace/src/auth.py      │  │
│  │          └── 02_login.rs → ../../workspace/src/login.rs    │  │
│  │                                                             │  │
│  │  Claude Code uses Glob on /.search/ for semantic queries!  │  │
│  └─────────────────────────────────────────────────────────────┘  │
│                                                                   │
│  FUSE Operations Implemented:                                     │
│  ┌────────────┬────────────────────────────────────────────────┐  │
│  │ lookup     │ Resolve name → inode + attrs                   │  │
│  │ getattr    │ Get file/directory attributes                  │  │
│  │ readdir    │ List directory contents                        │  │
│  │ read       │ Read file data                                 │  │
│  │ write      │ Write file data                                │  │
│  │ create     │ Create new file                                │  │
│  │ mkdir      │ Create directory                               │  │
│  │ unlink     │ Delete file                                    │  │
│  │ rmdir      │ Delete directory                               │  │
│  │ rename     │ Move/rename file                               │  │
│  │ readlink   │ Read symlink target (.search results)          │  │
│  └────────────┴────────────────────────────────────────────────┘  │
└═══════════════════════════════════════════════════════════════════┘
```

---

## MCP Server

JSON-RPC 2.0 MCP server for direct AI agent integration over stdio:

```
┌═══════════════════════════════════════════════════════════════┐
│                    MCP Server (ax-mcp)                         │
│                                                               │
│  Claude Code / AI Agent                                       │
│         │                                                     │
│         │  stdin/stdout (JSON-RPC 2.0)                        │
│         v                                                     │
│  ┌──────────────────┐                                         │
│  │    McpServer     │  Protocol version: "2024-11-05"         │
│  │                  │  Methods: initialize, tools/list,       │
│  │  1. Read JSON    │           tools/call, ping              │
│  │     from stdin   │                                         │
│  │  2. Parse as     │                                         │
│  │     JSON-RPC 2.0 │                                         │
│  └────────┬─────────┘                                         │
│           │                                                   │
│           v                                                   │
│  ┌──────────────────┐                                         │
│  │   McpHandler     │  Per-tool timeout: 30 seconds           │
│  │                  │  (tokio::time::timeout wrapper)         │
│  │  Dispatch to     │                                         │
│  │  tool handler    │  Timeout → JSON-RPC error response:     │
│  └────────┬─────────┘  "Tool 'X' timed out after 30s"        │
│           │                                                   │
│  ┌──┬──┬──┼──┬──┬──┬──┐                                      │
│  v  v  v  v  v  v  v  v                                      │
│ read write ls stat grep search mkdir rm                       │
│                                                               │
│  Tools:                                                       │
│  ┌────────────┬────────────────────────────────────────────┐  │
│  │ ax_read    │ Read file content (text or "[binary]")     │  │
│  │ ax_write   │ Write content to file                      │  │
│  │ ax_ls      │ List directory                             │  │
│  │ ax_stat    │ File metadata (size, is_dir, modified)     │  │
│  │ ax_grep    │ Regex search in files                      │  │
│  │ ax_search  │ Semantic search (if SearchEngine avail)    │  │
│  │ ax_mkdir   │ Create directory                           │  │
│  │ ax_rm      │ Delete file/directory                      │  │
│  └────────────┴────────────────────────────────────────────┘  │
│                                                               │
│  Example exchange:                                            │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │ → {"jsonrpc":"2.0","method":"tools/call",              │  │
│  │    "params":{"name":"ax_read",                         │  │
│  │    "arguments":{"path":"/file.txt"}}, "id":1}          │  │
│  │                                                         │  │
│  │ ← {"jsonrpc":"2.0","result":                           │  │
│  │    {"content":[{"type":"text",                         │  │
│  │     "text":"file contents..."}]}, "id":1}              │  │
│  └─────────────────────────────────────────────────────────┘  │
└═══════════════════════════════════════════════════════════════┘
```

---

## REST API Server

Axum-based HTTP server with authentication, middleware, and API versioning:

```
┌═══════════════════════════════════════════════════════════════════════┐
│                       REST API (ax-server)                            │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │                     Middleware Stack                             │  │
│  │  RequestBodyLimitLayer ── 50 MB max body size                   │  │
│  │  TimeoutLayer ─────────── 60 second request timeout             │  │
│  │  TraceLayer ───────────── Structured HTTP request logging       │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │                       Route Layout                              │  │
│  │                                                                 │  │
│  │  Health (no auth required):                                     │  │
│  │    GET  /health        Full health check (status, version)      │  │
│  │    GET  /health/live   Lightweight liveness probe (200 OK)      │  │
│  │    GET  /health/ready  Readiness check (VFS verify, 5s timeout) │  │
│  │                                                                 │  │
│  │  API routes (auth required if api_key configured):              │  │
│  │  Available at BOTH / (legacy) and /v1/ (versioned):             │  │
│  │                                                                 │  │
│  │    GET    /[v1/]status   Uptime + mount info                    │  │
│  │    GET    /[v1/]read     Read file (?path=/foo.txt)             │  │
│  │    POST   /[v1/]write    Write file (JSON body)                 │  │
│  │    DELETE /[v1/]delete   Delete file (?path=/foo.txt)           │  │
│  │    GET    /[v1/]stat     File metadata (?path=/foo.txt)         │  │
│  │    GET    /[v1/]ls       List directory (?path=/)               │  │
│  │    POST   /[v1/]search   Semantic search (JSON body)            │  │
│  │    GET    /[v1/]grep     Regex search (?pattern=&path=)         │  │
│  │    GET    /[v1/]openapi  OpenAPI 3.0.3 specification            │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  AppState                                                       │  │
│  │  ┌───────────────────────────────────────────────────────────┐  │  │
│  │  │ vfs: Vfs                                                 │  │  │
│  │  │ api_key: Option<Secret>      ← redacted in logs          │  │  │
│  │  │ search_engine: Option<SearchEngine>                      │  │  │
│  │  │ started_at: Instant                                      │  │  │
│  │  └───────────────────────────────────────────────────────────┘  │  │
│  │                                                                 │  │
│  │  Auth: Bearer token in Authorization header                     │  │
│  │  check_auth(): no key configured = open access                  │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  Graceful Shutdown                                              │  │
│  │  shutdown_signal() listens for SIGINT + SIGTERM (Unix)          │  │
│  │  Server: axum::serve(...).with_graceful_shutdown(signal)         │  │
│  │  In-flight requests complete before process exits               │  │
│  └─────────────────────────────────────────────────────────────────┘  │
└═══════════════════════════════════════════════════════════════════════┘
```

---

## Indexing Pipeline

Text processing pipeline for semantic search. Both dense embeddings and sparse
BM25 vectors are computed locally and pushed to Chroma — there is no local
vector storage. Only local (fs/memory) backends are indexed; remote backends

```
┌═══════════════════════════════════════════════════════════════════════┐
│                        Indexing Pipeline                              │
│                                                                       │
│  Input: File path or directory (local backends only)                  │
│         │                                                             │
│         v                                                             │
│  ┌──────────────────┐                                                 │
│  │    Extractor      │  Supported: .txt, .md, .rs, .py, .js, .pdf    │
│  │  Extract raw text │                                                │
│  └────────┬─────────┘                                                 │
│           │                                                           │
│           v                                                           │
│  ┌──────────────────┐  Strategies:                                    │
│  │     Chunker       │  ┌────────────┬────────────────────────────┐   │
│  │  Split into       │  │ Fixed      │ Equal-size chunks          │   │
│  │  chunks           │  │ Recursive  │ \n\n → \n → . → " " → ""  │   │
│  └────────┬─────────┘  │ Semantic   │ Headers, paragraphs        │   │
│           │             │ AST        │ tree-sitter code-aware     │   │
│           │             │ Row        │ Tabular data               │   │
│           │             └────────────┴────────────────────────────┘   │
│           │                                                           │
│           │    Chunk struct:                                          │
│           │    ┌──────────────────────┐                               │
│           │    │ source_path          │                               │
│           │    │ content              │                               │
│           │    │ start_offset/end     │                               │
│           │    │ start_line/end_line  │                               │
│           │    │ chunk_index          │                               │
│           │    │ total_chunks         │                               │
│           │    └──────────────────────┘                               │
│           │                                                           │
│           ├────────────────┐                                          │
│           v                v                                          │
│  ┌──────────────┐  ┌──────────────┐                                   │
│  │   Embedder    │  │SparseEncoder │                                   │
│  │ Dense vectors │  │ BM25 sparse  │                                   │
│  │ (384–1536d)   │  │ vectors      │                                   │
│  └───────┬──────┘  └──────┬───────┘                                   │
│          │                │                                           │
│          │  ┌─────────────┘                                           │
│          v  v                                                         │
│  ┌──────────────────┐                                                 │
│  │     Chroma        │  Both dense and sparse vectors pushed          │
│  │  (remote store)   │  via upsert() with SparseEmbedding             │
│  └──────────────────┘                                                 │
│                                                                       │
│  SparseEncoder State Persistence:                                    │
│  ┌──────────────────────────────────────────────────────────────────┐ │
│  │  SparseEncoder vocab + IDF stored as JSON in Chroma collection  │ │
│  │  metadata (key: "sparse_encoder_state"). Loaded on pipeline     │ │
│  │  startup, persisted after each batch. No local state files.     │ │
│  └──────────────────────────────────────────────────────────────────┘ │
│                                                                       │
├───────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  Persistent Work Queue (work_queue.rs):                               │
│  ┌──────────────────────────────────────────────────────────────────┐ │
│  │  SQLite-backed, WAL mode, survives crashes                      │ │
│  │                                                                  │ │
│  │  ┌─────────┐  process  ┌────────────┐  success  ┌─────────┐    │ │
│  │  │ Pending ├──────────>│ Processing ├──────────>│ Complete│    │ │
│  │  └────┬────┘           └─────┬──────┘           └─────────┘    │ │
│  │       ^                      │                                  │ │
│  │       │   retry (backoff)    │ fail (after max_retries)        │ │
│  │       └──────────────────────┤                                  │ │
│  │                              v                                  │ │
│  │                       ┌───────────┐                             │ │
│  │                       │Dead Letter│                             │ │
│  │                       │   Queue   │                             │ │
│  │                       └───────────┘                             │ │
│  │                                                                  │ │
│  │  Features:                                                      │ │
│  │  - Upsert semantics (latest event for path wins = debounce)    │ │
│  │  - Configurable debounce window                                 │ │
│  │  - Exponential backoff: base × 2^attempts                      │ │
│  │  - recover_stuck() for crashed items                            │ │
│  └──────────────────────────────────────────────────────────────────┘ │
│                                                                       │
│  Content Deduplication (index_state.rs):                              │
│  ┌──────────────────────────────────────────────────────────────────┐ │
│  │  BLAKE3 content hashing — skip re-indexing if content unchanged │ │
│  │                                                                  │ │
│  │  FileInfo: { path, size, mtime, content_hash: Option<String> }  │ │
│  │                                                                  │ │
│  │  Delta: 1. New file? → Index                                    │ │
│  │         2. Hash match? → SkipUnchangedContent                   │ │
│  │         3. Size/mtime differ? → Reindex                         │ │
│  └──────────────────────────────────────────────────────────────────┘ │
│                                                                       │
│  Cold Boot Reconciliation (index_state.rs):                           │
│  ┌──────────────────────────────────────────────────────────────────┐ │
│  │  reconcile(current_files) → Vec<ReconcileAction>:               │ │
│  │    Index          — new files                                   │ │
│  │    Reindex        — modified files                              │ │
│  │    SkipUnchanged  — content hash match                          │ │
│  │    RemoveOrphan   — files deleted while AX was stopped          │ │
│  └──────────────────────────────────────────────────────────────────┘ │
└═══════════════════════════════════════════════════════════════════════┘
```

---

## Search Architecture

All search (dense, sparse, hybrid) goes through Chroma. Both dense embeddings and
sparse BM25 vectors are computed locally and pushed to Chroma at index time. There is
no local vector storage — the pipeline generates vectors, Chroma stores and serves them.

```
┌═══════════════════════════════════════════════════════════════┐
│                    Search Architecture                        │
│                                                               │
│  Query: "how to configure S3 backend"                         │
│         │                                                     │
│         v                                                     │
│  ┌──────────────────┐                                         │
│  │  SearchEngine     │  (no local state — queries Chroma)     │
│  └───────┬──────────┘                                         │
│          │                                                    │
│     ┌────┴────┐                                               │
│     │         │                                               │
│     v         v                                               │
│  ┌───────┐ ┌───────┐                                          │
│  │ Dense │ │Sparse │                                          │
│  │Search │ │Search │                                          │
│  └───┬───┘ └───┬───┘                                          │
│      │         │                                              │
│      v         v                                              │
│  ┌───────┐ ┌───────┐                                          │
│  │Embed  │ │ BM25  │                                          │
│  │Query  │ │Encode │                                          │
│  │(Ollama│ │Query  │                                          │
│  │/OpenAI│ │       │                                          │
│  └───┬───┘ └───┬───┘                                          │
│      │         │                                              │
│      v         v                                              │
│  ┌───────┐ ┌───────┐                                          │
│  │Chroma │ │Chroma │  Both paths query Chroma:                │
│  │(dense │ │(sparse│  - Dense: query_by_embedding()           │
│  │ query)│ │ query)│  - Sparse: query_by_sparse_embedding()   │
│  └───┬───┘ └───┬───┘    (dot-product over stored BM25 vecs)  │
│      │         │                                              │
│      v         v                                              │
│  ┌───────┐ ┌───────┐                                          │
│  │Top K  │ │Top K  │                                          │
│  │Dense  │ │Sparse │                                          │
│  └───┬───┘ └───┬───┘                                          │
│      │         │                                              │
│      └────┬────┘                                              │
│           v                                                   │
│  ┌──────────────────┐                                         │
│  │  Weighted Fusion  │  Hybrid scoring:                       │
│  │                   │  score = 0.7×dense + 0.3×sparse        │
│  └────────┬─────────┘                                         │
│           v                                                   │
│  ┌──────────────────┐                                         │
│  │  Final Results    │                                         │
│  │                   │                                         │
│  │  SearchResult:    │                                         │
│  │  { chunk:         │                                         │
│  │    { source_path, │                                         │
│  │      content },   │                                         │
│  │    score,         │                                         │
│  │    dense_score,   │                                         │
│  │    sparse_score } │                                         │
│  └──────────────────┘                                         │
│                                                               │
│  Search Modes:                                                │
│  ┌────────────┬──────────────────────────────────────────┐    │
│  │ Dense      │ Embedding similarity only                │    │
│  │            │ "authentication" finds "login"           │    │
│  │ Sparse     │ BM25 keyword matching via Chroma         │    │
│  │            │ "S3Backend" finds exact matches           │    │
│  │ Hybrid     │ Dense + Sparse weighted fusion (default)  │    │
│  │            │ Best of both worlds                       │    │
│  └────────────┴──────────────────────────────────────────┘    │
│                                                               │
│  BM25 Scoring:                                                │
│  score(D, Q) = Σ IDF(qi) ×                                   │
│    (f(qi,D) × (k1+1)) / (f(qi,D) + k1 × (1-b + b×|D|/avgdl))│
│  where k1=1.5, b=0.75                                        │
│                                                               │
│  SparseEncoder State Persistence:                             │
│  - Encoder vocab/IDF state serialized to JSON via serde       │
│  - Stored in Chroma collection metadata on index completion   │
│  - Restored from Chroma on pipeline startup                   │
│  - No local index state beyond the operational work queue     │
└═══════════════════════════════════════════════════════════════┘
```

---

## Configuration System

YAML-based configuration with environment variable interpolation and smart defaults:

```
┌═══════════════════════════════════════════════════════════════┐
│                    Configuration Flow                          │
│                                                               │
│  ax.yaml                                                      │
│    │                                                          │
│    v                                                          │
│  ┌────────────────┐                                           │
│  │  Parse YAML    │  serde_yaml                               │
│  └───────┬────────┘                                           │
│          v                                                    │
│  ┌────────────────┐                                           │
│  │  Interpolate   │  ${VAR_NAME} → env value                  │
│  │  Environment   │  Missing vars → ConfigError               │
│  └───────┬────────┘                                           │
│          v                                                    │
│  ┌────────────────┐                                           │
│  │  Apply Smart   │  Infer backend from single backend        │
│  │  Defaults      │  Generate collection names                │
│  │                │  Set default mode (LocalIndexed)           │
│  └───────┬────────┘                                           │
│          v                                                    │
│  ┌────────────────┐                                           │
│  │  Validate      │  No duplicate/overlapping mounts          │
│  │                │  Backends exist for each mount             │
│  │                │  Paths start with /                        │
│  │                │  Read-only mounts can't have sync          │
│  │                │  Backend-specific validation               │
│  └───────┬────────┘                                           │
│          v                                                    │
│    VfsConfig (ready to use)                                   │
│                                                               │
├───────────────────────────────────────────────────────────────┤
│                                                               │
│  Example configuration:                                       │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │ name: my-workspace                                      │ │
│  │ version: 1                                              │ │
│  │                                                         │ │
│  │ backends:                                               │ │
│  │   local:                                                │ │
│  │     type: fs                                            │ │
│  │     root: ./data                                        │ │
│  │   cloud:                                                │ │
│  │     type: s3                                            │ │
│  │     bucket: ${S3_BUCKET}        ← env interpolation     │ │
│  │     region: us-east-1                                   │ │
│  │   db:                                                   │ │
│  │     type: postgres                                      │ │
│  │     connection_url: ${DATABASE_URL}  ← Secret type   │ │
│  │                                                         │ │
│  │ mounts:                                                 │ │
│  │   - path: /workspace                                    │ │
│  │     backend: local                                      │ │
│  │     mode: local_indexed                                 │ │
│  │   - path: /archive                                      │ │
│  │     backend: cloud                                      │ │
│  │     read_only: true                                     │ │
│  │     mode: pull_mirror                                   │ │
│  │                                                         │ │
│  │ defaults:                                               │ │
│  │   sync:                                                 │ │
│  │     write_mode: sync                                    │ │
│  │     conflict: last_write_wins                           │ │
│  │   chunk:                                                │ │
│  │     strategy: recursive                                 │ │
│  │     size: 512                                           │ │
│  │     overlap: 64                                         │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                               │
│  Config types with #[non_exhaustive] (forward-compatible):    │
│  MountMode, SearchMode, WriteMode, ConflictStrategy,          │
│  InvalidationStrategy, BackoffStrategy, ChunkStrategy,        │
│  ChunkGranularity, EmbeddingProvider, BackendConfig            │
└═══════════════════════════════════════════════════════════════┘
```

---

## Security & Credential Handling

```
┌═══════════════════════════════════════════════════════════════════════┐
│                   Security Architecture                               │
│                                                                       │
│  Secret Type (ax-config/types.rs):                                    │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  pub struct Secret(String);                                     │  │
│  │                                                                 │  │
│  │  Debug output:   Secret(***)    ← never leaks in logs           │  │
│  │  Display output: ***            ← never leaks in error msgs     │  │
│  │  expose() → &str               ← explicit opt-in to access     │  │
│  │  Serde: transparent             ← round-trips through JSON      │  │
│  │                                                                 │  │
│  │  Applied to:                                                    │  │
│  │  ┌──────────────────────────────────────────────────────────┐   │  │
│  │  │ PostgresBackendConfig.connection_url: Secret          │   │  │
│  │  │ ApiBackendConfig.auth_header: Option<Secret>             │   │  │
│  │  │ S3Config.access_key_id: Option<Secret>                   │   │  │
│  │  │ S3Config.secret_access_key: Option<Secret>               │   │  │
│  │  │ ServerConfig.api_key: Option<Secret>                     │   │  │
│  │  │ AppState.api_key: Option<Secret>                         │   │  │
│  │  └──────────────────────────────────────────────────────────┘   │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  SQL Injection Prevention (ax-backends/postgres.rs):                   │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  Table name validated against: ^[a-zA-Z_][a-zA-Z0-9_]*$        │  │
│  │  All queries use parameterized statements (sqlx params!)        │  │
│  │  Validation at construction time in PostgresBackend::new()      │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  REST API Authentication:                                             │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  Bearer token in Authorization header                           │  │
│  │  Health endpoints (/health, /health/live, /health/ready)        │  │
│  │  bypass authentication by design                                │  │
│  │  No api_key configured = open access (development mode)         │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  FFI Safety (ax-ffi):                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  All unsafe blocks documented with # Safety comments            │  │
│  │  Null pointer checks at FFI boundary                            │  │
│  │  catch_unwind at FFI boundary                                   │  │
│  └─────────────────────────────────────────────────────────────────┘  │
└═══════════════════════════════════════════════════════════════════════┘
```

---

## Error Handling & Resilience

```
┌═══════════════════════════════════════════════════════════════════════┐
│                     Error Type Hierarchy                              │
│                                                                       │
│  All cross-crate error enums are #[non_exhaustive] for forward       │
│  compatibility — downstream match statements require wildcard arms.   │
│                                                                       │
│  VfsError (ax-core, #[non_exhaustive])                                │
│  ├── NoMount(path)           No mount point for path                  │
│  ├── ReadOnly(path)          Write to read-only mount                 │
│  ├── NotFound(path)          File/directory not found                 │
│  ├── Backend(Box<Error>)     Wrapped backend error                    │
│  ├── Io(std::io::Error)      I/O error                                │
│  ├── Config(String)          Configuration error                      │
│  ├── Watch(String)           File watcher error                       │
│  └── Indexing(String)        Indexing pipeline error                   │
│                                                                       │
│  BackendError (ax-backends, #[non_exhaustive])                        │
│  ├── NotFound(path)          Resource not found                       │
│  ├── NotADirectory(path)     Expected directory                       │
│  ├── PathTraversal(path)     Traversal attack blocked                 │
│  ├── PermissionDenied(path)  Access denied                            │
│  ├── ConnectionFailed{..}    Connection failed (transient)            │
│  ├── Timeout{..}             Operation timed out (transient)          │
│  ├── Io(std::io::Error)      I/O error                                │
│  └── Other(String)           Backend-specific error                   │
│                                                                       │
│  ConfigError (ax-config)                                              │
│  ├── IoError                 File read error                          │
│  ├── YamlError               YAML parse error                         │
│  ├── MissingEnvVars(Vec)     Missing environment variables            │
│  ├── DuplicateMountPath      Duplicate mount                          │
│  ├── InvalidMountPath        Invalid path format                      │
│  ├── UndefinedBackend        Backend not found                        │
│  ├── OverlappingMountPaths   Overlapping mount paths                  │
│  └── InvalidConfig           General validation failure               │
│                                                                       │
│  FuseError (ax-fuse)                                                  │
│  ├── NotFound → ENOENT       ┐                                       │
│  ├── PermissionDenied → EACCES│  All map to libc errno               │
│  ├── IsDir → EISDIR          │  via to_errno()                        │
│  ├── NotDir → ENOTDIR        │                                       │
│  ├── Exists → EEXIST         │                                       │
│  ├── NotEmpty → ENOTEMPTY    │                                       │
│  ├── ReadOnly → EROFS        │                                       │
│  ├── Io → raw_os_error/EIO   │                                       │
│  └── Other → EIO             ┘                                       │
│                                                                       │
│  Error Propagation:                                                   │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  BackendError ────┐                                             │  │
│  │  ConfigError ─────┼──→ VfsError ──→ FuseError (errno)           │  │
│  │  std::io::Error ──┘         │                                   │  │
│  │                             ├──→ Python: IOError (via PyO3)     │  │
│  │                             ├──→ TypeScript: Error (via napi)   │  │
│  │                             ├──→ REST: HTTP status + JSON       │  │
│  │                             └──→ MCP: JSON-RPC error            │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  Resilience Patterns:                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  - No unwrap()/expect() in production code paths                │  │
│  │  - All errors propagated with ? or map_err()                    │  │
│  │  - Error swallowing replaced with tracing::warn!() logging      │  │
│  │  - unreachable!() replaced with proper error returns            │  │
│  │  - Transient errors (ConnectionFailed, Timeout) retried         │  │
│  │    with configurable exponential backoff                        │  │
│  └─────────────────────────────────────────────────────────────────┘  │
└═══════════════════════════════════════════════════════════════════════┘
```

---

## Observability

```
┌═══════════════════════════════════════════════════════════════════════┐
│                       Observability Stack                             │
│                                                                       │
│  Metrics (ax-core/metrics.rs):                                        │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  VfsMetrics: atomic counters, no-lock recording                 │  │
│  │                                                                 │  │
│  │  MetricsSnapshot:                                               │  │
│  │  ├── reads / read_bytes / read_errors / read_latency_avg/p99    │  │
│  │  ├── writes / write_bytes / write_errors / write_latency_avg/p99│  │
│  │  ├── deletes / delete_errors                                    │  │
│  │  └── lists / list_errors                                        │  │
│  │                                                                 │  │
│  │  Derived: read_error_rate(), total_operations(), total_errors() │  │
│  │  RAII: LatencyGuard auto-records elapsed time on drop           │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  HTTP Tracing (ax-server):                                            │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  tower-http TraceLayer: structured request/response logging     │  │
│  │  Includes: method, URI, status code, latency                    │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  Health Checks (ax-server/handlers.rs):                               │
│  ┌──────────────────┬──────────────────────────────────────────────┐  │
│  │ /health          │ Full check: {"status":"ok","version":"0.3.0"}│  │
│  │ /health/live     │ Liveness: 200 OK (process alive)             │  │
│  │ /health/ready    │ Readiness: VFS list("/") with 5s timeout     │  │
│  │                  │ Returns search_available status               │  │
│  │                  │ 503 if VFS unreachable or timed out           │  │
│  └──────────────────┴──────────────────────────────────────────────┘  │
│                                                                       │
│  WAL Status (ax wal status):                                          │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  Unapplied WAL entries, outbox pending/processing/failed counts │  │
│  │  Failed entry details: id, operation, path, attempts, error     │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  Cache Stats (via /status endpoint or VFS API):                       │
│  ┌─────────────────────────────────────────────────────────────────┐  │
│  │  hits, misses, hit_rate, entries, size, evictions, expirations  │  │
│  └─────────────────────────────────────────────────────────────────┘  │
└═══════════════════════════════════════════════════════════════════════┘
```

---

## AI Tool Generation

Generate tool definitions for AI assistants in multiple formats:

```
┌═══════════════════════════════════════════════════════════════┐
│                   Tool Generation Flow                        │
│                                                               │
│  VfsConfig                                                    │
│      │                                                        │
│      v                                                        │
│  ┌──────────────────┐                                         │
│  │  generate_tools  │  Analyze config:                        │
│  │                  │  - Mount paths and permissions           │
│  │                  │  - Read-only mounts?                     │
│  │                  │  - Indexing enabled?                     │
│  └────────┬─────────┘                                         │
│           │                                                   │
│           v                                                   │
│  ┌──────────────────┐                                         │
│  │ Vec<ToolDef>     │  vfs_read, vfs_write, vfs_list,         │
│  │                  │  vfs_delete, vfs_search, vfs_mounts     │
│  └────────┬─────────┘                                         │
│           │                                                   │
│      ┌────┼────┐                                              │
│      v    v    v                                              │
│    JSON  MCP  OpenAI                                          │
│                                                               │
│  Output Format Examples:                                      │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ JSON:   {"tools":[{"name":"vfs_read",                 │   │
│  │          "parameters":[{"name":"path","type":"string", │   │
│  │          "required":true}]}]}                          │   │
│  ├────────────────────────────────────────────────────────┤   │
│  │ MCP:    {"tools":[{"name":"vfs_read",                 │   │
│  │          "input_schema":{"type":"object",              │   │
│  │          "properties":{"path":{"type":"string"}}}}]}   │   │
│  ├────────────────────────────────────────────────────────┤   │
│  │ OpenAI: {"tools":[{"type":"function",                 │   │
│  │          "function":{"name":"vfs_read",               │   │
│  │          "parameters":{"type":"object",...}}}]}        │   │
│  └────────────────────────────────────────────────────────┘   │
└═══════════════════════════════════════════════════════════════┘
```

---

## Language Bindings

```
┌═══════════════════════════════════════════════════════════════┐
│                     Language Bindings                          │
│                                                               │
│                       ┌───────────┐                           │
│                       │  ax-core  │                           │
│                       │  (Rust)   │                           │
│                       └─────┬─────┘                           │
│               ┌─────────────┼─────────────┐                   │
│               v                           v                   │
│      ┌────────────────┐          ┌────────────────┐           │
│      │   ax-ffi       │          │    ax-js       │           │
│      │   (PyO3)       │          │  (napi-rs)     │           │
│      └────────┬───────┘          └────────┬───────┘           │
│               v                           v                   │
│      ┌────────────────┐          ┌────────────────┐           │
│      │   Python       │          │  TypeScript    │           │
│      │  import ax     │          │  import {..}   │           │
│      └────────────────┘          └────────────────┘           │
│                                                               │
│  Python (PyO3):                                               │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  Classes: PyVfs, PyEntry                                 │ │
│  │  Methods: from_yaml, from_file, read, read_text,        │ │
│  │           write, write_text, append, append_text,        │ │
│  │           delete, list, exists, stat, tools              │ │
│  │  Build: maturin develop / maturin build --release       │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                               │
│  TypeScript (napi-rs):                                        │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  Classes: JsVfs, JsEntry                                 │ │
│  │  Methods: loadConfig, readText, writeText, list, tools   │ │
│  │  Build: npm install && npm run build                     │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                               │
│  Type Mapping:                                                │
│  ┌────────────────┬──────────────┬──────────────────────────┐ │
│  │ Rust           │ Python       │ TypeScript               │ │
│  ├────────────────┼──────────────┼──────────────────────────┤ │
│  │ Vec<u8>        │ bytes        │ Buffer                   │ │
│  │ String         │ str          │ string                   │ │
│  │ Option<T>      │ T | None     │ T | null                 │ │
│  │ Result<T, E>   │ T (raises)   │ T (throws)               │ │
│  │ Entry          │ PyEntry      │ JsEntry                  │ │
│  │ VfsError       │ IOError      │ Error                    │ │
│  └────────────────┴──────────────┴──────────────────────────┘ │
└═══════════════════════════════════════════════════════════════┘
```

---

## CLI Reference

27 subcommands organized by category:

```
┌═══════════════════════════════════════════════════════════════════════┐
│                         ax CLI (ax-cli)                               │
│                                                                       │
│  Usage: ax [--config <path>] <command>                                │
│                                                                       │
│  File Operations:                                                     │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ ls       │ List directory contents                            │    │
│  │ cat      │ Display file contents                              │    │
│  │ write    │ Write content to file                              │    │
│  │ append   │ Append content to file                             │    │
│  │ rm       │ Remove file or directory                           │    │
│  │ cp       │ Copy file                                          │    │
│  │ mv       │ Move/rename file                                   │    │
│  │ stat     │ Show file metadata                                 │    │
│  │ exists   │ Check if path exists (exit code 0/1)               │    │
│  │ tree     │ Show directory tree (--depth)                      │    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  Search & Query:                                                      │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ grep     │ Regex search in file contents                      │    │
│  │ find     │ Find files by name pattern                         │    │
│  │ search   │ Semantic search (--mode dense/sparse/hybrid)       │    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  Indexing:                                                            │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ index    │ Index files (--incremental, --force, --chunker)    │    │
│  │ index-   │ Show index status (files indexed, chunks, dates)   │    │
│  │  status  │                                                    │    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  Filesystem:                                                          │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ mount    │ Mount AX as FUSE filesystem (--foreground)         │    │
│  │ unmount  │ Unmount FUSE filesystem (--force)                  │    │
│  │ watch    │ Watch for changes (--poll, --auto-index, --webhook)│    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  Servers:                                                             │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ serve    │ Start REST API (--host, --port, --api-key)         │    │
│  │ mcp      │ Run MCP server over stdio                          │    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  Configuration:                                                       │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ config   │ Show effective configuration                       │    │
│  │ validate │ Validate configuration file                        │    │
│  │ migrate  │ Migrate config to current version                  │    │
│  │ status   │ Show VFS status (mounts, backends)                 │    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  WAL Management:                                                      │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ wal      │ Manage Write-Ahead Log                             │    │
│  │  check-  │  checkpoint: prune old applied entries + VACUUM    │    │
│  │   point  │                                                    │    │
│  │  status  │  Show WAL + outbox stats and failed entries        │    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  Utilities:                                                           │
│  ┌──────────┬────────────────────────────────────────────────────┐    │
│  │ tools    │ Generate AI tool definitions (--format json/mcp)   │    │
│  └──────────┴────────────────────────────────────────────────────┘    │
│                                                                       │
│  Config Discovery (priority order):                                   │
│  1. --config flag                                                     │
│  2. AX_CONFIG environment variable                                    │
│  3. ./ax.yaml in current directory                                    │
│  4. ~/.config/ax/config.yaml                                          │
└═══════════════════════════════════════════════════════════════════════┘
```

---

## Design Principles

```
┌═══════════════════════════════════════════════════════════════════════┐
│                                                                       │
│  1. UNIFIED INTERFACE                                                 │
│     One API for local files, cloud storage, databases, and vectors.   │
│     Clients don't know or care which backend stores their data.       │
│                                                                       │
│  2. MOUNT-BASED ORGANIZATION                                          │
│     Unix-like mount points for organizing heterogeneous backends.     │
│     Longest-prefix matching routes paths to the right backend.        │
│                                                                       │
│  3. PERFORMANCE BY DEFAULT                                            │
│     Lock-free caching (moka TinyLFU), async I/O (tokio), zero-copy   │
│     where possible. Write-back mode for latency-sensitive workloads.  │
│                                                                       │
│  4. CRASH SAFETY                                                      │
│     WAL + durable outbox for write-back sync. Persistent work queue   │
│     with retry and dead letter queue. Cold boot reconciliation.       │
│                                                                       │
│  5. SECURITY BY DEFAULT                                               │
│     Secret type redacts credentials in Debug/Display. SQL injection   │
│     prevention via parameterized queries + table name validation.     │
│     No unwrap()/expect() in production paths. FFI boundary safety.    │
│                                                                       │
│  6. TRANSPARENCY                                                      │
│     FUSE integration means Claude Code sees normal files. MCP server  │
│     speaks native protocol. REST API has OpenAPI spec. All interfaces │
│     look like "just files" to the consumer.                           │
│                                                                       │
│  7. AI-NATIVE DESIGN                                                  │
│     Tool generation (MCP, OpenAI, JSON). Virtual .search/ directory.  │
│     Semantic search via hybrid dense+sparse. LLM merge resolution.    │
│                                                                       │
│  8. FORWARD COMPATIBILITY                                             │
│     #[non_exhaustive] on all public enums (12 total). API versioning  │
│     with /v1/ prefix. pub(crate) for internal modules.                │
│                                                                       │
│  9. INCREMENTAL & RESILIENT                                           │
│     Content-hash dedup skips re-indexing unchanged files. Persistent  │
│     work queue survives crashes. Exponential backoff on retries.      │
│     Auto-checkpoint prunes WAL after 500 applied entries.             │
│                                                                       │
│  10. OBSERVABLE                                                       │
│      Structured HTTP tracing. Health endpoints (live/ready). Metrics  │
│      with atomic counters. WAL status reporting. Cache hit rates.     │
│                                                                       │
│  11. CROSS-PLATFORM                                                   │
│      Python bindings (PyO3), TypeScript bindings (napi-rs).           │
│                                                                       │
└═══════════════════════════════════════════════════════════════════════┘

Architecture at a glance:

┌═══════════════════════════════════════════════════════════════════════┐
│                                                                       │
│   ┌──────────────────────────────────────────────────────────────┐    │
│   │                      Entry Points                             │    │
│   │  CLI  Python  TypeScript  FUSE  MCP Server  REST API          │    │
│   └────────────────────────┬─────────────────────────────────────┘    │
│                            │                                          │
│   ┌────────────────────────v─────────────────────────────────────┐    │
│   │                       ax-core                                 │    │
│   │   VFS → Router → CachedBackend → SyncEngine + WAL            │    │
│   │   SearchEngine  WorkQueue  IndexState  Watcher  Metrics       │    │
│   └────────────────────────┬─────────────────────────────────────┘    │
│                            │                                          │
│   ┌──────────┬─────────────┬──────────────────────┐                  │
│   │ ax-mcp   │ ax-server   │     ax-indexing      │                  │
│   │ MCP/stdio│ REST/Axum   │     Chunk/Embed     │                  │
│   │ JSON-RPC │ /v1/ + /    │     BM25/Sparse     │                  │
│   │ 30s t/o  │ 60s t/o     │     AST/PDF/...     │                  │
│   └──────────┴─────────────┴──────────────────────┘                  │
│                            │                                          │
│   ┌────────────────────────v─────────────────────────────────────┐    │
│   │                     ax-backends                               │    │
│   │   FS  Memory  S3  PostgreSQL  Chroma                          │    │
│   │   (all credentials wrapped in Secret type)                    │    │
│   └──────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  Crates: 10 (ax-config, ax-core, ax-backends, ax-indexing, ax-fuse,  │
│               ax-mcp, ax-server, ax-cli, ax-js, ax-ffi)              │
│  Tests: 617+ across all crates (incl. integration suites)              │
│  Sync: 4 modes (None/WriteThrough/WriteBack/PullMirror)              │
│        4 profiles (LocalOnly/LocalFirst/RemoteFirst/RemoteOnly)       │
│  Search: Dense + BM25 sparse + hybrid RRF fusion                      │
│  API: MCP (stdio), REST (Axum, /v1/, OpenAPI 3.0.3), FUSE mount      │
│  Security: Secret credential type, SQL injection protection,          │
│            #[non_exhaustive] enums, no unwrap() in production         │
└═══════════════════════════════════════════════════════════════════════┘
```

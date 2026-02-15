# AX Project Overview

AX (Agentic Files) is a virtual filesystem that gives AI agents and automation a single, consistent interface to files — regardless of where those files actually live. Local disk, S3, PostgreSQL, WebDAV, SFTP, GCS, Azure Blob, or an in-memory store: AX mounts them all into one unified file tree and provides read, write, search, and management operations across all of them.

## Why AX Exists

AI agents need to work with files. They need to read context, write output, search for relevant information, and manage state across sessions. But in practice, files live in many places — local directories, cloud buckets, databases, network shares. Each has its own API, auth model, and quirks.

AX eliminates this fragmentation. An agent writes to `/workspace/notes.txt` and reads from `/knowledge/api-docs.md` without knowing or caring that one is a local directory and the other is an S3 bucket. The routing, caching, syncing, and searching are handled by AX.

## Core Concepts

### Backends

A backend is a storage system. AX supports nine:

| Backend | Type | Use Case |
|---------|------|----------|
| Local filesystem | `fs` | Local directories |
| Memory | `memory` | Testing, ephemeral data |
| S3 | `s3` | AWS, MinIO, R2, DigitalOcean Spaces |
| PostgreSQL | `postgres` | Database-backed file storage |
| Chroma | `chroma` | Vector database for semantic search |
| WebDAV | `webdav` | Network file shares (NAS, Nextcloud) |
| SFTP | `sftp` | SSH-based remote access |
| Google Cloud Storage | `gcs` | GCP object storage |
| Azure Blob Storage | `azure_blob` | Azure object storage |

Each backend implements the same async trait: `read`, `write`, `append`, `delete`, `list`, `exists`, `stat`, `rename`.

### Mounts

Mounts map virtual paths to backends, just like Unix mount points. A config file defines the mapping:

```yaml
name: my-workspace

backends:
  code:
    type: fs
    root: ./src
  docs:
    type: s3
    bucket: team-docs
    region: us-east-1

mounts:
  - path: /code
    backend: code
  - path: /docs
    backend: docs
    read_only: true
```

AX uses longest-prefix matching to route operations to the right backend. `/code/main.rs` goes to the local filesystem; `/docs/api.md` goes to S3.

### Caching

A lock-free LRU cache (powered by `moka`) sits between the VFS and backends. It supports configurable TTL, max entries, max size, and uses TinyLFU admission for high hit rates. Cache invalidation happens automatically on writes.

### Sync Modes

For remote backends, AX provides four sync strategies:

| Mode | Reads | Writes | When to Use |
|------|-------|--------|-------------|
| None | Direct to backend | Direct to backend | Simple, low-latency local backends |
| WriteThrough | Cache, fallback to backend | Write to both synchronously | Strong consistency for remote data |
| WriteBack | Cache, fallback to backend | Write to cache, flush async | Low-latency writes, eventual consistency |
| PullMirror | Cache, fallback to backend | Blocked (read-only) | Remote reference material |

A SQLite-based write-ahead log (WAL) provides crash recovery and reliable async flushing for WriteBack mode.

### Semantic Search

AX can index files for semantic search using dense vector embeddings and BM25 sparse encoding, stored in Chroma. Features include:

- **Chunking strategies**: fixed-size, recursive, semantic, and AST-aware (tree-sitter)
- **Embedding providers**: Ollama (local) and OpenAI
- **Search modes**: dense (vector similarity), sparse (BM25 keyword), and hybrid (weighted fusion via Reciprocal Rank Fusion)
- **Incremental indexing**: BLAKE3 content hashing detects changes; only modified files are re-indexed
- **Watch mode**: file system notifications trigger auto-reindexing via a SQLite-backed work queue with debounce, retry, and crash recovery

## How to Access AX

AX provides six ways to interact with the virtual filesystem:

### CLI (27 commands)

```bash
ax write /workspace/hello.txt "Hello, world!"
ax cat /workspace/hello.txt
ax ls /workspace
ax grep "TODO" --path /workspace --recursive
ax search "authentication flow" --limit 5
```

### FUSE Mount

Mount the VFS as a native filesystem. Any program — including Claude Code — can read and write files through the mount point with standard file operations.

```bash
ax mount ~/ax-mount
```

A virtual `/.search/` directory exposes semantic search results as symlinks.

### REST API

An Axum-based HTTP server with 14 endpoints, Bearer token auth, and an OpenAPI spec.

```bash
ax serve --port 19557 --api-key "secret"
```

### MCP Server

A JSON-RPC 2.0 server over stdio exposing 7 tools (`ax_read`, `ax_write`, `ax_ls`, `ax_stat`, `ax_delete`, `ax_grep`, `ax_search`) for direct integration with LLM tool-use frameworks.

```bash
ax mcp
```

### Python

```python
import ax
vfs = ax.load_config_file("ax.yaml")
vfs.write_text("/workspace/hello.txt", "Hello from Python!")
```

### TypeScript

```typescript
const { loadConfigFile } = require('ax-vfs');
const vfs = loadConfigFile('ax.yaml');
vfs.writeText('/workspace/hello.txt', 'Hello from TypeScript!');
```

## Architecture

AX is built as a Rust workspace with 10 crates:

```
ax-config          Configuration parsing, validation, env var interpolation
ax-backends        9 storage backend implementations behind a common async trait
ax-core            Cache, metrics, AI tool generation, ChromaStore wrapper
ax-indexing        Text chunking, embeddings, BM25 sparse encoding
ax-local           Incremental indexer, search engine, watch mode, work queue
ax-remote          VFS router, cached backend, sync engine, WAL
ax-fuse            FUSE filesystem (macOS/Linux via fuser, Windows stub via WinFsp)
ax-mcp             MCP server (JSON-RPC 2.0, 7 tools)
ax-server          REST API (Axum, 14 endpoints, Bearer auth, OpenAPI)
ax-cli             CLI (27 subcommands via clap)
```

Language bindings (`ax-ffi` for Python via PyO3, `ax-js` for TypeScript via napi-rs) are built separately.

### Request Flow

```
Client request (CLI / Python / REST / MCP / FUSE)
    |
    v
VFS --- Router (longest-prefix mount matching)
    |
    v
CachedBackend --- Cache (moka LRU, lock-free reads)
    |
    v
SyncEngine --- WAL (SQLite, crash recovery)
    |
    v
Backend (fs / s3 / postgres / webdav / sftp / gcs / azure / memory / chroma)
```

## Configuration

AX is configured with a YAML file. Config discovery order: `$AX_CONFIG` env var, then `./ax.yaml`, then `~/.config/ax/config.yaml`. All values support `${VAR_NAME}` environment variable interpolation. Credentials are wrapped in a `Secret` type that redacts them in logs and debug output.

AX infers smart defaults from mount paths:
- Paths containing "workspace", "code", or "src" get AST chunking and hybrid search
- Paths containing "memory" or "context" get recursive chunking and dense search
- Paths containing "scratch" or "tmp" disable indexing

## Current Status

**Version**: 0.3.0
**Tests**: 617 passing
**License**: MIT

All core features are complete and tested. Known limitations: Windows FUSE is stubbed (macOS/Linux work fully), vector search requires an external Chroma server, and the project has not undergone a security audit.

See [PROJECT_STATUS.md](PROJECT_STATUS.md) for the full roadmap.

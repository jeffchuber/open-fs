# AX Project Status

**Last Updated**: February 12, 2026

## Executive Summary

AX (Agentic Files) is a virtual filesystem designed for AI agents, providing unified access to multiple storage backends with caching, sync strategies, semantic search, and AI tool generation. The project has reached **v0.3.0** with incremental indexing and a Unix FUSE implementation.

```
Total Tests: 617 passing (+5 ignored for external services)
Crates: 8 production + language bindings
Backends: 5 (fs, memory, s3, postgres, chroma)
CLI Commands: 27
```

---

## Project Structure

```
ax/
├── crates/
│   ├── ax-config/      # Configuration parsing, validation, env interpolation
│   ├── ax-core/        # VFS, router, cache, sync engine, tools generation
│   ├── ax-backends/    # Storage backends (fs, memory, s3, postgres, chroma)
│   ├── ax-indexing/    # Text chunking, embeddings, BM25, hybrid search
│   ├── ax-fuse/        # FUSE filesystem (macOS/Linux)
│   ├── ax-mcp/         # MCP server (JSON-RPC 2.0 over stdio, 7 tools)
│   ├── ax-server/      # REST API server (Axum, 14 endpoints)
│   ├── ax-cli/         # Command-line interface (27 commands)
│   ├── ax-ffi/         # Python bindings (PyO3)
│   └── ax-js/          # TypeScript bindings (napi-rs)
├── docs/
│   ├── ARCHITECTURE.md           # Detailed architecture with diagrams
│   ├── GETTING_STARTED.md        # FUSE + Claude Code quick start
│   ├── GUIDE.md                  # Gentle introduction to AX
│   ├── USE_CASES.md              # AI agent use case patterns
│   ├── CLAUDE_CODE_INTEGRATION.md # FUSE integration guide
│   └── PROJECT_STATUS.md         # This document
└── examples/
    └── coding-agent/             # Example agent configuration
```

---

## Completion Status

### Core Components (100% Complete)

| Component | Status | Tests | Notes |
|-----------|--------|-------|-------|
| **ax-config** | Complete | 53 | YAML parsing, env interpolation, validation, migration |
| **ax-core VFS** | Complete | 9 | Mount routing, file operations |
| **ax-core Cache** | Complete | 7 | Lock-free moka cache with TTL, LRU eviction |
| **ax-core Sync** | Complete | 7 | WriteThrough, WriteBack, PullMirror modes |
| **ax-core CachedBackend** | Complete | 5 | Unified caching layer for any backend |
| **ax-core Tools** | Complete | 4 | MCP, OpenAI, JSON format generation |
| **ax-core Router** | Complete | 3 | Mount routing with cross-mount rename |
| **ax-core Error** | Complete | 7 | thiserror-based structured errors |
| **ax-core Metrics** | Complete | 4 | Operation, cache, and error metrics |
| **ax-core Pipeline** | Complete | 3 | Indexing pipeline, sparse+dense vectors to Chroma |
| **ax-core Watcher** | Complete | 3 | File change notifications via notify |
| **ax-core IndexState** | Complete | 11 | Persistent index state with delta computation |
| **ax-core Incremental** | Complete | 7 | Delta-based incremental indexing |
| **ax-core WorkQueue** | Complete | 4 | SQLite-backed persistent work queue |
| **ax-core Integration** | Complete | 30 | Claude Code patterns, sync modes, edge cases |

### Backends (100% Complete)

| Backend | Status | Tests | Features |
|---------|--------|-------|----------|
| **Filesystem** | Complete | 5 + conformance | Full POSIX ops, path traversal protection |
| **Memory** | Complete | 27 + conformance | Testing backend, stable mtimes, full API |
| **S3** | Complete | - | AWS + S3-compatible (MinIO, etc.) |
| **PostgreSQL** | Complete | - | Auto table creation, binary storage |
| **Chroma** | Complete | 4 | Vector DB: dense+sparse vectors, collection metadata |
| **Error types** | Complete | 9 | Transient error detection, retry classification |

### Indexing & Search (100% Complete)

| Component | Status | Tests | Notes |
|-----------|--------|-------|-------|
| **Chunkers** | Complete | 30 | Fixed, recursive, semantic strategies |
| **Embeddings** | Complete | 15 | Ollama, OpenAI, stub providers |
| **BM25 Sparse** | Complete | 22 | Keyword search with TF-IDF, serde round-trip |
| **Hybrid Search** | Complete | 10 | Dense + sparse weighted fusion via Chroma |

### CLI (100% Complete — 27 subcommands)

| Command | Status | Description |
|---------|--------|-------------|
| `ls` | Complete | List directory contents |
| `cat` | Complete | Display file contents |
| `write` | Complete | Write content to file |
| `append` | Complete | Append content to file |
| `rm` | Complete | Remove file or directory |
| `stat` | Complete | Show file metadata |
| `exists` | Complete | Check path existence |
| `cp` | Complete | Copy file |
| `mv` | Complete | Move/rename file |
| `tree` | Complete | Directory tree view |
| `find` | Complete | Find files by pattern |
| `grep` | Complete | Search file contents |
| `index` | Complete | Index files for search (`--incremental`, `--force`) |
| `index-status` | Complete | Show index state (files, chunks, last updated) |
| `search` | Complete | Semantic search |
| `watch` | Complete | Watch for changes with work-queue-backed indexing |
| `mount` | Complete | FUSE mount (ax-fuse) |
| `unmount` | Complete | FUSE unmount (macOS/Linux) |
| `serve` | Complete | Start REST API server (--host, --port, --api-key) |
| `mcp` | Complete | Run MCP server over stdio |
| `config` | Complete | Show configuration |
| `status` | Complete | VFS status and stats |
| `validate` | Complete | Validate configuration file |
| `migrate` | Complete | Migrate config to current version |
| `tools` | Complete | Generate AI tool definitions |
| `wal` | Complete | WAL management (checkpoint, status) |

### FUSE Integration (100% Complete — 151 unit + 34 integration tests)

| Feature | Status | Tests | Notes |
|---------|--------|-------|-------|
| Platform-neutral core (`common.rs`) | Complete | 28 | `AxFsCore` with all VFS interaction logic |
| Unix FUSE driver (`unix_fuse.rs`) | Complete | 6 | `fuser::Filesystem` impl, FsOpError→errno |
| Inode management | Complete | 42 | Path-to-inode mapping, concurrent access |
| Async bridge | Complete | 29 | Sync FUSE callbacks to async VFS |
| Virtual .search/ | Complete | 46 | Semantic search via filesystem |
| mount/unmount CLI | Complete | - | macOS/Linux lifecycle |
| Integration tests | Complete | 34 | Full lifecycle, concurrent access, edge cases |

### MCP Server (100% Complete — 29 unit + 6 integration tests)

| Component | Status | Notes |
|-----------|--------|-------|
| **JSON-RPC 2.0 protocol** | Complete | Stdio transport, request/response/notification |
| **Tool dispatch** | Complete | 7 tools: ax_read, ax_write, ax_ls, ax_stat, ax_delete, ax_grep, ax_search |
| **Per-tool timeout** | Complete | 30s timeout per tool call |
| **MCP handshake** | Complete | initialize, tools/list, tools/call, ping |

### REST API Server (100% Complete — 36 unit + 10 integration tests)

| Component | Status | Notes |
|-----------|--------|-------|
| **Axum router** | Complete | /v1/ versioned + legacy routes |
| **14 endpoints** | Complete | CRUD, search, grep, health, status, openapi |
| **Auth** | Complete | Bearer token, health endpoints bypass auth |
| **Middleware** | Complete | 50MB body limit, 60s timeout, structured tracing |
| **Graceful shutdown** | Complete | SIGINT + SIGTERM handling |

### Language Bindings (90% Complete)

| Binding | Status | Notes |
|---------|--------|-------|
| **Python (PyO3)** | Complete | Full API coverage |
| **TypeScript (napi-rs)** | Mostly Complete | Core operations working |

---

## Recent Work Completed (v0.3.0)

### Indexing & Search Pipeline

- **IndexState** — Persistent JSON tracking of file path → (size, mtime, chunks, indexed_at)
- **IncrementalIndexer** — Wraps pipeline + state for delta-based indexing (new/modified/deleted/unchanged)
- **CLI flags** — `ax index --incremental` (only changed files), `ax index --force` (clear state, full re-index)
- **ax index-status** — Show index state (files indexed, total chunks, last updated, recent files)
- **Sparse vectors to Chroma** — Both dense embeddings and BM25 sparse vectors are pushed to Chroma at index time via `SparseEmbedding` in `upsert()`
- **SparseEncoder persistence** — Vocab/IDF state serialized to JSON and stored in Chroma collection metadata; restored on pipeline startup
- **Watch mode** — Uses SQLite-backed work queue (`.ax_watch_queue.db`) with debounce, dedup, retry, and crash recovery via `recover_stuck()`
- **No local vector storage** — All vectors (dense and sparse) stored and queried from Chroma; `SearchEngine` has no in-memory sparse cache

### FUSE Updates

- **AxFsCore** — All VFS interaction logic in platform-independent struct with `do_*` helper methods
- **UnixFuse** — `fuser::Filesystem` impl delegating to AxFsCore, FsOpError → errno mapping
- **Unmount** — macOS (`umount`) and Linux (`fusermount -u`)

### Bug Fixes

- **MemoryBackend mtime stability** — Now stores `(Vec<u8>, DateTime<Utc>)` tuples so mtimes are stable across list/stat calls (was generating new `Utc::now()` each time)

---

## Test Coverage

```
Total: 617 tests passing (+5 ignored for external services)

By Crate:
├── ax-config:      58 unit + 5 migration integration tests
├── ax-core:       100+ unit + integration
├── ax-backends:     6+ tests + 2 conformance suites (+ignored for external services)
├── ax-indexing:    83 tests (+ignored for external services)
├── ax-mcp:         29 unit + 6 protocol integration tests
├── ax-server:      36 unit + 10 API integration tests
├── ax-cli:          2 integration test suites
├── ax-fuse:       185 tests (151 unit + 34 integration)
└── ax-remote:      43 unit + 8 VFS integration tests
```

### Running Tests

```bash
# All tests
cargo test --workspace

# Specific crate
cargo test -p ax-core

# Integration tests
cargo test -p ax-core --test claude_code_integration

# With all features
cargo test --workspace --features all-backends
```

---

## What's Working

### Verified Scenarios

1. **Local Development**
   - Mount local directories with caching
   - Full file operations (CRUD, rename, copy)
   - Pattern search with glob and grep

2. **Remote Storage**
   - PostgreSQL for structured file storage
   - Cache with configurable TTL

3. **Semantic Search**
   - Index files with multiple chunking strategies
   - Hybrid search (dense embeddings + BM25 sparse, both via Chroma)
   - Search via FUSE .search/ directory
   - Incremental indexing (only re-index changed files)
   - Watch mode with SQLite-backed work queue (debounce, retry, crash recovery)

4. **AI Integration**
   - Generate tool definitions (MCP, OpenAI, JSON)
   - FUSE mount for transparent Claude Code access
   - Python and TypeScript bindings

5. **Concurrent Access**
   - Lock-free cache reads
   - Safe concurrent writes with proper locking
   - Background sync with write-back mode

6. **Cross-Platform FUSE**
   - macOS/Linux via fuser

---

## Known Limitations

### Current Limitations

1. **FUSE Platform Support**
   - macOS: Requires macFUSE (user must allow kernel extension)
   - Linux: Requires libfuse3

2. **S3 Credentials**
   - Uses AWS default credential chain
   - No explicit credential configuration in ax.yaml yet

3. **Vector Search**
   - Requires external Chroma server
   - No embedded vector index option

4. **Remote Backend Tests**
   - Unit tests cover config/path logic; integration tests are `#[ignore]`

### Not Production Ready

- No comprehensive security audit
- No rate limiting or quota management
- No multi-tenant isolation

---

## Next Steps

### Completed Milestones

- **v0.1.0** — Core VFS, 5 backends, caching, sync, search, CLI, FUSE, language bindings
- **v0.2.0** — Error handling (thiserror), config validation, watch mode, migration tool

### Next (v0.4.0)

1. **Production Hardening** (partially complete)
   - ~~Backend conformance test suite~~ (done — fs + memory)
   - ~~Multi-mount VFS integration tests~~ (done — 8 tests)
   - ~~Server API integration tests~~ (done — 10 HTTP tests with real Axum server)
   - ~~MCP protocol integration tests~~ (done — 6 JSON-RPC session tests)
   - ~~Config migration integration tests~~ (done — 5 tests)
   - REST API integration tests for binary round-trips (remaining)
   - Metrics exporter (Prometheus/OTel)
   - Deployment templates and `ax doctor` CLI

### Long Term (v1.0.0)

2. **Multi-Tenant Support**
   - Namespace isolation
   - Per-tenant quotas
   - Access control lists

3. **Distributed Sync**
   - Conflict resolution
   - Sync status dashboard

4. **Production Hardening**
   - Security audit
   - Rate limiting and quota management
   - Retry logic for transient backend failures

---

## Architecture Highlights

### Cache Layer (moka)

The cache was recently refactored from `RwLock<HashMap>` to `moka::future::Cache`:

```rust
// Before: Every read required a write lock to update LRU
let mut cache = self.cache.write().await;
if let Some(entry) = cache.get_mut(key) {
    entry.touch(); // Update access time
    return Some(entry.data.clone());
}

// After: Lock-free reads with automatic LRU management
match self.cache.get(key).await {
    Some(value) => {
        self.stats.hit();
        Some(value)
    }
    None => {
        self.stats.miss();
        None
    }
}
```

### Sync Modes

```
┌──────────────┬──────────────────┬───────────────────────────────┐
│    Mode      │   Read Path      │   Write Path                  │
├──────────────┼──────────────────┼───────────────────────────────┤
│   None       │ Backend direct   │ Backend direct                │
│ WriteThrough │ Cache → Backend  │ Cache + Backend (sync)        │
│ WriteBack    │ Cache → Backend  │ Cache, then async flush       │
│ PullMirror   │ Cache → Backend  │ READ-ONLY                     │
└──────────────┴──────────────────┴───────────────────────────────┘
```

### FUSE Integration

```
┌─────────────────────────────────────────────────────────────┐
│                     Claude Code                              │
│   Read, Write, Glob, Grep — standard file operations        │
└───────────────────────────┬─────────────────────────────────┘
                            │
               ┌────────────▼────────────┐
               │  FUSE Mount  ~/ax-mount │
               │        UnixFuse         │
               └────────────┬────────────┘
                            │
               ┌────────────▼────────────┐
               │    AxFsCore (common)    │
               │  inodes + search_dir    │
               └────────────┬────────────┘
                            │
               ┌────────────▼────────────┐
               │         AX VFS          │
               │  Routing + Cache + Sync │
               └────────────┬────────────┘
                            │
  ┌──────┬──────┬──────┬────┼────┬──────┬──────┬──────┐
  ▼      ▼      ▼      ▼   ▼    ▼      ▼      ▼      ▼
```

---

## Quick Reference

### Build

```bash
# Debug build
cargo build --workspace

# Release build
cargo build --workspace --release

# With all features
cargo build --workspace --features all-backends
```

### Test

```bash
# All tests
cargo test --workspace

# With output
cargo test --workspace -- --nocapture

# Single test
cargo test -p ax-core test_cache_put_get
```

### Install

```bash
# Install CLI
cargo install --path crates/ax-cli

# Run CLI
ax --help
```

### Example Configuration

```yaml
name: my-workspace

backends:
  local:
    type: fs
    root: ./data
  s3:
    type: s3
    bucket: my-bucket
    region: us-east-1

mounts:
  - path: /workspace
    backend: local
    cache:
      enabled: true
      ttl_seconds: 300
  - path: /remote
    backend: s3
    sync:
      mode: write_through
```

---

## Contributing

1. Fork the repository
2. Run tests: `cargo test --workspace`
3. Submit a pull request

See [ARCHITECTURE.md](./ARCHITECTURE.md) for detailed design documentation.

---

## License

MIT

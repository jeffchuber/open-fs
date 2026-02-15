# AX Architecture (Codex)

This document describes the architecture of the AX virtual filesystem (VFS), how its major components interact, and what is complete vs. stubbed or optional. It reflects the current workspace state and the focus on non‑FUSE functionality.

**Summary**
AX exposes a unified VFS that can mount multiple storage backends (local filesystem, memory, S3, Postgres, Chroma). The VFS supports caching, sync modes (write-through, write-back, pull mirror), indexing/search via Chroma, FUSE mounting, and a REST API + CLI + MCP server for automation. It is designed around clean traits and strong config validation to keep behavior predictable in production.

**High-Level Components**
- `ax-config`: YAML configuration, env interpolation, validation, migration.
- `ax-backends`: Storage backends (fs, memory, s3, postgres, chroma).
- `ax-core`: VFS, caching, sync/WAL, indexing pipeline, search utilities, watch engine.
- `ax-indexing`: Chunkers, embedding adapters, content hashing, extractors.
- `ax-server`: REST API and OpenAPI schema.
- `ax-cli`: Command-line operations over the VFS.
- `ax-mcp`: MCP server for tool-based AI integrations.

**Core Data Flow**
```text
User/Client
  |
  |  CLI / REST API / MCP
  v
ax-core::Vfs
  |
  |-- mount router -> per-mount backend adapter
  |        |
  |        +-- cache (optional, per mount)
  |        |
  |        +-- sync engine (optional, per mount)
  |        |
  |        +-- base backend (fs/memory/s3/postgres)
  |
  +-- indexing/search (optional, Chroma + embedders)
```

**VFS + Backends**
The VFS aggregates multiple mounts and routes all reads/writes/renames/etc. to the appropriate backend. Each mount can enable caching and/or sync behavior. Sync uses a WAL + outbox to guarantee durability and eventual persistence when configured for write‑back or mirror modes.

**Sync Modes**
- `WriteThrough`: write to cache (if enabled) and immediately persist to backend.
- `WriteBack`: write to cache and enqueue WAL/outbox for async persistence.
- `PullMirror`: read‑only view; local writes are blocked.

**Caching**
Caching is optional per mount. The cache enforces a maximum size and entry count and tracks hit/miss stats. It is integrated with sync modes so write‑back persists from the cache.

**Indexing + Search**
Indexing splits content into chunks (fixed, recursive, semantic) and embeds for vector search. Search uses Chroma as the vector store. There is a sparse search engine available for hybrid scoring, but the primary vector store is Chroma.

**Server + CLI + MCP**
The REST API provides file ops, search, and status. It supports base64 encoding for binary content. The CLI mirrors file ops and indexing features. MCP exposes a tool interface for AI assistants.

**Operational Diagram**
```text
          +------------------+
          |  Config (YAML)   |
          +------------------+
                    |
                    v
          +------------------+
          |  Vfs Builder     |
          +------------------+
                    |
     +--------------+--------------+
     |                             |
     v                             v
Mount A                        Mount B
(cache + sync)                 (no cache)
     |                             |
     v                             v
Base backend                   Base backend
fs / s3 / pg                   memory / fs
```

**Complete vs. Stubbed**

Complete and production‑ready:
- VFS routing, caching, and sync/WAL logic.
- Backends: `fs`, `memory`, `s3`, `postgres`, `chroma`.
- Chroma integration for vector storage.
- REST API + OpenAPI schema, including base64 content support.
- CLI (27 subcommands) and MCP server for automation.
- FUSE mount (macOS/Linux via fuser).
- Chunkers, content hash, and extractors.

External service dependencies:
- Embedders requiring external services (`openai`, `ollama`) are integration points; tests are ignored unless those services are available.
- Remote backend tests (S3, PostgreSQL, Chroma) require live services; unit tests are `#[ignore]`.
- `ax-ffi` and `ax-js` are excluded from the default workspace build.

**Diagrams: Sync + WAL**
```text
Write (client) -> cache -> WAL/outbox -> background sync -> backend
                    |                           |
                    +----- recover on restart ---+
```

**Config & Validation**
Config is strongly validated and supports env interpolation. Unknown fields are rejected in most sections to prevent silent misconfiguration. Postgres uses a `connection_url` and optional `table_name`. S3 supports explicit credentials or environment-based credentials.

**Testing Coverage**
The following are covered by unit and integration tests:
- Backends: fs/memory/s3/postgres behavior, path safety, normalization. Backend conformance suite validates all operations against fs and memory backends.
- VFS: routing, cache integration, sync and WAL recovery. Multi-mount VFS integration tests cover routing isolation, read-only enforcement, cross-mount rename, and mixed backend types.
- Indexing: chunker behavior, hashing, extractors.
- REST API: request/response schema and endpoints. HTTP integration tests spin up a real Axum server on an ephemeral port and exercise the full middleware stack (auth, health, CRUD, versioned routes).
- MCP: protocol serialization and tool behaviors. Protocol integration tests simulate full JSON-RPC sessions (initialize, tools/list, tool calls, error handling, compliance).
- CLI: integration tests for core file ops and search utilities.
- Config: migration pipeline tested with real YAML (v0.1→v0.2 migration, field preservation, validation, unknown version rejection).

**Known Operational Notes**
- File watch uses native OS notifications by default. Polling can be forced with `AX_WATCH_POLL_INTERVAL_MS` if native events are unreliable (e.g., CI or networked volumes).
- Chroma/OpenAI/Ollama tests are skipped by default because they require external services.

**Future Work (Optional)**
- Add REST API integration tests for binary data round‑trips over base64.
- Metrics exporter (Prometheus/OTel) and deployment templates.

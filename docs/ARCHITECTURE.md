# AX Architecture Guide

A comprehensive guide to the AX virtual filesystem architecture with ASCII diagrams.

## Table of Contents

1. [Overview](#overview)
2. [High-Level Architecture](#high-level-architecture)
3. [Crate Structure](#crate-structure)
4. [Request Flow](#request-flow)
5. [Backend Abstraction](#backend-abstraction)
6. [Mount-Based Routing](#mount-based-routing)
7. [Caching Layer](#caching-layer)
8. [Sync Engine](#sync-engine)
9. [Indexing Pipeline](#indexing-pipeline)
10. [AI Tool Generation](#ai-tool-generation)
11. [Language Bindings](#language-bindings)
12. [Configuration System](#configuration-system)

---

## Overview

AX (Agentic Files) is a virtual filesystem designed for AI agents and automation.
It provides a unified interface to multiple storage backends with support for
caching, syncing, semantic search, and tool generation for AI assistants.

```
+------------------------------------------------------------------+
|                         AX VIRTUAL FILESYSTEM                     |
|                                                                   |
|  "One API to rule them all - local files, S3, databases, vectors" |
+------------------------------------------------------------------+
|                                                                   |
|   +-------------+  +-------------+  +-------------+               |
|   |   Python    |  | TypeScript  |  |     CLI     |               |
|   |  Bindings   |  |  Bindings   |  |   (ax cmd)  |               |
|   +------+------+  +------+------+  +------+------+               |
|          |                |                |                      |
|          +----------------+----------------+                      |
|                           |                                       |
|                    +------v------+                                |
|                    |   ax-core   |                                |
|                    |     VFS     |                                |
|                    +------+------+                                |
|                           |                                       |
|          +----------------+----------------+                      |
|          |                |                |                      |
|   +------v------+  +------v------+  +------v------+               |
|   | Local Files |  |     S3      |  |  PostgreSQL |               |
|   +-------------+  +-------------+  +-------------+               |
|                                                                   |
+------------------------------------------------------------------+
```

---

## High-Level Architecture

```
                              +------------------+
                              |    YAML Config   |
                              |    (ax.yaml)     |
                              +--------+---------+
                                       |
                                       v
+------------------------------------------------------------------------------+
|                                   ax-config                                   |
|  +----------------+  +------------------+  +------------------+               |
|  |   Parsing &    |  |   Environment    |  |   Validation    |               |
|  |    Types       |  |   Interpolation  |  |   & Defaults    |               |
|  +----------------+  +------------------+  +------------------+               |
+------------------------------------------------------------------------------+
                                       |
                                       v
+------------------------------------------------------------------------------+
|                                    ax-core                                    |
|                                                                               |
|  +----------+    +----------+    +----------+    +----------+                |
|  |   VFS    |--->|  Router  |--->|  Cache   |--->|   Sync   |                |
|  +----------+    +----------+    +----------+    +----------+                |
|       |                                                                       |
|       v                                                                       |
|  +----------+    +----------+    +----------+                                |
|  | Metrics  |    |  Tools   |    | Pipeline |                                |
|  +----------+    +----------+    +----------+                                |
+------------------------------------------------------------------------------+
                                       |
                                       v
+------------------------------------------------------------------------------+
|                                 ax-backends                                   |
|                                                                               |
|  +----------+    +----------+    +----------+    +----------+                |
|  |    FS    |    |    S3    |    | Postgres |    |  Chroma  |                |
|  +----------+    +----------+    +----------+    +----------+                |
+------------------------------------------------------------------------------+
                                       |
                                       v
+------------------------------------------------------------------------------+
|                                ax-indexing                                    |
|                                                                               |
|  +------------+    +------------+    +------------+    +------------+        |
|  | Extractors |    |  Chunkers  |    | Embedders  |    |   Sparse   |        |
|  | (txt, pdf) |    | (semantic) |    | (ollama)   |    |   (BM25)   |        |
|  +------------+    +------------+    +------------+    +------------+        |
+------------------------------------------------------------------------------+
```

---

## Crate Structure

```
ax/
|
+-- Cargo.toml                 # Workspace definition
|
+-- crates/
|   |
|   +-- ax-config/             # Configuration parsing & validation
|   |   +-- src/
|   |   |   +-- lib.rs         # Public API: VfsConfig, from_yaml, from_file
|   |   |   +-- types.rs       # Config structs: BackendConfig, MountConfig
|   |   |   +-- env.rs         # Environment variable interpolation
|   |   |   +-- validation.rs  # Config validation rules
|   |   |   +-- defaults.rs    # Smart defaults application
|   |   +-- Cargo.toml
|   |
|   +-- ax-core/               # Core VFS implementation
|   |   +-- src/
|   |   |   +-- lib.rs         # Public API exports
|   |   |   +-- vfs.rs         # Main VFS struct & operations
|   |   |   +-- router.rs      # Mount-based path routing
|   |   |   +-- cache.rs       # LRU cache with TTL
|   |   |   +-- cached_backend.rs  # Cache wrapper for backends
|   |   |   +-- sync.rs        # Write-through/write-back sync
|   |   |   +-- metrics.rs     # Operation metrics & latency
|   |   |   +-- tools.rs       # AI tool definition generation
|   |   |   +-- pipeline.rs    # Indexing pipeline orchestration
|   |   |   +-- search.rs      # Hybrid search (dense + sparse)
|   |   |   +-- traits.rs      # Backend trait definition
|   |   |   +-- error.rs       # VFS error types
|   |   +-- Cargo.toml
|   |
|   +-- ax-backends/           # Storage backend implementations
|   |   +-- src/
|   |   |   +-- lib.rs         # Public API exports
|   |   |   +-- traits.rs      # Backend trait (read, write, list, etc.)
|   |   |   +-- fs.rs          # Local filesystem backend
|   |   |   +-- memory.rs      # In-memory backend (testing)
|   |   |   +-- s3.rs          # S3-compatible storage
|   |   |   +-- postgres.rs    # PostgreSQL storage
|   |   |   +-- chroma.rs      # Chroma vector database
|   |   |   +-- error.rs       # Backend error types
|   |   +-- Cargo.toml
|   |
|   +-- ax-indexing/           # Text processing & search
|   |   +-- src/
|   |   |   +-- lib.rs         # Public API exports
|   |   |   +-- types.rs       # Chunk, SparseVector, etc.
|   |   |   +-- chunkers/      # Text chunking strategies
|   |   |   |   +-- mod.rs
|   |   |   |   +-- fixed.rs       # Fixed-size chunks
|   |   |   |   +-- recursive.rs   # Recursive splitting
|   |   |   |   +-- semantic.rs    # Semantic boundaries
|   |   |   |   +-- ast.rs         # AST-based (code)
|   |   |   +-- embedders/     # Embedding providers
|   |   |   |   +-- mod.rs
|   |   |   |   +-- ollama.rs      # Ollama local models
|   |   |   |   +-- openai.rs      # OpenAI API
|   |   |   |   +-- stub.rs        # Testing stub
|   |   |   +-- extractors/    # Text extraction
|   |   |   |   +-- mod.rs
|   |   |   |   +-- plaintext.rs   # Plain text files
|   |   |   |   +-- pdf.rs         # PDF extraction
|   |   |   +-- sparse.rs      # BM25 sparse encoding
|   |   +-- Cargo.toml
|   |
|   +-- ax-cli/                # Command-line interface
|   |   +-- src/
|   |   |   +-- main.rs        # CLI entry point
|   |   |   +-- commands/      # Subcommands
|   |   |       +-- mod.rs
|   |   |       +-- cat.rs, ls.rs, write.rs, rm.rs, ...
|   |   +-- tests/
|   |   |   +-- cli_integration.rs  # Integration tests
|   |   +-- Cargo.toml
|   |
|   +-- ax-ffi/                # Python bindings (PyO3)
|   |   +-- src/
|   |   |   +-- lib.rs         # PyO3 module definition
|   |   +-- python/
|   |   |   +-- ax/
|   |   |       +-- __init__.py
|   |   +-- Cargo.toml
|   |
|   +-- ax-js/                 # TypeScript bindings (napi-rs)
|       +-- src/
|       |   +-- lib.rs         # napi module definition
|       +-- index.d.ts         # TypeScript type definitions
|       +-- package.json
|       +-- Cargo.toml
|
+-- configs/                   # Example configurations
|   +-- minimal.yaml
|   +-- multi-mount.yaml
|   +-- env-vars.yaml
|
+-- examples/                  # Usage examples
|   +-- python_example.py
|   +-- typescript_example.ts
|
+-- docs/                      # Documentation
    +-- ARCHITECTURE.md        # This file
```

---

## Request Flow

### Read Operation

```
   Client Request: vfs.read("/workspace/docs/readme.md")
         |
         v
+------------------+
|       VFS        |
|                  |
|  1. Validate     |
|     path format  |
+--------+---------+
         |
         v
+------------------+
|     Router       |
|                  |
|  2. Find mount   |
|     /workspace   |
|                  |
|  3. Strip prefix |
|     -> docs/     |
|        readme.md |
+--------+---------+
         |
         v
+------------------+
|   CachedBackend  |  <-- Optional caching layer
|                  |
|  4. Check cache  |-----> HIT: Return cached content
|     for key      |
|                  |
|  5. MISS: Forward|
|     to backend   |
+--------+---------+
         |
         v
+------------------+
|    FsBackend     |
|                  |
|  6. Read file    |
|     from disk    |
|                  |
|  7. Return bytes |
+--------+---------+
         |
         v
+------------------+
|   CachedBackend  |
|                  |
|  8. Store in     |
|     cache        |
+--------+---------+
         |
         v
   Return to Client
```

### Write Operation

```
   Client Request: vfs.write("/data/file.txt", content)
         |
         v
+------------------+
|       VFS        |
|                  |
|  1. Validate     |
|     path format  |
+--------+---------+
         |
         v
+------------------+
|     Router       |
|                  |
|  2. Find mount   |
|     /data        |
|                  |
|  3. Check if     |-----> READ_ONLY: Return Error
|     read_only    |
|                  |
|  4. Strip prefix |
|     -> file.txt  |
+--------+---------+
         |
         v
+------------------+
|    SyncEngine    |  <-- Optional sync layer
|                  |
|  WRITE_THROUGH:  |
|  5a. Write to    |
|      backend     |
|      immediately |
|                  |
|  WRITE_BACK:     |
|  5b. Queue write |
|      for later   |
+--------+---------+
         |
         v
+------------------+
|    FsBackend     |
|                  |
|  6. Write file   |
|     to disk      |
|                  |
|  7. Create parent|
|     directories  |
+--------+---------+
         |
         v
+------------------+
|   CachedBackend  |
|                  |
|  8. Invalidate   |
|     cache entry  |
+--------+---------+
         |
         v
   Return Success
```

---

## Backend Abstraction

All backends implement a common trait for uniform access:

```
+------------------------------------------------------------------+
|                        Backend Trait                              |
+------------------------------------------------------------------+
|                                                                   |
|   async fn read(&self, path: &str) -> Result<Vec<u8>>            |
|   async fn write(&self, path: &str, content: &[u8]) -> Result<()>|
|   async fn append(&self, path: &str, content: &[u8]) -> Result<()>|
|   async fn delete(&self, path: &str) -> Result<()>               |
|   async fn list(&self, path: &str) -> Result<Vec<Entry>>         |
|   async fn exists(&self, path: &str) -> Result<bool>             |
|   async fn stat(&self, path: &str) -> Result<Entry>              |
|                                                                   |
+------------------------------------------------------------------+
         ^              ^              ^              ^
         |              |              |              |
+--------+--+    +------+----+   +-----+-----+   +----+------+
| FsBackend |    | S3Backend |   | PostgresB |   | ChromaB   |
+-----------+    +-----------+   +-----------+   +-----------+
|           |    |           |   |           |   |           |
| Local     |    | S3-compat |   | PostgreSQL|   | Vector DB |
| filesystem|    | storage   |   | with blob |   | with      |
| via       |    | (AWS, R2, |   | storage   |   | embeddings|
| std::fs   |    | MinIO)    |   |           |   |           |
+-----------+    +-----------+   +-----------+   +-----------+

Entry Structure:
+------------------+
|      Entry       |
+------------------+
| path: String     |  Full path to the entry
| name: String     |  Filename or directory name
| is_dir: bool     |  True if directory
| size: Option<u64>|  File size in bytes
| modified: Option |  Last modification time
|   <DateTime>     |
+------------------+
```

---

## Mount-Based Routing

Mount points provide Unix-like path organization across backends:

```
Configuration:
+-----------------------------------------+
|  backends:                              |
|    local:                               |
|      type: fs                           |
|      root: ./data                       |
|    remote:                              |
|      type: s3                           |
|      bucket: my-bucket                  |
|                                         |
|  mounts:                                |
|    - path: /workspace                   |
|      backend: local                     |
|    - path: /archive                     |
|      backend: remote                    |
|      read_only: true                    |
+-----------------------------------------+

Virtual Filesystem View:
/
+-- workspace/              --> FsBackend (./data)
|   +-- src/
|   |   +-- main.rs
|   |   +-- lib.rs
|   +-- Cargo.toml
|
+-- archive/                --> S3Backend (my-bucket) [READ-ONLY]
    +-- 2024/
    |   +-- backup-01.tar
    +-- 2023/
        +-- backup-12.tar

Path Resolution:
+-----------------------------------+
|  Input: /workspace/src/main.rs    |
|                                   |
|  1. Match mount: /workspace       |
|  2. Backend: local (FsBackend)    |
|  3. Relative: src/main.rs         |
|  4. Full path: ./data/src/main.rs |
+-----------------------------------+

|  Input: /archive/2024/backup.tar  |
|                                   |
|  1. Match mount: /archive         |
|  2. Backend: remote (S3Backend)   |
|  3. Relative: 2024/backup.tar     |
|  4. S3 key: 2024/backup.tar       |
+-----------------------------------+

Longest Prefix Matching:
+-------------------------------------------+
|  Mounts:                                  |
|    /data           -> BackendA            |
|    /data/special   -> BackendB            |
|                                           |
|  Path: /data/special/file.txt             |
|                                           |
|  Result: BackendB (longer prefix wins)    |
+-------------------------------------------+
```

---

## Caching Layer

LRU cache with TTL support for improved read performance:

```
+------------------------------------------------------------------+
|                        Cache Architecture                         |
+------------------------------------------------------------------+

                    +------------------------+
                    |      CachedBackend     |
                    +------------------------+
                    |                        |
  read(path) ------>|  1. Generate cache key |
                    |     hash(path)         |
                    |                        |
                    |  2. Check LRU cache    |
                    |        |               |
                    |   +----v----+          |
                    |   |  Cache  |          |
                    |   | Lookup  |          |
                    |   +----+----+          |
                    |        |               |
                    |   HIT  |  MISS         |
                    |   |    |    |          |
                    |   v    |    v          |
                    | Return |  Forward to   |
                    | cached |  backend      |
                    | data   |    |          |
                    |        |    v          |
                    |        | Store result  |
                    |        | in cache      |
                    |        |    |          |
                    |        +----+          |
                    |             |          |
                    +-------------+----------+
                                  |
                                  v
                            Return data

Cache Entry Structure:
+----------------------------------+
|          CacheEntry              |
+----------------------------------+
| data: Vec<u8>    # File content  |
| inserted_at:     # For TTL       |
|   Instant        # expiration    |
| size: usize      # For eviction  |
+----------------------------------+

Cache Configuration:
+----------------------------------+
|         CacheConfig              |
+----------------------------------+
| max_entries: 1000  # Max items   |
| max_size_bytes:    # Max total   |
|   100MB            # size        |
| ttl_seconds: 300   # 5 min TTL   |
| enabled: true      # Toggle      |
+----------------------------------+

Eviction Strategy (LRU):
+---------------------------------------------+
|  Cache Full? Need to insert new entry?      |
|                                             |
|  1. Check TTL - remove expired entries      |
|  2. If still full, remove Least Recently    |
|     Used entries until space available      |
|                                             |
|  Access Order (most recent first):          |
|  [D] -> [B] -> [A] -> [C]                   |
|                        ^^^                  |
|                    Evict first              |
+---------------------------------------------+
```

---

## Sync Engine

Write-through and write-back modes for remote backends:

```
+------------------------------------------------------------------+
|                     Sync Engine Modes                             |
+------------------------------------------------------------------+

WRITE-THROUGH Mode (sync: { mode: write_through })
+--------------------------------------------------+
|                                                  |
|  write(path, content)                            |
|        |                                         |
|        v                                         |
|  +-----------+      +-----------+                |
|  |  Backend  |----->|  Remote   |   Synchronous  |
|  |   Write   |      |  Storage  |   write        |
|  +-----------+      +-----------+                |
|        |                  |                      |
|        +--------+---------+                      |
|                 |                                |
|                 v                                |
|           Return to client                       |
|           (after remote confirms)                |
|                                                  |
|  PRO: Strong consistency                         |
|  CON: Higher latency                             |
+--------------------------------------------------+

WRITE-BACK Mode (sync: { mode: write_back, interval: 30 })
+--------------------------------------------------+
|                                                  |
|  write(path, content)                            |
|        |                                         |
|        v                                         |
|  +-----------+      +----------------+           |
|  |  Local    |      |  Write Queue   |           |
|  |  Backend  |----->|  (in-memory)   |           |
|  +-----------+      +-------+--------+           |
|        |                    |                    |
|        v                    |                    |
|  Return to client           |  Background        |
|  (immediately)              |  sync thread       |
|                             v                    |
|                    +----------------+            |
|                    |  Batch writes  |            |
|                    |  to remote     |            |
|                    |  every 30s     |            |
|                    +-------+--------+            |
|                            |                     |
|                            v                     |
|                    +-----------+                 |
|                    |  Remote   |                 |
|                    |  Storage  |                 |
|                    +-----------+                 |
|                                                  |
|  PRO: Low latency writes                         |
|  CON: Potential data loss on crash               |
+--------------------------------------------------+

Sync Stats:
+-----------------------------------+
|          SyncStats                |
+-----------------------------------+
| pending_writes: 42    # Queued    |
| completed_syncs: 1337 # Completed |
| failed_syncs: 2       # Failed    |
| last_sync: <DateTime> # Last run  |
+-----------------------------------+
```

---

## Indexing Pipeline

Text processing pipeline for semantic search:

```
+------------------------------------------------------------------+
|                     Indexing Pipeline                             |
+------------------------------------------------------------------+

Input: File path or directory
         |
         v
+------------------+
|    Extractor     |    Supports: .txt, .md, .rs, .py, .js, .pdf
|                  |
|  Extract raw     |
|  text content    |
+--------+---------+
         |
         v
+------------------+
|     Chunker      |    Strategies:
|                  |    - Fixed: equal-size chunks
|  Split into      |    - Recursive: natural boundaries
|  chunks          |    - Semantic: paragraphs/sections
+--------+---------+    - AST: code-aware
         |
         |    +------------------+
         +--->|      Chunk       |
         |    +------------------+
         |    | source_path      |
         |    | content          |
         |    | start_offset     |
         |    | end_offset       |
         |    | start_line       |
         |    | end_line         |
         |    | chunk_index      |
         |    | total_chunks     |
         |    +------------------+
         |
         v
+------------------+     +------------------+
|    Embedder      |     |  SparseEncoder   |
|                  |     |                  |
|  Dense vectors   |     |  BM25 sparse     |
|  (768-1536 dim)  |     |  vectors         |
+--------+---------+     +--------+---------+
         |                        |
         v                        v
+------------------+     +------------------+
| [0.12, -0.45,   |     | indices: [42,    |
|  0.89, ...]     |     |   156, 892]      |
|                 |     | values: [1.2,    |
| Dense embedding |     |   0.8, 2.1]      |
+--------+---------+     +--------+---------+
         |                        |
         +------------+-----------+
                      |
                      v
            +------------------+
            |   Vector Store   |   Chroma, Pinecone, etc.
            |                  |
            |  Store chunks    |
            |  with metadata   |
            +------------------+

Chunking Strategies:

Fixed Chunker:
+-------+-------+-------+-------+
| 500   | 500   | 500   | 234   |  Fixed size chunks
| chars | chars | chars | chars |  with optional overlap
+-------+-------+-------+-------+

Recursive Chunker:
+------------------------------------------+
|  Try separators in order:                |
|  1. "\n\n" (paragraphs)                  |
|  2. "\n" (lines)                         |
|  3. ". " (sentences)                     |
|  4. " " (words)                          |
|  5. "" (characters)                      |
+------------------------------------------+

Semantic Chunker:
+------------------------------------------+
|  # Section 1        <- Header boundary   |
|  Content here...                         |
|                     <- Paragraph break   |
|  More content...                         |
|                                          |
|  ## Section 2       <- Header boundary   |
|  Different topic... <- Kept together     |
+------------------------------------------+
```

---

## AI Tool Generation

Generate tool definitions for AI assistants:

```
+------------------------------------------------------------------+
|                     Tool Generation Flow                          |
+------------------------------------------------------------------+

VfsConfig
    |
    v
+------------------+
|  generate_tools  |
|                  |
|  Analyze config: |
|  - Mount paths   |
|  - Read-only?    |
|  - Has indexing? |
+--------+---------+
         |
         v
+------------------+
| Vec<ToolDef>     |
|                  |
| - vfs_read       |
| - vfs_write      |
| - vfs_list       |
| - vfs_delete     |
| - vfs_search     |  (if indexing enabled)
| - vfs_mounts     |
+--------+---------+
         |
    +----+----+----+
    |         |    |
    v         v    v
  JSON      MCP  OpenAI
 Format   Format Format

JSON Format:
+------------------------------------------+
{
  "tools": [
    {
      "name": "vfs_read",
      "description": "Read file contents",
      "parameters": [
        {
          "name": "path",
          "type": "string",
          "required": true
        }
      ]
    }
  ]
}
+------------------------------------------+

MCP Format (Model Context Protocol):
+------------------------------------------+
{
  "tools": [
    {
      "name": "vfs_read",
      "description": "Read file contents",
      "input_schema": {
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Path to read"
          }
        },
        "required": ["path"]
      }
    }
  ]
}
+------------------------------------------+

OpenAI Function Calling Format:
+------------------------------------------+
{
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "vfs_read",
        "description": "Read file contents",
        "parameters": {
          "type": "object",
          "properties": {
            "path": {
              "type": "string",
              "description": "Path to read"
            }
          },
          "required": ["path"]
        }
      }
    }
  ]
}
+------------------------------------------+

Usage with AI:
+------------------------------------------+
|  AI Assistant receives tools definition  |
|                 |                        |
|                 v                        |
|  User: "Read the config file"            |
|                 |                        |
|                 v                        |
|  AI calls: vfs_read("/workspace/ax.yaml")|
|                 |                        |
|                 v                        |
|  VFS returns file content                |
|                 |                        |
|                 v                        |
|  AI: "The config contains..."            |
+------------------------------------------+
```

---

## Language Bindings

Python and TypeScript bindings for cross-language support:

```
+------------------------------------------------------------------+
|                      Language Bindings                            |
+------------------------------------------------------------------+

                         +-------------+
                         |   ax-core   |
                         |    (Rust)   |
                         +------+------+
                                |
               +----------------+----------------+
               |                                 |
               v                                 v
      +--------+--------+               +--------+--------+
      |     ax-ffi      |               |      ax-js      |
      |     (PyO3)      |               |    (napi-rs)    |
      +--------+--------+               +--------+--------+
               |                                 |
               v                                 v
      +--------+--------+               +--------+--------+
      |     Python      |               |   TypeScript    |
      |    import ax    |               |   import {..}   |
      +-----------------+               +-----------------+

Python Usage:
+------------------------------------------+
import ax

# Load configuration
vfs = ax.load_config('''
name: my-vfs
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
''')

# File operations
vfs.write_text('/workspace/hello.txt', 'Hello!')
content = vfs.read_text('/workspace/hello.txt')

# List directory
for entry in vfs.list('/workspace'):
    print(f"{entry.name}: {'dir' if entry.is_dir else 'file'}")

# Generate AI tools
tools = vfs.tools(format='openai')
+------------------------------------------+

TypeScript Usage:
+------------------------------------------+
import { loadConfig, JsVfs } from 'ax-vfs';

// Load configuration
const vfs = loadConfig(`
name: my-vfs
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
`);

// File operations
vfs.writeText('/workspace/hello.txt', 'Hello!');
const content = vfs.readText('/workspace/hello.txt');

// List directory
for (const entry of vfs.list('/workspace')) {
  console.log(`${entry.name}: ${entry.isDir ? 'dir' : 'file'}`);
}

// Generate AI tools
const tools = vfs.tools('mcp');
+------------------------------------------+

Type Mapping:
+------------------------------------------+
|   Rust          | Python       | TypeScript    |
|-----------------+--------------+---------------|
|   Vec<u8>       | bytes        | Buffer        |
|   String        | str          | string        |
|   Option<T>     | T | None     | T | null      |
|   Result<T, E>  | T (raises)   | T (throws)    |
|   Entry         | PyEntry      | JsEntry       |
|   VfsError      | IOError      | Error         |
+------------------------------------------+
```

---

## Configuration System

YAML-based configuration with environment variable support:

```
+------------------------------------------------------------------+
|                    Configuration Flow                             |
+------------------------------------------------------------------+

YAML File (ax.yaml)
         |
         v
+------------------+
|   Parse YAML     |
|   (serde_yaml)   |
+--------+---------+
         |
         v
+------------------+
|  Interpolate     |    ${VAR_NAME} -> env value
|  Environment     |
|  Variables       |
+--------+---------+
         |
         v
+------------------+
|  Apply Smart     |    - Infer backend from mount
|  Defaults        |    - Generate collection names
|                  |    - Set default cache config
+--------+---------+
         |
         v
+------------------+
|   Validate       |    - No duplicate mounts
|   Configuration  |    - Valid mount paths (start with /)
|                  |    - Backend exists for each mount
|                  |    - No overlapping mounts
+--------+---------+
         |
         v
    VfsConfig (ready to use)

Configuration Structure:
+------------------------------------------+
name: my-vfs                # Optional name
version: 1                  # Config version

backends:
  local:                    # Backend name
    type: fs                # Backend type
    root: ./data            # Type-specific config

  s3:
    type: s3
    bucket: ${S3_BUCKET}    # Environment variable
    region: us-east-1
    access_key_id: ${AWS_ACCESS_KEY_ID}
    secret_access_key: ${AWS_SECRET_ACCESS_KEY}

mounts:
  - path: /workspace        # Virtual mount path
    backend: local          # Which backend to use
    read_only: false        # Allow writes
    collection: workspace   # For indexing

  - path: /archive
    backend: s3
    read_only: true

defaults:                   # Global defaults
  cache:
    enabled: true
    max_entries: 1000
    ttl_seconds: 300

  sync:
    mode: write_through
+------------------------------------------+

Smart Defaults:
+------------------------------------------+
|  Input:                                  |
|  backends:                               |
|    local:                                |
|      type: fs                            |
|      root: ./data                        |
|  mounts:                                 |
|    - path: /workspace                    |
|                                          |
|  After defaults applied:                 |
|  mounts:                                 |
|    - path: /workspace                    |
|      backend: local      # Inferred      |
|      collection: workspace # Generated   |
|      read_only: false    # Default       |
+------------------------------------------+

Validation Rules:
+------------------------------------------+
|  1. Mount paths must start with /        |
|     /workspace     OK                    |
|     workspace      ERROR                 |
|                                          |
|  2. No duplicate mount paths             |
|     /data, /data   ERROR                 |
|                                          |
|  3. Backend must exist                   |
|     backend: unknown  ERROR              |
|                                          |
|  4. No overlapping mounts                |
|     /data, /data/sub  ERROR              |
|                                          |
|  5. Read-only mounts can't have sync     |
|     read_only: true                      |
|     sync: {...}   ERROR                  |
+------------------------------------------+
```

---

## Search Architecture

Hybrid search combining dense and sparse vectors:

```
+------------------------------------------------------------------+
|                     Search Architecture                           |
+------------------------------------------------------------------+

Query: "how to configure S3 backend"
         |
         v
+------------------+
|  Search Engine   |
+--------+---------+
         |
    +----+----+
    |         |
    v         v
+-------+  +-------+
| Dense |  |Sparse |
| Search|  |Search |
+---+---+  +---+---+
    |          |
    v          v
+-------+  +-------+
|Embed  |  | BM25  |
|Query  |  |Encode |
+---+---+  +---+---+
    |          |
    v          v
+-------+  +-------+
|Vector |  |Keyword|
|  DB   |  | Match |
+---+---+  +---+---+
    |          |
    v          v
+-------+  +-------+
|Top K  |  |Top K  |
|Dense  |  |Sparse |
+---+---+  +---+---+
    |          |
    +----+-----+
         |
         v
+------------------+
|  Reciprocal Rank |    Hybrid fusion
|     Fusion       |
+--------+---------+
         |
         v
+------------------+
|  Final Results   |
|                  |
| 1. config/s3.md  |
| 2. README.md     |
| 3. examples/...  |
+------------------+

Search Modes:
+------------------------------------------+
|  Mode: dense                             |
|  - Uses only embedding similarity        |
|  - Good for semantic meaning             |
|  - "authentication" finds "login"        |
|                                          |
|  Mode: sparse                            |
|  - Uses only BM25 keyword matching       |
|  - Good for exact terms                  |
|  - "S3Backend" finds exact matches       |
|                                          |
|  Mode: hybrid (default)                  |
|  - Combines both with RRF                |
|  - Best of both worlds                   |
|  - Semantic + keyword relevance          |
+------------------------------------------+

BM25 Scoring:
+------------------------------------------+
|  score(D, Q) = Σ IDF(qi) *               |
|    (f(qi, D) * (k1 + 1)) /               |
|    (f(qi, D) + k1 * (1 - b + b * |D|/avgdl))|
|                                          |
|  where:                                  |
|    qi = query term                       |
|    f(qi, D) = term frequency in doc      |
|    |D| = document length                 |
|    avgdl = average document length       |
|    k1 = 1.5, b = 0.75 (tuning params)    |
+------------------------------------------+
```

---

## Metrics & Observability

Operation tracking and performance metrics:

```
+------------------------------------------------------------------+
|                        Metrics System                             |
+------------------------------------------------------------------+

                    +------------------+
                    |    VfsMetrics    |
                    +------------------+
                    |                  |
  Operation ------->| record_read()   |
                    | record_write()  |
                    | record_error()  |
                    | record_latency()|
                    +--------+---------+
                             |
                             v
                    +------------------+
                    | MetricsSnapshot  |
                    +------------------+

Snapshot Contents:
+------------------------------------------+
|  MetricsSnapshot                         |
|  ----------------------------------------|
|  reads: 1542          # Total reads      |
|  read_bytes: 15.2MB   # Bytes read       |
|  read_errors: 3       # Failed reads     |
|  read_latency_avg_ms: 12.5               |
|  read_latency_p99_ms: 45.2               |
|                                          |
|  writes: 234          # Total writes     |
|  write_bytes: 2.1MB   # Bytes written    |
|  write_errors: 0      # Failed writes    |
|  write_latency_avg_ms: 8.3               |
|  write_latency_p99_ms: 22.1              |
|                                          |
|  deletes: 45          # Total deletes    |
|  delete_errors: 1     # Failed deletes   |
|                                          |
|  lists: 892           # List operations  |
|  list_errors: 0       # Failed lists     |
+------------------------------------------+

Latency Guard (RAII):
+------------------------------------------+
|  let guard = LatencyGuard::read(&metrics);|
|                                          |
|  // ... perform operation ...            |
|                                          |
|  // guard dropped here                   |
|  // automatically records elapsed time   |
+------------------------------------------+

Derived Metrics:
+------------------------------------------+
|  read_error_rate():  read_errors / reads |
|  write_error_rate(): write_errors/writes |
|  total_operations(): reads + writes +    |
|                      deletes + lists     |
|  total_errors():     sum of all errors   |
+------------------------------------------+
```

---

## Error Handling

Comprehensive error types across all layers:

```
+------------------------------------------------------------------+
|                     Error Type Hierarchy                          |
+------------------------------------------------------------------+

VfsError (ax-core)
|
+-- NoMount(path)           # No mount point for path
+-- ReadOnly(path)          # Write to read-only mount
+-- NotFound(path)          # File/directory not found
+-- AlreadyExists(path)     # Path already exists
+-- Backend(BackendError)   # Wrapped backend error
+-- Config(String)          # Configuration error
+-- Io(std::io::Error)      # I/O error

BackendError (ax-backends)
|
+-- NotFound(path)          # Resource not found
+-- PermissionDenied(path)  # Access denied
+-- AlreadyExists(path)     # Resource exists
+-- Io(std::io::Error)      # I/O error
+-- Connection(String)      # Connection failed
+-- Timeout(String)         # Operation timed out

ConfigError (ax-config)
|
+-- ParseError(String)      # YAML parse error
+-- ValidationError(String) # Validation failed
+-- MissingEnvVars(Vec)     # Missing env variables
+-- DuplicateMountPath(p)   # Duplicate mount
+-- InvalidMountPath(p, m)  # Invalid path format
+-- UndefinedBackend(b, m)  # Backend not found
+-- OverlappingMountPaths   # Overlapping mounts

IndexingError (ax-indexing)
|
+-- ChunkingError(String)   # Chunking failed
+-- EmbeddingError(String)  # Embedding failed
+-- ExtractionError(String) # Text extraction failed
+-- UnsupportedFileType(t)  # Can't process file type

Error Propagation:
+------------------------------------------+
|                                          |
|  BackendError ----+                      |
|                   |                      |
|  ConfigError -----+---> VfsError         |
|                   |         |            |
|  std::io::Error --+         |            |
|                             v            |
|                     Python: IOError      |
|                     TypeScript: Error    |
|                                          |
+------------------------------------------+
```

---

## Summary

AX provides a powerful abstraction layer for AI agents to interact with
various storage systems through a unified, familiar filesystem-like API.

Key design principles:

1. **Unified Interface**: One API for local files, cloud storage, and databases
2. **Mount-Based**: Unix-like mount points for organizing heterogeneous backends
3. **Performance**: LRU caching with TTL for frequently accessed files
4. **Flexibility**: Write-through and write-back sync modes
5. **AI-Native**: Built-in tool generation for AI assistants
6. **Search**: Hybrid semantic + keyword search capabilities
7. **Cross-Platform**: Python and TypeScript bindings for broad adoption

```
+------------------------------------------------------------------+
|                                                                   |
|   "Simplifying file access for AI agents, one mount at a time"   |
|                                                                   |
+------------------------------------------------------------------+
```

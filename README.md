# AX - Agentic Files

A virtual filesystem designed for AI agents and automation. AX provides a unified interface to multiple storage backends with support for caching, syncing, semantic search, and tool generation for AI assistants.

## Features

- **9 Storage Backends**: Local filesystem, memory, S3-compatible, PostgreSQL, Chroma vector DB, WebDAV, SFTP, Google Cloud Storage, Azure Blob Storage
- **Mount-based Routing**: Unix-like mount points for organizing different backends
- **Caching**: Lock-free LRU cache (moka) with TTL support
- **Sync Engine**: Write-through, write-back, and pull-mirror modes with WAL-based durability
- **Semantic Search**: Text chunking, embeddings, and hybrid search (dense + BM25 sparse) via Chroma
- **FUSE Filesystem**: Mount the VFS as a native filesystem (macOS/Linux, Windows stub)
- **MCP Server**: JSON-RPC 2.0 Model Context Protocol server over stdio (7 tools)
- **REST API**: Axum-based HTTP API with OpenAPI spec
- **AI Tool Generation**: Generate tool definitions for MCP, OpenAI, and JSON formats
- **Language Bindings**: Python (PyO3) and TypeScript (napi-rs) support

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/ax-vfs/ax.git
cd ax

# Build the CLI
cargo build --release

# Install to PATH
cargo install --path crates/ax-cli
```

### Optional Features

```bash
# Build with S3 support
cargo build --release --features s3

# Build with PostgreSQL support
cargo build --release --features postgres

# Build with all backends
cargo build --release --features all-backends

# Build with PDF extraction
cargo build --release -p ax-indexing --features extractor-pdf
```

## Quick Start

### 1. Create a Configuration File

Create `ax.yaml` in your project directory:

```yaml
name: my-workspace

backends:
  local:
    type: fs
    root: ./data

mounts:
  - path: /workspace
    backend: local
```

### 2. Use the CLI

```bash
# Write a file
ax write /workspace/hello.txt "Hello, World!"

# Read it back
ax cat /workspace/hello.txt

# List directory
ax ls /workspace

# Show file tree
ax tree /workspace

# Delete a file
ax rm /workspace/hello.txt
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `ls [path]` | List directory contents |
| `cat <path>` | Display file contents |
| `write <path> [content]` | Write content to a file |
| `append <path> [content]` | Append content to a file |
| `rm <path>` | Remove a file or directory |
| `stat <path>` | Show file metadata |
| `exists <path>` | Check if path exists (exit 0/1) |
| `cp <src> <dst>` | Copy a file |
| `mv <src> <dst>` | Move/rename a file |
| `tree [path]` | Show directory tree |
| `find <pattern>` | Find files by regex pattern |
| `grep <pattern> [path]` | Search file contents |
| `index [path]` | Index files for semantic search (`--incremental`, `--force`) |
| `index-status` | Show index state (files, chunks, last updated) |
| `search <query>` | Semantic search in indexed files |
| `watch` | Watch for changes with auto-indexing |
| `mount <path>` | FUSE mount the VFS |
| `unmount <path>` | FUSE unmount |
| `serve` | Start REST API server |
| `mcp` | Start MCP server (stdio) |
| `config` | Show effective configuration |
| `status` | Show VFS status and stats |
| `validate` | Validate configuration |
| `migrate` | Migrate configuration to latest format |
| `tools` | Generate AI tool definitions |
| `wal` | WAL management (checkpoint, status) |

## Configuration

### Backend Types

#### Local Filesystem
```yaml
backends:
  local:
    type: fs
    root: ./data
```

#### Memory
```yaml
backends:
  mem:
    type: memory
```

#### S3-Compatible Storage
```yaml
backends:
  s3:
    type: s3
    bucket: my-bucket
    region: us-east-1
    prefix: data/
    endpoint: http://localhost:9000  # Optional, for MinIO
    access_key_id: ${AWS_ACCESS_KEY_ID}
    secret_access_key: ${AWS_SECRET_ACCESS_KEY}
```

#### PostgreSQL
```yaml
backends:
  pg:
    type: postgres
    connection_url: postgres://user:pass@localhost/db
    table_name: ax_files
    max_connections: 5
```

#### WebDAV
```yaml
backends:
  nas:
    type: webdav
    url: https://server/dav
    username: user
    password: ${WEBDAV_PASS}
```

#### SFTP
```yaml
backends:
  remote:
    type: sftp
    host: server.example.com
    username: deploy
    private_key: ~/.ssh/id_ed25519
    root: /var/data
```

#### Google Cloud Storage
```yaml
backends:
  gcs:
    type: gcs
    bucket: my-gcs-bucket
    prefix: data/
```

#### Azure Blob Storage
```yaml
backends:
  azure:
    type: azure_blob
    container: my-container
    account: mystorageaccount
    access_key: ${AZURE_KEY}
```

### Mount Options

```yaml
mounts:
  - path: /workspace
    backend: local
    read_only: false      # Allow writes (default)
    collection: workspace # For indexing/search
```

### Environment Variables

Use `${VAR_NAME}` syntax for environment variable interpolation:

```yaml
backends:
  s3:
    type: s3
    bucket: ${S3_BUCKET}
    access_key_id: ${AWS_ACCESS_KEY_ID}
    secret_access_key: ${AWS_SECRET_ACCESS_KEY}
```

## AI Tool Generation

Generate tool definitions for AI assistants:

```bash
# JSON format (default)
ax tools

# MCP (Model Context Protocol) format
ax tools --format mcp

# OpenAI function calling format
ax tools --format openai

# Pretty-printed
ax tools --format openai --pretty
```

## Python Bindings

```bash
# Build Python bindings
cd crates/ax-ffi
maturin develop
```

```python
import ax

# Load from YAML string
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
vfs.write_text('/workspace/hello.txt', 'Hello from Python!')
content = vfs.read_text('/workspace/hello.txt')
print(content)

# List directory
for entry in vfs.list('/workspace'):
    print(f"{entry.name} - {'dir' if entry.is_dir else 'file'}")

# Generate tools for AI
tools_json = vfs.tools(format='openai')
```

## TypeScript/Node.js Bindings

```bash
# Build Node.js bindings
cd crates/ax-js
npm install
npm run build
```

```typescript
import { loadConfig, JsVfs } from 'ax-vfs';

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
vfs.writeText('/workspace/hello.txt', 'Hello from TypeScript!');
const content = vfs.readText('/workspace/hello.txt');
console.log(content);

// List directory
for (const entry of vfs.list('/workspace')) {
  console.log(`${entry.name} - ${entry.isDir ? 'dir' : 'file'}`);
}

// Generate tools
const tools = vfs.tools('mcp');
```

## Semantic Search

### Indexing Files

```bash
# Full index
ax index /workspace

# Incremental (only changed files)
ax index /workspace --incremental

# Force full re-index
ax index /workspace --force

# Check index status
ax index-status
```

### Searching

```bash
# Hybrid search (default — dense + BM25 sparse)
ax search "mount point configuration" --limit 5
```

## Architecture

```
ax/
├── crates/
│   ├── ax-config/     # Configuration parsing, validation, env interpolation
│   ├── ax-core/       # VFS, routing, caching, sync/WAL, tools, search, pipeline
│   ├── ax-backends/   # Storage backends (fs, memory, s3, postgres, chroma, webdav, sftp, gcs, azure)
│   ├── ax-indexing/   # Text chunking, embeddings, BM25 sparse, hybrid search
│   ├── ax-fuse/       # FUSE filesystem (macOS/Linux + Windows stub)
│   ├── ax-mcp/        # MCP server (JSON-RPC 2.0 over stdio)
│   ├── ax-server/     # REST API server (Axum)
│   ├── ax-cli/        # Command-line interface (27 subcommands)
│   ├── ax-ffi/        # Python bindings (PyO3, excluded from default build)
│   └── ax-js/         # TypeScript bindings (napi-rs, excluded from default build)
├── docs/              # Architecture, guides, and status docs
└── examples/          # Example configurations
```

## Development

```bash
# Run all tests
cargo test --workspace

# Run with all features
cargo test --workspace --features all-backends

# Run specific crate tests
cargo test -p ax-core

# Run integration tests
cargo test -p ax-cli --test cli_integration
```

## License

MIT

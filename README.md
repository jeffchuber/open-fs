# AX - Agentic Files

A virtual filesystem designed for AI agents and automation. AX provides a unified interface to multiple storage backends with support for caching, syncing, semantic search, and tool generation for AI assistants.

## Features

- **Multiple Backends**: Local filesystem, S3-compatible storage, PostgreSQL, and Chroma vector database
- **Mount-based Routing**: Unix-like mount points for organizing different backends
- **Caching**: LRU cache with TTL support for improved performance
- **Sync Engine**: Write-through and write-back modes for remote backends
- **Semantic Search**: Text chunking, embeddings, and hybrid search (dense + sparse)
- **AI Tool Generation**: Generate tool definitions for MCP, OpenAI, and other formats
- **Language Bindings**: Python (PyO3) and TypeScript (napi-rs) support

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/your-org/ax.git
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
| `config` | Show effective configuration |
| `status` | Show VFS status and stats |
| `tools` | Generate AI tool definitions |
| `index [path]` | Index files for semantic search |
| `search <query>` | Semantic search in indexed files |

## Configuration

### Backend Types

#### Local Filesystem
```yaml
backends:
  local:
    type: fs
    root: ./data
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
# Index a directory with Chroma
ax index /workspace --chroma-endpoint http://localhost:8000 --collection docs

# Custom chunking
ax index /workspace --chunker recursive --chunk-size 500
```

### Searching

```bash
# Dense search (embeddings only)
ax search "how to configure backends" --mode dense

# Sparse search (BM25 keyword matching)
ax search "configuration yaml" --mode sparse

# Hybrid search (default)
ax search "mount point configuration" --limit 5
```

## Architecture

```
ax/
├── crates/
│   ├── ax-config/     # Configuration parsing and validation
│   ├── ax-core/       # VFS, routing, caching, sync, tools
│   ├── ax-backends/   # Storage backends (fs, s3, postgres, chroma)
│   ├── ax-indexing/   # Text chunking, embeddings, search
│   ├── ax-cli/        # Command-line interface
│   ├── ax-ffi/        # Python bindings (PyO3)
│   └── ax-js/         # TypeScript bindings (napi-rs)
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

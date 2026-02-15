# Getting Started

This guide walks you through installing AX, creating your first config, and using it from the CLI, Python, TypeScript, and as a FUSE mount.

## Install

```bash
git clone https://github.com/ax-vfs/ax.git
cd ax
cargo build --release
cargo install --path crates/ax-cli
```

To enable all optional storage backends (S3, PostgreSQL):

```bash
cargo install --path crates/ax-cli --features all-backends
```

Verify:

```bash
ax --version
```

## Create a Config

Save this as `ax.yaml`:

```yaml
name: hello

backends:
  local:
    type: fs
    root: ./data

mounts:
  - path: /files
    backend: local
```

Create the data directory:

```bash
mkdir -p data
```

You now have a virtual filesystem that maps `/files` to `./data` on disk.

## Use the CLI

```bash
# Write a file
ax write /files/hello.txt "Hello, world!"

# Read it back
ax cat /files/hello.txt

# List the directory
ax ls /files

# Append to it
ax append /files/hello.txt " More content."

# File metadata
ax stat /files/hello.txt

# Check existence (exit code 0 = exists, 1 = not found)
ax exists /files/hello.txt

# Copy and move
ax cp /files/hello.txt /files/copy.txt
ax mv /files/copy.txt /files/renamed.txt

# Directory tree
ax tree /files

# Find files by regex
ax find "\.txt$" --path /files

# Search file contents
ax grep "Hello" --path /files --recursive

# Delete
ax rm /files/renamed.txt
```

All commands accept `--config <path>` to specify the config file. Without it, AX looks for `ax.yaml` in the current directory, then `~/.config/ax/config.yaml`.

## Multiple Mounts

The real value of AX is routing different paths to different backends:

```yaml
name: multi-mount

backends:
  workspace:
    type: fs
    root: ./workspace
  reference:
    type: fs
    root: ./reference

mounts:
  - path: /work
    backend: workspace
  - path: /ref
    backend: reference
    read_only: true
```

```bash
mkdir -p workspace reference
echo "API docs here" > reference/api.md
```

Now `/work` is read-write and `/ref` is read-only. From the agent's perspective, they look the same — AX handles the routing.

## Cloud Backends

Replace `fs` with any supported backend. The CLI and API work identically regardless of backend:

```yaml
name: cloud-workspace

backends:
  local:
    type: fs
    root: ./src
  s3:
    type: s3
    bucket: my-bucket
    region: us-east-1

mounts:
  - path: /code
    backend: local
  - path: /data
    backend: s3
    sync:
      mode: write_through
```

Environment variables are interpolated with `${VAR_NAME}` syntax. See [project_overview.md](project_overview.md) for the full list of supported backends.

## Caching

For remote backends, enable caching to avoid repeated network round-trips:

```yaml
defaults:
  cache:
    enabled: true
    max_entries: 1000
    ttl_seconds: 300
```

The cache is a lock-free LRU (moka) that automatically invalidates on writes.

## Semantic Search

AX can index your files for semantic search using vector embeddings and BM25 keyword matching, stored in a Chroma vector database.

### Prerequisites

Start a Chroma server:

```bash
pip install chromadb
chroma run --port 8000
```

### Index

```bash
# Full index
ax index /files

# Incremental (only changed files)
ax index /files --incremental

# Force re-index from scratch
ax index /files --force

# Check index status
ax index-status
```

### Search

```bash
# Hybrid search (dense + sparse, default)
ax search "how does authentication work" --limit 5

# Dense-only (vector similarity)
ax search "auth flow" --mode dense

# Sparse-only (BM25 keyword)
ax search "login" --mode sparse
```

### Watch Mode

Auto-reindex when files change:

```bash
ax watch --auto-index
```

Uses a SQLite-backed work queue with debounce, dedup, retry, and crash recovery.

## FUSE Mount

Mount the VFS as a native filesystem so any program can access it with standard file operations.

### Prerequisites

**macOS:**
```bash
brew install --cask macfuse
# Allow the kernel extension in System Preferences > Privacy & Security
```

**Linux:**
```bash
sudo apt install libfuse3-dev fuse3    # Debian/Ubuntu
sudo dnf install fuse3-devel fuse3     # Fedora
```

### Mount and Use

```bash
ax mount ~/ax-mount --config ax.yaml

# Now standard tools work
ls ~/ax-mount/files/
cat ~/ax-mount/files/hello.txt

# Claude Code can use it directly
claude --working-dir ~/ax-mount

# Semantic search via virtual directory
ls ~/ax-mount/.search/query/authentication/

# Unmount when done
ax unmount ~/ax-mount
```

## REST API

Start an HTTP server:

```bash
ax serve --port 19557 --api-key "my-secret"
```

```bash
# Write
curl -X POST http://localhost:19557/v1/write \
  -H "Authorization: Bearer my-secret" \
  -H "Content-Type: application/json" \
  -d '{"path": "/files/hello.txt", "content": "Hello from curl!"}'

# Read
curl -H "Authorization: Bearer my-secret" \
  "http://localhost:19557/v1/read?path=/files/hello.txt"

# List
curl -H "Authorization: Bearer my-secret" \
  "http://localhost:19557/v1/ls?path=/files"

# OpenAPI spec
curl http://localhost:19557/v1/openapi
```

14 endpoints total. Full spec available at `/v1/openapi`.

## MCP Server

For LLM tool-use frameworks that speak the Model Context Protocol:

```bash
ax mcp
```

Starts a JSON-RPC 2.0 server over stdio with 7 tools: `ax_read`, `ax_write`, `ax_ls`, `ax_stat`, `ax_delete`, `ax_grep`, `ax_search`.

## Python

```bash
cd crates/ax-ffi && maturin develop
```

```python
import ax

vfs = ax.load_config_file("ax.yaml")

vfs.write_text("/files/hello.txt", "Hello from Python!")
print(vfs.read_text("/files/hello.txt"))

for entry in vfs.list("/files"):
    print(f"  {entry.name}  ({'dir' if entry.is_dir else str(entry.size) + 'b'})")

for match in vfs.grep("Hello", "/files", recursive=True):
    print(f"  {match.path}:{match.line_number}  {match.line}")

# Generate AI tool definitions
tools = vfs.tools("openai")
```

## TypeScript

```bash
cd crates/ax-js && npm install && npm run build
```

```typescript
const { loadConfigFile } = require('ax-vfs');

const vfs = loadConfigFile('ax.yaml');

vfs.writeText('/files/hello.txt', 'Hello from Node!');
console.log(vfs.readText('/files/hello.txt'));

for (const entry of vfs.list('/files')) {
  console.log(`  ${entry.name}  ${entry.isDir ? 'dir' : entry.size + 'b'}`);
}

const tools = vfs.tools('mcp');
```

## AI Tool Generation

Generate tool definitions to give AI agents file access:

```bash
# MCP format (for Claude, etc.)
ax tools --format mcp --pretty

# OpenAI function calling format
ax tools --format openai --pretty

# Raw JSON
ax tools --format json
```

Also available from Python (`vfs.tools("openai")`) and TypeScript (`vfs.tools('mcp')`).

## Troubleshooting

**macFUSE not loading (macOS):**
Open System Preferences > Privacy & Security, scroll down, and click "Allow" for the macFUSE kernel extension. Reboot.

**Permission denied on mount:**
On Linux, add your user to the `fuse` group: `sudo usermod -aG fuse $USER`, then log out and back in.

**Mount point busy on unmount:**
```bash
ax unmount ~/ax-mount --force
# or directly:
umount -f ~/ax-mount        # macOS
fusermount -uz ~/ax-mount   # Linux
```

**Config not found:**
AX searches in order: `$AX_CONFIG` env var, then `ax.yaml` in the current directory, then `~/.config/ax/config.yaml`. Pass `--config path/to/ax.yaml` explicitly.

**Stale FUSE mount after crash:**
If the AX process dies, the mount point may become stale. Force-unmount it, then re-mount:
```bash
ax unmount ~/ax-mount --force
ax mount ~/ax-mount --config ax.yaml
```

## What's Next

- [project_overview.md](project_overview.md) — Architecture, concepts, and full feature overview
- [USE_CASES.md](USE_CASES.md) — Real-world patterns: agent memory, skill libraries, code generation, data fabric, multi-agent collaboration
- [ARCHITECTURE.md](ARCHITECTURE.md) — Detailed internals with diagrams
- [PROJECT_STATUS.md](PROJECT_STATUS.md) — Development status and roadmap

Every CLI command has built-in help:

```bash
ax --help
ax search --help
ax mount --help
```

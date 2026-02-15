# AX - Agentic Files

AX is a virtual filesystem for AI agents and automation.

It provides a unified namespace across mounted storage backends, with:
- routing by mount path
- optional caching + sync behavior
- grep + semantic search
- MCP tool server
- Unix FUSE mount (macOS/Linux)

## Current Scope

AX currently ships as a Rust workspace with these production crates:
- `ax-config` config parsing/validation
- `ax-core` shared types/errors/cache/tool schema
- `ax-local` indexing + search pipeline (this is the indexing implementation)
- `ax-remote` VFS routing/backends/sync/WAL/grep
- `ax-fuse` Unix FUSE integration
- `ax-mcp` MCP server
- `ax-cli` CLI
- `ax-sim` simulation harness

Removed from this repo surface:
- REST API server (`ax-server`)
- Python bindings (`ax-ffi`)
- TypeScript bindings (`ax-js`)
- standalone `ax-indexing` crate

## Install

```bash
git clone https://github.com/ax-vfs/ax.git
cd ax
cargo build --release
cargo install --path crates/ax-cli
```

### Optional backend features

```bash
# S3 support
cargo build --release --features s3

# PostgreSQL support
cargo build --release --features postgres

# all optional backends in workspace crates
cargo build --release --features all-backends
```

## Quick Start

Create `ax.yaml`:

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

Use the CLI:

```bash
ax write /workspace/hello.txt "Hello, world!"
ax cat /workspace/hello.txt
ax ls /workspace
ax grep "Hello" /workspace --recursive
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `ls [path]` | List directory contents |
| `cat <path>` | Display file contents |
| `write <path> [content]` | Write file contents |
| `append <path> [content]` | Append to a file |
| `rm <path>` | Remove file/directory |
| `stat <path>` | Show metadata |
| `exists <path>` | Check path existence |
| `cp <src> <dst>` | Copy file |
| `mv <src> <dst>` | Move/rename file |
| `tree [path]` | Show directory tree |
| `find <pattern>` | Find files by regex |
| `grep <pattern> [path]` | Search file contents |
| `index [path]` | Index files for semantic search |
| `index-status` | Show index status |
| `search <query>` | Semantic search |
| `watch` | Watch filesystem changes |
| `mount <path>` | FUSE mount (feature `fuse`) |
| `unmount <path>` | FUSE unmount |
| `mcp` | Start MCP server |
| `config` | Print effective config |
| `status` | Show VFS status |
| `validate` | Validate config |
| `migrate` | Migrate config |
| `tools` | Generate tool definitions |
| `wal` | WAL status/checkpoint |

## Config Backends

Supported backend types:
- `fs`
- `memory`
- `s3` (feature `s3`)
- `postgres` (feature `postgres`)
- `chroma`

Example S3 backend:

```yaml
backends:
  s3:
    type: s3
    bucket: my-bucket
    region: us-east-1
```

Example Postgres backend:

```yaml
backends:
  pg:
    type: postgres
    connection_url: postgres://user:pass@localhost/db
```

## Semantic Search

Indexing/search is implemented in `ax-local`.

```bash
ax index /workspace
ax search "authentication flow" --limit 5
```

## MCP

Run MCP server over stdio:

```bash
ax mcp
```

## FUSE (macOS/Linux)

```bash
ax mount ~/ax-mount --config ax.yaml
ax unmount ~/ax-mount
```

## License

MIT

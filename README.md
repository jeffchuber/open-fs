# OpenFS

OpenFS is a virtual filesystem for AI agents and automation.

It provides a unified namespace across mounted storage backends, with:
- routing by mount path
- optional caching + sync behavior
- grep + semantic search
- MCP tool server
- Unix FUSE mount (macOS/Linux)

## Current Scope

OpenFS currently ships as a Rust workspace with these production crates:
- `openfs-config` config parsing/validation
- `openfs-core` shared types/errors/cache/tool schema
- `openfs-local` indexing + search pipeline (this is the indexing implementation)
- `openfs-remote` VFS routing/backends/sync/WAL/grep
- `openfs-fuse` Unix FUSE integration
- `openfs-mcp` MCP server
- `openfs-cli` CLI
- `openfs-sim` simulation harness

Also in this repo:
- `ts/` — TypeScript package (`@open-fs/core`): thin typed Vfs wrapper over the Rust binary via MCP

## Install

```bash
cargo build --release
cargo install --path crates/openfs-cli
```

### Optional backend features

```bash
# S3 backend support
cargo install --path crates/openfs-cli --features openfs-remote/s3

# PostgreSQL backend support
cargo install --path crates/openfs-cli --features openfs-remote/postgres

# all optional remote backends
cargo install --path crates/openfs-cli --features openfs-remote/all-backends

# FUSE mount command support
cargo install --path crates/openfs-cli --features fuse
```

## Quick Start

Create `openfs.yaml`:

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
openfs write /workspace/hello.txt "Hello, world!"
openfs cat /workspace/hello.txt
openfs ls /workspace
openfs grep "Hello" /workspace --recursive
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
| `sync` | Sync status + manual write-back flush |
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
- `s3` (build `openfs-cli` with feature `openfs-remote/s3`)
- `postgres` (build `openfs-cli` with feature `openfs-remote/postgres`)
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

Indexing/search is implemented in `openfs-local`.

```bash
openfs index /workspace
openfs search "authentication flow" --limit 5
```

## MCP

Run MCP server over stdio:

```bash
openfs mcp
```

## FUSE (macOS/Linux)

```bash
openfs --config openfs.yaml mount ~/openfs-mount
openfs unmount ~/openfs-mount
```

## Write-Back Sync

For write-back mounts, inspect sync state and force a flush:

```bash
openfs sync status
openfs sync flush
```

## Simulation Debug UI

Generate an interactive debug dashboard from `openfs-sim` output:

```bash
cargo run -p openfs-sim --example debug_ui -- --steps 200 --seed 42 --mode mixed --write-back --out /tmp/openfs-sim-debug
```

This writes:
- `/tmp/openfs-sim-debug/sim-debug-ui.html`
- `/tmp/openfs-sim-debug/sim-debug-data.json`

Open the HTML and use the replay controls to scrub and play simulation state forward/backward by step.
The replay panel includes a file state map (local vs remote vs pending/WAL) so you can see where each file currently lives.
The debug example runs with three clients:
- `agent 1` and `agent 2` share one indexed backing store shown as `Remote 0`.
- `agent 0` keeps a separate indexed backing store.

## License

MIT

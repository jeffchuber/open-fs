# Getting Started

This guide covers a minimal AX setup with CLI, semantic indexing/search, MCP, and FUSE.

## Install

```bash
git clone https://github.com/ax-vfs/ax.git
cd ax
cargo build --release
cargo install --path crates/ax-cli
```

Optional backend features:

```bash
cargo install --path crates/ax-cli --features ax-remote/s3
cargo install --path crates/ax-cli --features ax-remote/postgres
cargo install --path crates/ax-cli --features ax-remote/all-backends
```

FUSE command support:

```bash
cargo install --path crates/ax-cli --features fuse
```

## Create a config

Save as `ax.yaml`:

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

```bash
mkdir -p data
```

## Basic CLI flow

```bash
ax write /files/hello.txt "Hello, world!"
ax cat /files/hello.txt
ax ls /files
ax append /files/hello.txt " More text."
ax stat /files/hello.txt
ax grep "Hello" /files --recursive
```

## Multiple mounts

```yaml
name: multi

backends:
  code:
    type: fs
    root: ./workspace
  ref:
    type: fs
    root: ./reference

mounts:
  - path: /work
    backend: code
  - path: /ref
    backend: ref
    read_only: true
```

## Semantic indexing/search

Indexing is implemented in `ax-local`.

```bash
ax index /files
ax index /files --incremental
ax index-status
ax search "how authentication works" --limit 5
```

## MCP

```bash
ax mcp
```

## Sync

```bash
ax sync status
ax sync flush
```

## FUSE mount (macOS/Linux)

```bash
ax --config ax.yaml mount ~/ax-mount
ls ~/ax-mount/files
ax unmount ~/ax-mount
```

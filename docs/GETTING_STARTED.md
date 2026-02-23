# Getting Started

This guide covers a minimal AX setup with CLI, semantic indexing/search, MCP, and FUSE.

## Install

```bash
git clone https://github.com/open-fs/openfs.git
cd openfs
cargo build --release
cargo install --path crates/openfs-cli
```

Optional backend features:

```bash
cargo install --path crates/openfs-cli --features openfs-remote/s3
cargo install --path crates/openfs-cli --features openfs-remote/postgres
cargo install --path crates/openfs-cli --features openfs-remote/all-backends
```

FUSE command support:

```bash
cargo install --path crates/openfs-cli --features fuse
```

## Create a config

Save as `openfs.yaml`:

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
openfs write /files/hello.txt "Hello, world!"
openfs cat /files/hello.txt
openfs ls /files
openfs append /files/hello.txt " More text."
openfs stat /files/hello.txt
openfs grep "Hello" /files --recursive
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

Indexing is implemented in `openfs-local`.

```bash
openfs index /files
openfs index /files --incremental
openfs index-status
openfs search "how authentication works" --limit 5
```

## MCP

```bash
openfs mcp
```

## Sync

```bash
openfs sync status
openfs sync flush
```

## FUSE mount (macOS/Linux)

```bash
openfs --config openfs.yaml mount ~/openfs-mount
ls ~/openfs-mount/files
openfs unmount ~/openfs-mount
```

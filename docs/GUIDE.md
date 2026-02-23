# OpenFS Guide

OpenFS is a virtual filesystem for agent workflows.

## What OpenFS does

- Maps multiple storage backends into one namespace.
- Routes operations by longest mount-path prefix.
- Supports cache/sync strategies per mount.
- Provides grep and semantic search.
- Exposes an MCP tool server.
- Mounts via FUSE on macOS/Linux.

## CLI DX Quickstarts

- `docs/openfs-local.md` - local indexing + semantic search dev loop
- `docs/openfs-remote.md` - VFS operations and remote-interface workflow
- `docs/openfs-local-remote.md` - local-first write-back with explicit flush control

## Minimal config

```yaml
name: demo

backends:
  local:
    type: fs
    root: ./data

mounts:
  - path: /workspace
    backend: local
```

## Core operations

```bash
openfs write /workspace/a.txt "hello"
openfs cat /workspace/a.txt
openfs ls /workspace
openfs cp /workspace/a.txt /workspace/b.txt
openfs mv /workspace/b.txt /workspace/c.txt
openfs rm /workspace/c.txt
```

## Search

```bash
openfs grep "hello" /workspace --recursive
openfs index /workspace
openfs search "where is greeting logic" --limit 10
```

## Watch config

`openfs watch` supports defaults from config (`defaults.watch` or `mounts[].watch`):

```yaml
defaults:
  watch:
    native: true
    poll_interval: 2s
    debounce: 500ms
    auto_index: true
    include:
      - "^/workspace/.*\\.rs$"
    exclude:
      - "/target/"
```

CLI flags still override config values.

## Backends

Supported backend types:
- `fs`
- `memory`
- `s3` (build `openfs-cli` with `--features openfs-remote/s3`)
- `postgres` (build `openfs-cli` with `--features openfs-remote/postgres`)
- `chroma`

Example mixed configuration:

```yaml
name: mixed

backends:
  code:
    type: fs
    root: ./src
  docs:
    type: s3
    bucket: team-docs
    region: us-east-1
  records:
    type: postgres
    connection_url: ${DATABASE_URL}

mounts:
  - path: /code
    backend: code

  - path: /docs
    backend: docs
    read_only: true

  - path: /records
    backend: records
```

## MCP

Run the MCP server over stdio:

```bash
openfs mcp
```

## Sync Control

For write-back mounts:

```bash
openfs sync status
openfs sync flush
```

## FUSE

```bash
openfs --config openfs.yaml mount ~/openfs-mount
openfs unmount ~/openfs-mount
```

## Notes

- Semantic indexing and search are implemented in `openfs-local`.
- There is no standalone `openfs-indexing` crate in the active workspace surface.

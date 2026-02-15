# AX Guide

AX is a virtual filesystem for agent workflows.

## What AX does

- Maps multiple storage backends into one namespace.
- Routes operations by longest mount-path prefix.
- Supports cache/sync strategies per mount.
- Provides grep and semantic search.
- Exposes an MCP tool server.
- Mounts via FUSE on macOS/Linux.

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
ax write /workspace/a.txt "hello"
ax cat /workspace/a.txt
ax ls /workspace
ax cp /workspace/a.txt /workspace/b.txt
ax mv /workspace/b.txt /workspace/c.txt
ax rm /workspace/c.txt
```

## Search

```bash
ax grep "hello" /workspace --recursive
ax index /workspace
ax search "where is greeting logic" --limit 10
```

## Watch config

`ax watch` supports defaults from config (`defaults.watch` or `mounts[].watch`):

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
- `s3` (build `ax-cli` with `--features ax-remote/s3`)
- `postgres` (build `ax-cli` with `--features ax-remote/postgres`)
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
ax mcp
```

## FUSE

```bash
ax --config ax.yaml mount ~/ax-mount
ax unmount ~/ax-mount
```

## Notes

- Semantic indexing and search are implemented in `ax-local`.
- There is no standalone `ax-indexing` crate in the active workspace surface.

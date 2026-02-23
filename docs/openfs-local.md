# OpenFS Local DX (CLI)

This is the local developer loop for indexing and semantic retrieval.

## What this flow gives you

- fast local file iteration
- incremental re-indexing
- semantic search over your local workspace

## 1) Minimal local config

Create `openfs.yaml`:

```yaml
name: local-dev

backends:
  code:
    type: fs
    root: ./workspace

mounts:
  - path: /workspace
    backend: code
    mode: local_indexed
```

## 2) Index your workspace (incremental by default)

```bash
openfs index /workspace --recursive --incremental \
  --chroma-endpoint http://localhost:8000 \
  --collection local_dev
```

## 3) Run semantic search while coding

```bash
openfs search "where auth tokens are parsed" \
  --mode hybrid \
  --limit 10 \
  --chroma-endpoint http://localhost:8000 \
  --collection local_dev
```

## 4) Inspect index state

```bash
openfs index-status
```

## 5) Full rebuild when needed

```bash
openfs index /workspace --recursive --force \
  --chroma-endpoint http://localhost:8000 \
  --collection local_dev
```

## Notes

- `openfs search` currently requires `--chroma-endpoint`.
- If you run `openfs index` without `--chroma-endpoint`, indexing runs but results are not sent to Chroma for later CLI semantic search.

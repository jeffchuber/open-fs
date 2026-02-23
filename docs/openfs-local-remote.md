# OpenFS Local + Remote DX (CLI)

This is the local-first + flush-to-remote workflow.

## Goal

- keep local write performance with `write_back`
- explicitly flush buffered changes to remote when you choose

## 1) Configure a write-back mount

Create `openfs.yaml`:

```yaml
name: local-remote-dx

backends:
  remote_fs:
    type: fs
    root: ./remote-data

mounts:
  - path: /workspace
    backend: remote_fs
    mode: write_back
    sync:
      interval: 30s
```

## 2) Generate local changes

Use either AX commands or a mounted view. If using FUSE, writes come from any app/editor.

```bash
openfs write /workspace/notes.txt "first draft"
openfs append /workspace/notes.txt "\nnext line"
```

## 3) Check sync backlog

```bash
openfs sync status
```

## 4) Force flush local buffered changes to remote

```bash
openfs sync flush
openfs sync status
```

## 5) Verify remote state

For fs backends in this example:

```bash
cat ./remote-data/notes.txt
```

## Important behavior

- CLI mutating commands already trigger a flush at command completion.
- `openfs sync flush` is still the explicit drain control and is most useful when changes come from non-CLI writers (for example through FUSE-mounted workflows).

# OpenFS Remote DX (CLI)

This is the CLI-first workflow for using AX as a virtual filesystem over mounted backends.

## What this flow gives you

- one namespace over multiple backends
- normal file ops (`ls`, `cat`, `write`, `mv`, `rm`)
- regex search (`grep`) across mounts
- explicit sync status for write-back mounts

## 1) Minimal VFS config

Create `openfs.yaml`:

```yaml
name: remote-dx

backends:
  local:
    type: fs
    root: ./data

mounts:
  - path: /workspace
    backend: local
```

## 2) Core operational loop

```bash
openfs status
openfs ls /workspace
openfs write /workspace/hello.txt "hello"
openfs cat /workspace/hello.txt
openfs grep "hello" /workspace --recursive
openfs mv /workspace/hello.txt /workspace/hello-v2.txt
openfs rm /workspace/hello-v2.txt
```

## 3) Remote-only interface mode

For mounts that should behave as remote-only interfaces:

```yaml
mounts:
  - path: /remote
    backend: object_store
    mode: remote
```

Then operate through the same CLI surface:

```bash
openfs ls /remote
openfs cat /remote/path/to/file.txt
```

## 4) Validate and inspect config

```bash
openfs validate
openfs config
```

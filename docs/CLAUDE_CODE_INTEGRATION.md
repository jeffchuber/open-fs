# AX + Claude Code Integration Guide

This guide explains how to use AX's FUSE filesystem integration with Claude Code for transparent access to multi-backend storage with semantic search capabilities.

## Overview

AX exposes a FUSE (Filesystem in Userspace) mount that allows Claude Code to work with AX's virtual filesystem using standard file operations. This means:

- **Zero learning curve**: Claude Code uses its existing Read, Write, Glob, and Grep tools
- **Transparent multi-backend access**: Files can be backed by local filesystem, memory, S3, PostgreSQL, or Chroma
- **Per-mount sync strategies**: Different directories can have different caching and sync behaviors
- **Semantic search via filesystem**: Query indexed files by listing special `.search/` directories

## Prerequisites

### macOS

Install macFUSE (formerly OSXFUSE):

```bash
brew install --cask macfuse
```

After installation, you may need to allow the kernel extension:
1. Open System Preferences → Security & Privacy → General
2. Click "Allow" for the blocked macFUSE extension
3. Restart your computer

### Linux

Install FUSE3:

```bash
# Ubuntu/Debian
sudo apt install libfuse3-dev fuse3

# Fedora
sudo dnf install fuse3-devel fuse3

# Arch Linux
sudo pacman -S fuse3
```

## Quick Start

### 1. Create an AX Configuration

Create an `ax.yaml` file:

```yaml
name: claude-workspace

backends:
  local:
    type: fs
    root: ./data

mounts:
  - path: /workspace
    backend: local
```

### 2. Mount the VFS

```bash
# Mount AX VFS
ax mount ~/ax-mount --config ax.yaml

# The mount runs in the foreground by default
# Use Ctrl+C or `ax unmount` to unmount
```

### 3. Use with Claude Code

```bash
# Start Claude Code with the mounted directory
claude --working-dir ~/ax-mount/workspace

# Claude can now use standard file operations:
# - Read files with the Read tool
# - Write files with the Write tool
# - Search files with Glob and Grep
```

### 4. Unmount

```bash
# Graceful unmount
ax unmount ~/ax-mount

# Force unmount if busy
ax unmount ~/ax-mount --force
```

## Configuration Examples

### Basic Local Filesystem

```yaml
name: simple-project

backends:
  local:
    type: fs
    root: ./project-data

mounts:
  - path: /workspace
    backend: local
```

### Multi-Backend with Sync Strategies

```yaml
name: claude-project

backends:
  local:
    type: fs
    root: ./data
  docs-bucket:
    type: s3
    bucket: company-docs
    region: us-east-1
  memories-bucket:
    type: s3
    bucket: claude-memories

mounts:
  # Project code: local with sync to S3
  - path: /workspace
    backend: local
    sync:
      mode: write_through
      remote: docs-bucket
    cache:
      enabled: true
      ttl_seconds: 300

  # Documentation: read-only from S3, cache aggressively
  - path: /docs
    backend: docs-bucket
    mode: pull_mirror
    cache:
      ttl_seconds: 3600

  # Agent memories: fast local, async backup
  - path: /memories
    backend: local
    sync:
      mode: write_back
      remote: memories-bucket
      flush_interval: 30
```

### Sync Mode Reference

| Mode | Read Behavior | Write Behavior | Best For |
|------|---------------|----------------|----------|
| `write_through` | Cache → Backend | Cache + Backend (sync) | Strong consistency |
| `write_back` | Cache → Backend | Cache, then async flush | Low latency writes |
| `pull_mirror` | Cache → Backend (fetch on miss) | READ-ONLY | Remote documentation |

## Semantic Search via Filesystem

AX exposes semantic search through a virtual `.search/` directory. This allows Claude Code to perform semantic searches using standard filesystem operations.

### Directory Structure

```
/.search/
├── query/                 # Search by reading/listing this path
│   └── {url-encoded-query}/
│       ├── 01_file.py -> ../../workspace/src/file.py
│       └── 02_other.rs -> ../../workspace/src/other.rs
└── recent/               # Recent search results (future)
```

### Example: Searching for Authentication Code

```bash
# First, index your files
ax index /workspace

# Then search via filesystem
ls ~/ax-mount/.search/query/how+does+authentication+work/

# Results appear as symlinks:
# 01_auth.py -> ../../workspace/src/auth.py
# 02_login.py -> ../../workspace/src/login.py

# Read the search results
cat ~/ax-mount/.search/query/authentication/01_*
```

### How Claude Code Uses This

Claude Code can search semantically by using Glob on the search directory:

```
Claude: *uses Glob("/.search/query/jwt+token+validation/*")*
# Returns symlinks to matching files

Claude: *reads the symlink targets*
# Gets actual file content with context
```

## Troubleshooting

### Mount Fails with "Operation not permitted"

On macOS, ensure macFUSE kernel extension is allowed:
1. System Preferences → Security & Privacy → General
2. Look for blocked macFUSE extension and click "Allow"

### Mount Point Not Empty

Create a new, empty directory for the mount point:

```bash
mkdir -p ~/ax-mount
ax mount ~/ax-mount --config ax.yaml
```

### Permission Denied on Linux

Add your user to the `fuse` group:

```bash
sudo usermod -aG fuse $USER
# Log out and back in for changes to take effect
```

### Unmount Hangs

Force unmount:

```bash
# macOS
umount -f ~/ax-mount

# Linux
fusermount -uz ~/ax-mount
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     Claude Code                          │
│                                                          │
│   Read("/ax/workspace/auth.py")                         │
│   Write("/ax/workspace/test.py", code)                  │
│   Glob("/.search/query/authentication/*")               │
│                           │                              │
│           Claude thinks   │   these are normal files     │
└───────────────────────────┼─────────────────────────────┘
                            │
                    ┌───────▼───────┐
                    │  FUSE Mount   │  ~/ax-mount
                    │   (ax-fuse)   │
                    └───────┬───────┘
                            │
                    ┌───────▼───────┐
                    │    AX VFS     │
                    │   (Router)    │
                    └───────┬───────┘
                            │
    ┌───────────────────────┼───────────────────────┐
    │                       │                       │
    ▼                       ▼                       ▼
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│ /workspace  │     │   /docs     │     │ /.search    │
│   (local)   │     │    (S3)     │     │  (virtual)  │
└─────────────┘     └─────────────┘     └─────────────┘
```

## Best Practices

1. **Use write-through for critical data**: Ensures immediate consistency with remote storage
2. **Use pull-mirror for documentation**: Read-only access with aggressive caching
3. **Use write-back for logs/memories**: Fast local writes with eventual sync
4. **Index important directories**: Enable semantic search for code exploration
5. **Set appropriate cache TTLs**: Balance freshness vs. performance

## API Reference

### CLI Commands

```bash
# Mount filesystem
ax mount <mountpoint> [--config <path>]

# Unmount filesystem
ax unmount <mountpoint> [--force]

# Index files for semantic search
ax index <path> [--incremental] [--force]

# Check index status
ax index-status

# Search indexed files
ax search <query> [--limit <n>]

# Watch for changes and auto-index
ax watch
```

### Configuration Schema

See `ax/examples/` for complete configuration examples.

# @open-fs/core

Typed TypeScript client for [OpenFS](../README.md), a virtual filesystem for AI agents and automation. This package provides a `Vfs` interface that talks to the OpenFS Rust binary over MCP (Model Context Protocol) via stdio, plus an in-memory implementation for development and testing.

## Install

```bash
npm install @open-fs/core
```

You also need the `openfs` binary on your PATH (or specify the path explicitly):

```bash
cargo install --path crates/openfs-cli
```

## Quick Start

### With the Rust binary (production)

```typescript
import { createVfs } from "@open-fs/core";

const vfs = await createVfs();

await vfs.write("/notes/todo.md", "# TODO\n- Ship it");
const content = await vfs.read("/notes/todo.md");

const entries = await vfs.list("/notes");
console.log(entries); // [{ path: "/notes/todo.md", name: "todo.md", is_dir: false, size: 21, modified: "..." }]

await vfs.close();
```

### With the in-memory backend (dev/testing)

```typescript
import { createMemoryVfs } from "@open-fs/core";

const vfs = createMemoryVfs();

await vfs.write("/test/hello.txt", "world");
console.log(await vfs.read("/test/hello.txt")); // "world"
```

## API

### Factory Functions

#### `createVfs(options?): Promise<Vfs>`

Spawns the `openfs mcp` subprocess and returns a connected `Vfs`. Options:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `openFsBinary` | `string` | `"openfs"` | Path to the openfs binary |
| `configPath` | `string` | — | Path to a YAML config file |
| `cwd` | `string` | — | Working directory for the subprocess |

#### `createMemoryVfs(): Vfs`

Returns an in-memory `Vfs` that requires no subprocess. Useful for unit tests and local development. Does not support `search()` (always returns `[]`).

### Vfs Interface

All methods are async and use POSIX-style absolute paths.

#### File Operations

```typescript
vfs.read(path: string): Promise<string>
vfs.write(path: string, content: string): Promise<void>
vfs.append(path: string, content: string): Promise<void>
vfs.delete(path: string): Promise<void>
vfs.rename(from: string, to: string): Promise<void>
vfs.exists(path: string): Promise<boolean>
vfs.stat(path: string): Promise<Entry>
vfs.list(path: string): Promise<Entry[]>
```

#### Search

```typescript
// Regex grep across files
vfs.grep(pattern: string, path?: string): Promise<GrepMatch[]>

// Semantic search (requires indexing + embeddings configured in openfs)
vfs.search(query: string, limit?: number): Promise<SearchResult[]>
```

#### Batch Operations

```typescript
vfs.readBatch(paths: string[]): Promise<Map<string, string>>
vfs.writeBatch(files: { path: string; content: string }[]): Promise<void>
vfs.deleteBatch(paths: string[]): Promise<void>
```

#### Cache

```typescript
vfs.prefetch(paths: string[]): Promise<{ prefetched: number; errors: number }>
vfs.cacheStats(): Promise<CacheStats>
```

#### Lifecycle

```typescript
vfs.close(): Promise<void>
```

### Types

```typescript
interface Entry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number | null;
  modified: string | null;
}

interface GrepMatch {
  path: string;
  line_number: number;
  line: string;
}

interface SearchResult {
  score: number;
  source: string;
  snippet: string;
}

interface CacheStats {
  hits: number;
  misses: number;
  hit_rate: number;
  entries: number;
  size: number;
  evictions: number;
}
```

### Errors

Errors follow POSIX conventions with a `code` property:

| Code | Function | Meaning |
|------|----------|---------|
| `ENOENT` | `enoent(path)` | File or directory not found |
| `EISDIR` | `eisdir(path)` | Illegal operation on a directory |
| `ENOTDIR` | `enotdir(path)` | Not a directory |
| `EIO` | `eio(msg)` | I/O or transport error |
| `ENOTSUP` | `enotsup(op)` | Operation not supported |
| `EEXIST` | `eexist(path)` | File already exists |

```typescript
import { enoent, type VfsError } from "@open-fs/core";

try {
  await vfs.read("/missing");
} catch (err) {
  const e = err as VfsError;
  console.log(e.code); // "ENOENT"
  console.log(e.path); // "/missing"
}
```

## Configuration

`createVfs({ configPath: "./openfs.yaml" })` loads a YAML config with `${ENV_VAR}` and `${ENV_VAR:-default}` interpolation:

```yaml
backends:
  local:
    type: fs
    root: ./data

  docs_s3:
    type: s3
    bucket: my-docs
    prefix: v2
    region: us-east-1

  knowledge:
    type: chroma
    url: http://localhost:8000
    collection: codebase

mounts:
  - path: /workspace
    backend: local
    mode: write_through

  - path: /docs
    backend: docs_s3
    mode: remote_cached

  - path: /knowledge
    backend: knowledge
    mode: write_back
```

See the [Getting Started guide](../docs/GETTING_STARTED.md) for full config reference.

## How It Works

`SubprocessVfs` spawns the `openfs mcp` command as a child process and communicates over JSON-RPC 2.0 (MCP protocol) via stdin/stdout. Each `Vfs` method maps to a `tools/call` request targeting the corresponding `openfs_*` tool.

```
TypeScript (your code)
  → @open-fs/core (Vfs interface)
    → SubprocessVfs (JSON-RPC over stdio)
      → openfs mcp (Rust binary)
        → VFS router → backend (fs, s3, postgres, chroma, ...)
```

`MemoryVfs` is a standalone in-memory implementation of the same `Vfs` interface with no external dependencies.

## Development

```bash
cd ts
npm install
npm run typecheck
npm test
```

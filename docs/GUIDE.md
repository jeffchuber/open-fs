# AX: A Gentle Introduction

AX is a virtual filesystem for AI agents. It lets you mount local directories, S3 buckets, databases, and vector stores into a single unified file tree — then read, write, search, and manage files across all of them with one consistent API.

You can talk to AX from the CLI, from Python, from TypeScript, over REST, or through MCP. This guide starts with the simplest possible setup and adds capabilities one at a time.

---

## Install

```bash
git clone https://github.com/ax-vfs/ax.git
cd ax
cargo build --release
cargo install --path crates/ax-cli
```

Verify it works:

```bash
ax --version
```

---

## Your first config

AX needs a YAML file that tells it where your data lives and how to expose it. The smallest useful config is three lines of substance:

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

Save this as `ax.yaml`. Then create the data directory:

```bash
mkdir -p data
```

That's it. You have a virtual filesystem with one mount point (`/files`) backed by the `./data` directory on disk.

```
What you see            Where it actually lives
-----------             ----------------------
/files/                 ./data/
/files/notes.txt        ./data/notes.txt
/files/sub/readme.md    ./data/sub/readme.md
```

---

## Using the CLI

Now try it out:

```bash
# Write a file
ax write /files/hello.txt "Hello, world!"

# Read it back
ax cat /files/hello.txt

# List the directory
ax ls /files

# Check if it exists
ax exists /files/hello.txt

# Append to it
ax append /files/hello.txt " And goodbye."

# See file metadata
ax stat /files/hello.txt
```

All of these operations go through the VFS — AX resolves `/files/hello.txt` to `./data/hello.txt` via the mount config. Simple so far, but this indirection is what makes everything else possible.

---

## Adding a second mount

The real value shows up when you have multiple data sources. Add a second backend:

```yaml
name: two-mounts

backends:
  workspace:
    type: fs
    root: ./workspace
  reference:
    type: fs
    root: ./reference

mounts:
  - path: /work
    backend: workspace
  - path: /ref
    backend: reference
    read_only: true
```

```bash
mkdir -p workspace reference
echo "Look up the API docs" > reference/api.md
```

Now the agent (or you) can read from `/ref/api.md` and write to `/work/` — and the reference material can't be accidentally modified because it's mounted read-only.

```
/work/           <-- read/write, backed by ./workspace/
/ref/            <-- read-only,  backed by ./reference/
```

---

## Searching with grep

AX has built-in regex search across your virtual filesystem:

```bash
# Search for a pattern in a directory
ax grep "TODO" --path /work --recursive

# Find files by name pattern
ax find "\.md$" --path /work
```

This works the same regardless of what backend is behind the mount — local files, S3, whatever.

---

## Using AX from Python

Install the Python bindings:

```bash
cd crates/ax-ffi
maturin develop
```

Then:

```python
import ax

vfs = ax.load_config("""
name: python-demo
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /files
    backend: local
""")

# Write and read
vfs.write_text("/files/hello.txt", "Hello from Python!")
print(vfs.read_text("/files/hello.txt"))

# List a directory
for entry in vfs.list("/files"):
    print(f"  {entry.name}  ({'dir' if entry.is_dir else str(entry.size) + 'b'})")

# Grep
for match in vfs.grep("Hello", "/files", recursive=True):
    print(f"  {match.path}:{match.line_number}  {match.line}")
```

You can also load a config from a file:

```python
vfs = ax.load_config_file("ax.yaml")
```

---

## Using AX from TypeScript / Node.js

Build the Node bindings:

```bash
cd crates/ax-js
npm install
npm run build
```

Then:

```typescript
const { loadConfig } = require('ax-vfs');

const vfs = loadConfig(`
name: node-demo
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /files
    backend: local
`);

// Write and read
vfs.writeText('/files/hello.txt', 'Hello from Node!');
console.log(vfs.readText('/files/hello.txt'));

// List a directory
for (const entry of vfs.list('/files')) {
  console.log(`  ${entry.name}  ${entry.isDir ? 'dir' : entry.size + 'b'}`);
}

// Grep
const matches = vfs.grep('Hello', '/files', true);
matches.forEach(m => console.log(`  ${m.path}:${m.lineNumber}  ${m.line}`));
```

---

## Using the REST API

Start the server:

```bash
ax serve --port 19557
```

Now you can talk to AX over HTTP:

```bash
# Write a file
curl -X POST http://localhost:19557/v1/write \
  -H "Content-Type: application/json" \
  -d '{"path": "/files/hello.txt", "content": "Hello from curl!"}'

# Read it back
curl "http://localhost:19557/v1/read?path=/files/hello.txt"

# List a directory
curl "http://localhost:19557/v1/ls?path=/files"

# Grep
curl "http://localhost:19557/v1/grep?pattern=Hello&path=/files"

# Check if a file exists
curl "http://localhost:19557/v1/exists?path=/files/hello.txt"
```

The full OpenAPI spec is available at `http://localhost:19557/v1/openapi`.

To require an API key (recommended; required when binding to a non-local host):

```bash
ax serve --port 19557 --api-key "my-secret-key"

# Then pass it in requests:
curl -H "Authorization: Bearer my-secret-key" \
  "http://localhost:19557/v1/read?path=/files/hello.txt"
```

---

## Using MCP (for AI agents)

AX speaks the Model Context Protocol, so LLM tool-use frameworks can call it directly:

```bash
ax mcp
```

This starts a JSON-RPC 2.0 server over stdio that exposes these tools:

| Tool | What it does |
|------|-------------|
| `ax_read` | Read a file |
| `ax_write` | Write a file |
| `ax_ls` | List a directory |
| `ax_stat` | Get file metadata |
| `ax_delete` | Delete a file |
| `ax_grep` | Regex search across files |
| `ax_search` | Semantic search (requires indexing) |

Any MCP-compatible agent framework can discover and call these tools automatically.

---

## Cloud backends

So far we've used local `fs` backends. AX supports several remote backends that work the same way — mount them, and every operation (CLI, Python, TypeScript, REST, MCP) works identically.

### Memory

```yaml
backends:
  mem:
    type: memory
```

Use the in-memory backend for tests, demos, or short-lived data.

### S3

```yaml
backends:
  s3:
    type: s3
    bucket: my-data-bucket
    region: us-east-1
    prefix: project/          # optional: only expose a key prefix

mounts:
  - path: /cloud
    backend: s3
```

Works with AWS S3, MinIO, Cloudflare R2, and any S3-compatible service. For non-AWS endpoints:

```yaml
backends:
  minio:
    type: s3
    bucket: local-bucket
    endpoint: http://localhost:9000
```

### PostgreSQL

```yaml
backends:
  db:
    type: postgres
    connection_url: ${DATABASE_URL}
    table_name: ax_files
```

Files are stored as rows in a table. Useful for structured data that agents need to read/write.

### Chroma (vector database)

```yaml
backends:
  vectors:
    type: chroma
    url: http://localhost:8000
    collection: my_embeddings
```

---

## Mixing backends

The real power is combining backends in one config. Each mount is independent — reads and writes route to the right backend based on the path.

```yaml
name: mixed-workspace

backends:
  code:
    type: fs
    root: ./src
  docs:
    type: s3
    bucket: team-docs
    region: us-east-1
  db:
    type: postgres
    connection_url: ${DATABASE_URL}

mounts:
  - path: /code
    backend: code

  - path: /docs
    backend: docs
    read_only: true

  - path: /data
    backend: db
    read_only: true
```

```
             AX VFS
               |
    +----------+----------+
    |          |          |
  /code      /docs      /data
    |          |          |
  local      S3       Postgres
  ./src    team-docs   ax_files
```

From the agent's perspective, `ax cat /docs/readme.md` and `ax cat /code/main.rs` look exactly the same. AX handles the routing.

---

## Environment variables

Any value in the config can reference environment variables:

```yaml
backends:
  s3:
    type: s3
    bucket: ${MY_BUCKET}
    region: ${AWS_REGION}
```

This lets you use the same config across environments (dev, staging, prod) by changing the env vars.

---

## Generating tool definitions

If you're building an AI agent and want to give it AX as a tool, you can auto-generate tool definitions:

```bash
# MCP format
ax tools --format mcp

# OpenAI function-calling format
ax tools --format openai

# Raw JSON
ax tools --format json
```

From Python:

```python
tools_json = vfs.tools("openai")
# Pass this to your LLM's tool definitions
```

From TypeScript:

```typescript
const tools = vfs.tools('mcp');
```

---

## What's next

This guide covered the basics. For more:

- **[USE_CASES.md](USE_CASES.md)** — Five real-world patterns for AI agents (memory, skills, code generation, data fabric, multi-agent collaboration)
- **[GETTING_STARTED.md](GETTING_STARTED.md)** — FUSE mounting for Claude Code integration
- **[ARCHITECTURE.md](ARCHITECTURE.md)** — How AX works under the hood
- **[PROJECT_STATUS.md](PROJECT_STATUS.md)** — Current development status

The CLI has built-in help for every command:

```bash
ax --help
ax write --help
ax grep --help
```

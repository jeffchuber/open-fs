# AX Use Cases: AI Agents & Data Management

AX gives AI agents a unified filesystem that spans local storage, cloud buckets, vector databases, and more — accessible via REST API, MCP, Python, TypeScript, or CLI. Below are five key use cases centered on how agents manage data, state, and collaboration.

---

## 1. Agent Long-Term Memory

AI agents need persistent memory that survives across sessions, scales beyond context windows, and can be selectively recalled. AX provides this as a structured filesystem with semantic search.

**How it works:** An agent mounts a local backend for fast journaling and a Chroma backend for embedding-indexed recall. During a session, the agent appends observations, decisions, and user preferences to files organized by topic (`/memory/user_prefs.md`, `/memory/project_context/acme.md`). Between sessions, the agent searches its memory store with natural language queries to pull in only the context it needs — no full replay of prior conversations required.

```
                           +-----------+
                           |   Agent   |
                           +-----+-----+
                                 |
                  write / append | search("user prefs")
                                 |
                  +--------------+--------------+
                  |           AX VFS            |
                  +--------------+--------------+
                  |                             |
          /memory |                     /recall |
       (fs mount) |              (chroma mount) |
                  v                             v
  +---+-----------+----------+    +-------------+---------+
  |   ~/.agent/memory/       |    |   Chroma @ :8000      |
  |                          |    |                       |
  |   facts/                 |    |   collection:         |
  |     user.md         -----+--->|     agent_memory      |
  |     project.md       auto|    |                       |
  |   sessions/          index    |   [embeddings]        |
  |     2025-01-06.md    ----+--->|   [embeddings]        |
  |     2025-01-07.md        |    |   [embeddings]        |
  +---+----------------------+    +---+-------------------+
```

**What AX brings:**
- **Append-only journaling** — `append()` streams new observations without rewriting files
- **Semantic retrieval** — `/search` finds relevant memories by meaning, not just keyword
- **Unified namespace** — flat files and vector embeddings live under the same `/memory` mount, navigable with `ls`, `read`, and `grep`

**Example config** — `memory-agent.yaml`:
```yaml
name: agent-memory

backends:
  journal:
    type: fs
    root: ${AGENT_DATA_DIR:-~/.agent}/memory
  vectors:
    type: chroma
    url: http://localhost:8000
    collection: agent_memory

mounts:
  - path: /memory
    backend: journal
    collection: memory
    index:
      enabled: true
      search_modes: [hybrid]
      chunk:
        strategy: recursive
        size: 500
        overlap: 50
      embedding:
        provider: ollama
        model: nomic-embed-text
        dimensions: 768
    watch:
      native: true
      debounce: 500ms
      auto_index: true

  - path: /recall
    backend: vectors
    read_only: true

defaults:
  sync:
    interval: 5s
```

**Example flow:**
```
POST /write  {"path": "/memory/facts/user.md", "content": "Prefers concise answers. Timezone: PST."}
POST /append {"path": "/memory/sessions/2025-01-06.md", "content": "Discussed Q4 budget..."}
GET  /search {"query": "user communication preferences", "limit": 5}
```

---

## 2. Agent Skill & Tool Library

Agents that can write, store, and reuse code — tool definitions, API wrappers, data transforms — become dramatically more capable over sessions. AX acts as the agent's skill repository.

**How it works:** When an agent writes a useful function (a Slack notifier, a CSV parser, a SQL query builder), it saves it to `/skills/` as an executable artifact. Tool definitions are stored alongside implementations. Before tackling a new task, the agent greps its skill library for relevant prior work.

```
                           +-----------+
                           |   Agent   |
                           +-----+-----+
                                 |
             +-------------------+--------------------+
             |                   |                    |
        1. grep()          2. write()           3. tools()
     "Do I already       "Save new skill"     "What can I do?"
      have this?"              |                    |
             |                 v                    v
             |    /skills/                   +------+------+
             |      slack_notify.py          | Tool Manifest|
             |      slack_notify.tool.json   | (MCP/OpenAI) |
             |      csv_parser.py            +-------------+
             |      csv_parser.tool.json
             |      sql_builder.py
             |
             v
    +--------+--------+
    | Search Results   |
    | - slack_notify   |
    | - csv_parser     |
    +--------+--------+
             |
      "Found it, reuse"

             /docs/ (read-only)
               api_reference.md
               style_guide.md
               patterns.md
```

**What AX brings:**
- **`tools()` generation** — auto-generates MCP/OpenAI-format tool manifests from the VFS contents, so agents can dynamically discover their own capabilities
- **Grep for reuse** — `grep("fetch.*api", "/skills", recursive=True)` finds relevant skills before writing new ones
- **Cross-format storage** — Python scripts, JSON schemas, YAML configs, and documentation coexist in one tree

**Example config** — `skills-agent.yaml`:
```yaml
name: agent-skills

backends:
  skills_store:
    type: fs
    root: ${AGENT_DATA_DIR:-~/.agent}/skills
  docs_store:
    type: fs
    root: ${AGENT_DATA_DIR:-~/.agent}/docs

mounts:
  - path: /skills
    backend: skills_store
    collection: skills
    index:
      enabled: true
      search_modes: [dense, sparse]
      chunk:
        strategy: ast
        size: 2000
        overlap: 200
        granularity: function
      embedding:
        provider: ollama
        model: nomic-embed-text
        dimensions: 768
    watch:
      native: true
      debounce: 1s
      auto_index: true
      include: ["*.py", "*.js", "*.ts", "*.json"]

  - path: /docs
    backend: docs_store
    read_only: true
    collection: docs
    index:
      enabled: true
      search_modes: [hybrid]
      chunk:
        strategy: semantic
        size: 1000
        overlap: 100
```

**Example flow:**
```python
import ax
vfs = ax.load_config_file("skills-agent.yaml")
vfs.write_text("/skills/slack_notify.py", code)
vfs.write_text("/skills/slack_notify.tool.json", tool_def)
matches = vfs.grep("slack", "/skills", recursive=True)
```

---

## 3. Agentic Code Generation & Iteration

When agents generate, test, and refine code — whether building a feature, fixing a bug, or scaffolding a project — they need a workspace they can read, write, and search without touching the real filesystem until ready.

**How it works:** A coding agent gets a task ("build a REST API for inventory management"). It mounts a sandboxed workspace via AX, scaffolds files, and iterates on implementation. Once validated, the final state can be exported or synced to the real project directory.

```
  /reference (read-only)              /workspace (sandbox)
  +----------------------+            +----------------------+
  |  Real project code   |            |  Agent's working     |
  |  ${PROJECT_ROOT}     |   read     |  /tmp/ax-sandbox/    |
  |                      +----------->|                      |
  |  src/                |  reference |  src/                |
  |    models.py         |            |    models.py         |
  |    routes.py         |            |    routes.py         |
  |    auth.py           |            |    auth.py           |
  +----------------------+            +----------------------+
```

**What AX brings:**
- **Isolated workspaces** — mount a temp backend so agent writes don't touch production code until explicitly synced
- **Copy and rename** — filesystem-level refactoring operations (`rename`, `copy`) that agents can invoke as tool calls
- **Full text search** — `grep` across the generated codebase to verify patterns, find TODOs, or check for consistency

**Example config** — `coding-agent.yaml`:
```yaml
name: coding-sandbox

backends:
  sandbox:
    type: fs
    root: /tmp/ax-sandbox/${SESSION_ID}
  reference:
    type: fs
    root: ${PROJECT_ROOT}

mounts:
  - path: /workspace
    backend: sandbox
    collection: workspace
    index:
      enabled: true
      search_modes: [sparse]
      chunk:
        strategy: ast
        size: 1500
        granularity: function

  - path: /reference
    backend: reference
    read_only: true
    collection: reference
    index:
      enabled: true
      search_modes: [dense]
      chunk:
        strategy: ast
        size: 1500
        granularity: function
      embedding:
        provider: ollama
        model: nomic-embed-text
        dimensions: 768

defaults:
  cache:
    enabled: true
    max_entries: 500
    max_size_bytes: 104857600
    ttl_seconds: 600
```

**Example flow:**
```
POST /write  {"path": "/workspace/src/models.py", "content": "..."}
POST /write  {"path": "/workspace/src/routes.py", "content": "..."}
POST /write  {"path": "/workspace/src/auth.py", "content": "..."}
GET  /grep   {"pattern": "TODO", "path": "/workspace", "recursive": true}
```

---

## 4. SaaS Data Fabric for Agent Workflows

Agents that interact with business data — customer records, documents, analytics, configs — face a fragmentation problem: data lives in S3 buckets, local caches, vector stores, and SaaS exports, each with different access patterns. AX unifies these into a single navigable tree.

**How it works:** An organization configures AX with multiple backends: S3 for document storage, local filesystem for cached datasets, and Chroma for semantic search over processed content. An agent tasked with "prepare the Q4 board report" navigates `/documents/financials/`, `/data/analytics/`, and searches `/knowledge/` for prior reports — all through the same API. It reads source material, generates the report, writes it back, and indexes it for future retrieval.

```
                            +------------------+
                            |      Agent       |
                            | "Prepare the Q4  |
                            |  board report"   |
                            +--------+---------+
                                     |
          vfs.read("/documents/financials/q4.xlsx")
          vfs.read("/records/customers")
          vfs.search("prior board reports")
          vfs.write("/cache/drafts/q4-report.md", report)
                                     |
       +-----------------------------+-----------------------------+
       |                          AX VFS                           |
       |                    (unified namespace)                    |
       +---------+-----------+-----------+------------+------------+
                 |           |           |            |
     /documents  |  /records |   /cache  | /knowledge |
                 |           |           |            |
                 v           v           v            v
       +---------+--+ +-----+----+ +----+-------+ +--+-----------+
       | S3          | | Postgres | | Local FS   | | Chroma       |
       |             | |          | |            | |              |
       | bucket:     | | table_name:   | | /var/cache | | collection:  |
       |  reports/   | | biz_data | | /agent/    | | biz_knowledge|
       |             | |          | |            | |              |
       | financials/ | | customers| | drafts/    | | [embeddings] |
       |   q4.xlsx   | | orders   | |  q4-rpt.md | | [embeddings] |
       |   q3.xlsx   | | invoices | |  notes.md  | | [embeddings] |
       +-------------+ +----------+ +------------+ +--------------+
              |                           ^
              |       remote_cached       |
              +-------> local cache ------+
                       (auto-synced)

       Access the same data from any surface:
       +----------+ +----------+ +--------+ +-----+ +-----+
       | REST API | | Python   | | TypeScript | MCP | | CLI |
       | :19557   | | import ax| | require(ax)| stdio| | ax  |
       +----------+ +----------+ +--------+ +-----+ +-----+
```

**What AX brings:**
- **Backend abstraction** — S3, local, Chroma, GCS, Azure Blob, WebDAV, SFTP, PostgreSQL all appear as directories in one namespace
- **Mount-based routing** — `/documents` routes to S3, `/cache` to local disk, `/knowledge` to Chroma — transparent to the agent
- **Caching layer** — frequently accessed S3 objects are cached locally, reducing latency for iterative agent access patterns
- **Environment interpolation** — config references like `${AWS_BUCKET}` resolve at runtime, so the same agent config works across dev/staging/prod
- **Access via any surface** — the same data is accessible from Python SDK, TypeScript SDK, REST API, MCP, or CLI depending on the agent framework

**Example config** — `data-fabric.yaml`:
```yaml
name: business-data-fabric

backends:
  s3_docs:
    type: s3
    bucket: ${DOCS_BUCKET}
    region: ${AWS_REGION:-us-east-1}
    prefix: reports/
  pg_records:
    type: postgres
    connection_url: ${DATABASE_URL}
    table_name: business_data
  local_cache:
    type: fs
    root: /var/cache/agent/data
  knowledge:
    type: chroma
    url: http://localhost:8000
    collection: business_knowledge

mounts:
  - path: /documents
    backend: s3_docs
    mode: remote_cached
    sync:
      interval: 5m
      write_mode: async

  - path: /records
    backend: pg_records
    read_only: true

  - path: /cache
    backend: local_cache

  - path: /knowledge
    backend: knowledge
    collection: knowledge
    index:
      enabled: true
      search_modes: [hybrid]
      chunk:
        strategy: semantic
        size: 1000
        overlap: 100
      embedding:
        provider: openai
        model: text-embedding-3-small
        dimensions: 1536

defaults:
  cache:
    enabled: true
    max_entries: 1000
    max_size_bytes: 536870912  # 512MB
    ttl_seconds: 3600
  sync:
    interval: 1m
```

---

## 5. Multi-Agent Collaboration & Shared State

Complex tasks benefit from multiple specialized agents — a researcher, a coder, a reviewer — working on shared artifacts. AX provides the coordination layer: shared state with WAL-based sync and an audit trail.

**How it works:** A planning agent decomposes a task and writes the plan to `/project/plan.md`. A research agent reads the plan, gathers data, and writes findings to `/project/research/`. A coding agent reads both and builds the implementation in `/project/src/`. The WAL-based sync engine ensures no writes are lost during concurrent access, with crash recovery built in.

```
                           +----------------+
                           |  Orchestrator  |
                           +-------+--------+
                                   |
                       assign tasks to agents
                                   |
           +-----------+----+  +--+------+  +----+-----------+
           |  Planner Agent |  | Research|  | Coding Agent   |
           +-------+--------+  | Agent  |  +-------+--------+
                   |            +---+----+          |
                   |                |               |
             write plan.md    write research/   write src/
                   |                |               |
                   v                v               v
  +----------------+----------------------------------------+
  |                     AX VFS (shared)                      |
  |                                                          |
  | /project/                                                |
  |   plan.md         (from planner)                         |
  |   tasks.md        (from planner)                         |
  |   research/                                              |
  |     findings.md   (from researcher)                      |
  |     data.json     (from researcher)                      |
  |   src/                                                   |
  |     models.py     (from coder)                           |
  |     routes.py     (from coder)                           |
  |     auth.py       (from coder)                           |
  +----------------------------------------------------------+
```

**What AX brings:**
- **WAL-based sync** — write-ahead log ensures no writes are lost during concurrent access, with crash recovery
- **Shared namespace** — all agents read from and write to the same VFS tree, with mount-based routing to appropriate backends
- **Access via any surface** — each agent can use whichever SDK fits its framework (Python, TypeScript, REST, MCP, CLI)

**Example config** — `multi-agent.yaml`:
```yaml
name: multi-agent-project

backends:
  shared:
    type: s3
    bucket: ${TEAM_BUCKET}
    region: us-west-2
    prefix: project-${PROJECT_ID}/
  local_workspace:
    type: fs
    root: ${AGENT_DATA_DIR:-/tmp/ax-agent}/workspace
  scratch:
    type: fs
    root: /tmp/ax-scratch

mounts:
  - path: /project
    backend: shared
    mode: write_through
    sync:
      interval: 5s
      write_mode: sync
    watch:
      native: true
      debounce: 500ms
      auto_index: true
    index:
      enabled: true
      search_modes: [sparse]
      chunk:
        strategy: recursive
        size: 1000
        overlap: 100

  - path: /workspace
    backend: local_workspace
    collection: workspace
    sync:
      interval: 10s
      write_mode: async

  - path: /scratch
    backend: scratch

defaults:
  cache:
    enabled: true
    max_entries: 200
    max_size_bytes: 52428800  # 50MB
    ttl_seconds: 300
```

**Example flow:**
```javascript
const ax = require('ax');
const vfs = ax.loadConfigFile('multi-agent.yaml');

// Planner writes the plan
vfs.writeText('/project/plan.md', '# Project Plan\n...');

// Researcher writes findings
vfs.writeText('/project/research/findings.md', '# Research\n...');

// Coder reads plan + research, writes implementation
const plan = vfs.readText('/project/plan.md');
const research = vfs.readText('/project/research/findings.md');
vfs.writeText('/project/src/models.py', generatedCode);
```

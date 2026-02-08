# Coding Agent Example

A multi-tier storage architecture for an AI coding assistant demonstrating
how to organize skills, memories, code, and documentation with different
storage characteristics.

## Architecture

```
                           CODING AGENT VFS
    ┌─────────────────────────────────────────────────────────────┐
    │                                                             │
    │  ┌─────────────────────────────────────────────────────┐   │
    │  │                    HOT TIER                          │   │
    │  │              (Fast Local Storage)                    │   │
    │  │                                                      │   │
    │  │   /context     Active working context                │   │
    │  │   /scratch     Temporary workspace                   │   │
    │  │                                                      │   │
    │  │   • Sub-millisecond latency                         │   │
    │  │   • Frequent read/write                             │   │
    │  │   • Session-scoped data                             │   │
    │  └─────────────────────────────────────────────────────┘   │
    │                           │                                 │
    │                           ▼                                 │
    │  ┌─────────────────────────────────────────────────────┐   │
    │  │                   WARM TIER                          │   │
    │  │           (Indexed Knowledge Base)                   │   │
    │  │                                                      │   │
    │  │   /skills      Reusable capabilities                 │   │
    │  │   /memories    Conversation history                  │   │
    │  │   /code        Code snippets & templates             │   │
    │  │   /docs        Documentation (read-only)             │   │
    │  │                                                      │   │
    │  │   • Semantic search enabled                         │   │
    │  │   • Chunked and embedded                            │   │
    │  │   • Persistent across sessions                      │   │
    │  └─────────────────────────────────────────────────────┘   │
    │                           │                                 │
    │                           ▼                                 │
    │  ┌─────────────────────────────────────────────────────┐   │
    │  │                   COLD TIER                          │   │
    │  │              (Long-term Archive)                     │   │
    │  │                                                      │   │
    │  │   /archive     Historical data (S3)                  │   │
    │  │                                                      │   │
    │  │   • Read-only access                                │   │
    │  │   • Cost-optimized storage                          │   │
    │  │   • Infrequent access                               │   │
    │  └─────────────────────────────────────────────────────┘   │
    │                                                             │
    └─────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
/
├── context/                    # Hot: Active working context
│   ├── current_task.md         # Current task description
│   ├── plan.md                 # Execution plan
│   ├── state.json              # Agent state
│   └── files/                  # Files being worked on
│       ├── main.py
│       └── test_main.py
│
├── scratch/                    # Hot: Temporary workspace
│   ├── draft_1.py              # Work in progress
│   └── experiments/            # Experimental code
│
├── skills/                     # Warm: Reusable capabilities
│   ├── code_review.md          # Code review skill
│   ├── refactoring.md          # Refactoring patterns
│   ├── testing.md              # Testing strategies
│   ├── debugging.md            # Debugging techniques
│   └── tools/                  # Tool definitions
│       ├── git.json            # Git operations
│       ├── python.json         # Python tools
│       └── shell.json          # Shell commands
│
├── memories/                   # Warm: Learned patterns
│   ├── conversations/          # Past interactions
│   │   ├── 2024-01-15.jsonl
│   │   └── 2024-01-16.jsonl
│   ├── patterns/               # Learned patterns
│   │   ├── user_preferences.md
│   │   └── project_conventions.md
│   └── feedback/               # User feedback
│       └── corrections.jsonl
│
├── code/                       # Warm: Code knowledge
│   ├── snippets/               # Reusable snippets
│   │   ├── python/
│   │   ├── rust/
│   │   └── typescript/
│   ├── templates/              # Project templates
│   │   ├── fastapi/
│   │   └── cli/
│   └── examples/               # Reference examples
│       └── async_patterns.py
│
├── docs/                       # Warm: Documentation (read-only)
│   ├── python/                 # Python docs
│   ├── rust/                   # Rust docs
│   └── libraries/              # Library docs
│       ├── fastapi.md
│       └── tokio.md
│
└── archive/                    # Cold: Historical data
    ├── 2023/
    │   └── projects/
    └── 2024/
        └── projects/
```

## Usage

### Setup

```bash
# Create data directories
mkdir -p data/local data/knowledge

# Set environment variables (optional)
export AGENT_DATA_DIR=./data

# For S3 cold storage (optional)
export S3_BUCKET=my-agent-archive
export AWS_ACCESS_KEY_ID=...
export AWS_SECRET_ACCESS_KEY=...
```

### CLI Usage

```bash
# Initialize context for a new task
ax write /context/current_task.md "Implement user authentication"

# Save a skill
ax write /skills/auth_patterns.md "$(cat << 'EOF'
# Authentication Patterns

## JWT Authentication
...

## OAuth2 Flow
...
EOF
)"

# Record a memory
ax append /memories/conversations/$(date +%Y-%m-%d).jsonl \
  '{"role": "user", "content": "Use bcrypt for passwords"}'

# Store a code snippet
ax write /code/snippets/python/jwt_auth.py "$(cat my_auth.py)"

# Search for relevant code
ax search "JWT token validation" --path /code --limit 5

# Use scratch for experiments
ax write /scratch/experiment.py "# Quick test..."
ax rm /scratch/experiment.py  # Clean up
```

### Python Usage

```python
import ax
import json
from datetime import datetime

# Load the coding agent VFS
vfs = ax.load_config_file("ax.yaml")

# =============================================================================
# CONTEXT MANAGEMENT
# =============================================================================

def start_task(task_description: str, files: list[str]):
    """Initialize context for a new coding task."""
    # Save task description
    vfs.write_text("/context/current_task.md", task_description)

    # Initialize plan
    vfs.write_text("/context/plan.md", "# Execution Plan\n\n- [ ] Analyze task\n")

    # Initialize state
    state = {
        "task": task_description,
        "started_at": datetime.now().isoformat(),
        "status": "in_progress",
        "files": files
    }
    vfs.write_text("/context/state.json", json.dumps(state, indent=2))

def get_current_context() -> dict:
    """Get the current working context."""
    return {
        "task": vfs.read_text("/context/current_task.md"),
        "plan": vfs.read_text("/context/plan.md"),
        "state": json.loads(vfs.read_text("/context/state.json"))
    }

# =============================================================================
# SKILLS MANAGEMENT
# =============================================================================

def load_skill(skill_name: str) -> str:
    """Load a skill definition."""
    return vfs.read_text(f"/skills/{skill_name}.md")

def save_skill(skill_name: str, content: str):
    """Save a new or updated skill."""
    vfs.write_text(f"/skills/{skill_name}.md", content)

def list_skills() -> list[str]:
    """List all available skills."""
    entries = vfs.list("/skills")
    return [e.name for e in entries if not e.is_dir and e.name.endswith(".md")]

# =============================================================================
# MEMORY MANAGEMENT
# =============================================================================

def record_conversation(role: str, content: str):
    """Record a conversation turn."""
    today = datetime.now().strftime("%Y-%m-%d")
    entry = json.dumps({
        "timestamp": datetime.now().isoformat(),
        "role": role,
        "content": content
    })
    vfs.append_text(f"/memories/conversations/{today}.jsonl", entry + "\n")

def get_recent_conversations(days: int = 7) -> list[dict]:
    """Get recent conversation history."""
    conversations = []
    entries = vfs.list("/memories/conversations")

    for entry in sorted(entries, key=lambda e: e.name, reverse=True)[:days]:
        content = vfs.read_text(f"/memories/conversations/{entry.name}")
        for line in content.strip().split("\n"):
            if line:
                conversations.append(json.loads(line))

    return conversations

def save_pattern(name: str, pattern: str):
    """Save a learned pattern."""
    vfs.write_text(f"/memories/patterns/{name}.md", pattern)

# =============================================================================
# CODE MANAGEMENT
# =============================================================================

def save_snippet(language: str, name: str, code: str):
    """Save a code snippet."""
    vfs.write_text(f"/code/snippets/{language}/{name}", code)

def get_snippet(language: str, name: str) -> str:
    """Get a code snippet."""
    return vfs.read_text(f"/code/snippets/{language}/{name}")

def list_snippets(language: str) -> list[str]:
    """List snippets for a language."""
    entries = vfs.list(f"/code/snippets/{language}")
    return [e.name for e in entries if not e.is_dir]

# =============================================================================
# SCRATCH WORKSPACE
# =============================================================================

def save_draft(name: str, content: str):
    """Save a draft to scratch space."""
    vfs.write_text(f"/scratch/{name}", content)

def clear_scratch():
    """Clear all scratch files."""
    entries = vfs.list("/scratch")
    for entry in entries:
        if not entry.is_dir:
            vfs.delete(f"/scratch/{entry.name}")

# =============================================================================
# EXAMPLE USAGE
# =============================================================================

if __name__ == "__main__":
    # Start a new coding task
    start_task(
        "Implement JWT authentication for the FastAPI backend",
        files=["auth.py", "models.py", "test_auth.py"]
    )

    # Load relevant skill
    auth_skill = load_skill("auth_patterns")
    print(f"Loaded auth skill: {len(auth_skill)} chars")

    # Record the interaction
    record_conversation("user", "Implement JWT auth with refresh tokens")
    record_conversation("assistant", "I'll implement JWT authentication...")

    # Save a useful snippet discovered during the task
    save_snippet("python", "jwt_decode.py", '''
from jose import jwt, JWTError

def decode_token(token: str, secret: str) -> dict:
    """Decode and validate a JWT token."""
    try:
        payload = jwt.decode(token, secret, algorithms=["HS256"])
        return payload
    except JWTError:
        raise ValueError("Invalid token")
''')

    # Use scratch for experimentation
    save_draft("token_test.py", "# Testing token validation...")

    # Generate tools for AI
    tools = vfs.tools(format="openai")
    print(f"Generated {len(tools)} chars of tool definitions")
```

### TypeScript Usage

```typescript
import { loadConfigFile } from 'ax-vfs';

const vfs = loadConfigFile('ax.yaml');

// Context management
interface TaskContext {
  task: string;
  plan: string;
  state: {
    status: string;
    files: string[];
  };
}

function startTask(description: string, files: string[]): void {
  vfs.writeText('/context/current_task.md', description);
  vfs.writeText('/context/plan.md', '# Plan\n\n- [ ] Start');
  vfs.writeText('/context/state.json', JSON.stringify({
    task: description,
    started_at: new Date().toISOString(),
    status: 'in_progress',
    files
  }, null, 2));
}

function getContext(): TaskContext {
  return {
    task: vfs.readText('/context/current_task.md'),
    plan: vfs.readText('/context/plan.md'),
    state: JSON.parse(vfs.readText('/context/state.json'))
  };
}

// Memory management
function recordConversation(role: string, content: string): void {
  const today = new Date().toISOString().split('T')[0];
  const entry = JSON.stringify({
    timestamp: new Date().toISOString(),
    role,
    content
  });
  vfs.appendText(`/memories/conversations/${today}.jsonl`, entry + '\n');
}

// Skills
function loadSkill(name: string): string {
  return vfs.readText(`/skills/${name}.md`);
}

// Code snippets
function saveSnippet(lang: string, name: string, code: string): void {
  vfs.writeText(`/code/snippets/${lang}/${name}`, code);
}

// Example
startTask('Build REST API', ['api.ts', 'routes.ts']);
recordConversation('user', 'Add pagination to list endpoint');
saveSnippet('typescript', 'pagination.ts', `
interface PaginatedResult<T> {
  items: T[];
  total: number;
  page: number;
  pageSize: number;
}
`);

// Generate MCP tools for Claude
const tools = vfs.tools('mcp');
console.log('Tools:', tools);
```

## Data Tier Characteristics

| Tier | Mount Points | Storage | Latency | Indexing | Use Case |
|------|--------------|---------|---------|----------|----------|
| **Hot** | `/context`, `/scratch` | Local SSD | <1ms | Optional | Active work |
| **Warm** | `/skills`, `/memories`, `/code`, `/docs` | Local SSD | <5ms | Semantic | Knowledge base |
| **Cold** | `/archive` | S3 | 50-200ms | None | Historical |

## Search Examples

```bash
# Find authentication-related skills
ax search "JWT authentication" --path /skills

# Search code patterns
ax search "async database connection" --path /code

# Find relevant memories
ax search "user prefers functional style" --path /memories

# Search documentation
ax search "FastAPI dependency injection" --path /docs
```

## Integration with AI Assistants

The VFS can generate tool definitions for AI assistants:

```bash
# For Claude (MCP format)
ax tools --format mcp > tools.json

# For OpenAI/GPT
ax tools --format openai > functions.json
```

This enables AI assistants to:
- Read and write context files
- Search the knowledge base
- Store and retrieve code snippets
- Record conversation history
- Access documentation

# AX Use Cases

## 1. Agent Workspace Isolation

Mount isolated workspaces per task/agent:
- `/workspace` for mutable task files
- `/reference` for read-only context

Agents work in one namespace while storage stays separated.

## 2. Retrieval-Augmented Coding

Use indexing + search to retrieve relevant code/docs quickly:

```bash
ax index /workspace --incremental
ax search "token refresh behavior" --limit 8
ax grep "TODO|FIXME" /workspace --recursive
```

## 3. Tiered Storage

Route paths to different backends:
- hot local files in `fs`
- shared docs in `s3`
- structured artifacts in `postgres`
- vectors in `chroma`

Same interface for all paths.

## 4. Durable Write-Back Sync

Use write-back modes with WAL for reliability during intermittent connectivity.

## 5. Tool-Driven Automation via MCP

Run `ax mcp` and let tool-using agents perform file/search operations directly through MCP.

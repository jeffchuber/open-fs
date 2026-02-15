# AX + Claude Code

Use AX through either:
- FUSE mount (filesystem workflow)
- MCP server (`ax mcp`) for tool-based workflows

## FUSE path

1. Create `ax.yaml`.
2. Mount AX:

```bash
ax --config ax.yaml mount ~/ax-mount
```

3. Run Claude Code against the mount:

```bash
claude --working-dir ~/ax-mount
```

4. Unmount when done:

```bash
ax unmount ~/ax-mount
```

## MCP path

Run MCP server:

```bash
ax mcp
```

This exposes AX operations as MCP tools (`read`, `write`, `ls`, `stat`, `delete`, `grep`, `search`).

## Notes

- FUSE support targets macOS/Linux.
- Indexing + semantic search are provided by `ax-local`.

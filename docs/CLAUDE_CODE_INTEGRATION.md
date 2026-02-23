# OpenFS + Claude Code

Use AX through either:
- FUSE mount (filesystem workflow)
- MCP server (`openfs mcp`) for tool-based workflows

## FUSE path

1. Create `openfs.yaml`.
2. Mount AX:

```bash
openfs --config openfs.yaml mount ~/openfs-mount
```

3. Run Claude Code against the mount:

```bash
claude --working-dir ~/openfs-mount
```

4. Unmount when done:

```bash
openfs unmount ~/openfs-mount
```

## MCP path

Run MCP server:

```bash
openfs mcp
```

This exposes AX operations as MCP tools (`read`, `write`, `ls`, `stat`, `delete`, `grep`, `search`).

## Notes

- FUSE support targets macOS/Linux.
- Indexing + semantic search are provided by `openfs-local`.

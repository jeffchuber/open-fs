import { type ChildProcess, spawn } from "node:child_process";
import { type Interface, createInterface } from "node:readline";
import { eio, mcpErrorToVfsError } from "./errors.js";
import type { BatchReadResult, CacheStats, Entry, GrepMatch, SearchResult, Vfs } from "./types.js";

const MCP_PROTOCOL_VERSION = "2024-11-05";

interface JsonRpcRequest {
  id: number;
  method: string;
  params?: unknown;
  jsonrpc: "2.0";
}

interface JsonRpcResponse {
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
  jsonrpc: "2.0";
}

interface McpToolResult {
  content: Array<{ type: string; text: string }>;
  isError?: boolean | null;
}

export interface SubprocessVfsOptions {
  configPath?: string;
  openFsBinary?: string;
  /** @deprecated Use openFsBinary instead */
  axBinary?: string;
  cwd?: string;
}

export class SubprocessVfs implements Vfs {
  private proc: ChildProcess | null = null;
  private rl: Interface | null = null;
  private nextId = 1;
  private pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  private binaryPath: string;
  private configPath?: string;
  private cwd?: string;

  constructor(options: SubprocessVfsOptions = {}) {
    this.binaryPath = options.openFsBinary ?? options.axBinary ?? "openfs";
    this.configPath = options.configPath;
    this.cwd = options.cwd;
  }

  async connect(): Promise<void> {
    const args = ["mcp"];
    if (this.configPath) {
      args.push("--config", this.configPath);
    }

    this.proc = spawn(this.binaryPath, args, {
      stdio: ["pipe", "pipe", "pipe"],
      cwd: this.cwd,
    });

    this.proc.on("error", (err) => {
      this.rejectAll(new Error(`openfs process error: ${err.message}`));
    });

    this.proc.on("exit", (code) => {
      this.rejectAll(new Error(`openfs process exited with code ${code}`));
    });

    if (!this.proc.stdout) {
      throw eio("openfs process has no stdout");
    }
    this.rl = createInterface({ input: this.proc.stdout });
    this.rl.on("line", (line) => this.handleLine(line));

    const initResult = (await this.sendRequest("initialize", {
      protocolVersion: MCP_PROTOCOL_VERSION,
      capabilities: {},
      clientInfo: { name: "openfs", version: "0.1.0" },
    })) as { protocolVersion: string };

    if (!initResult?.protocolVersion) {
      throw eio("MCP initialization failed: no protocol version in response");
    }

    this.sendNotification("notifications/initialized");
  }

  // --- Vfs interface ---

  async read(path: string): Promise<string> {
    const raw = await this.callTool("openfs_read", { path });
    // openfs_read returns JSON: {"content":"...","cas_token":"..."}
    // Extract just the content string.
    try {
      const parsed = JSON.parse(raw);
      if (typeof parsed.content === "string") return parsed.content;
    } catch {
      // Not JSON envelope — return raw text
    }
    return raw;
  }

  async write(path: string, content: string): Promise<void> {
    await this.callTool("openfs_write", { path, content });
  }

  async append(path: string, content: string): Promise<void> {
    await this.callTool("openfs_append", { path, content });
  }

  async delete(path: string): Promise<void> {
    await this.callTool("openfs_delete", { path });
  }

  async list(path: string): Promise<Entry[]> {
    const text = await this.callTool("openfs_ls", { path });
    if (!text.trim()) return [];
    return JSON.parse(text) as Entry[];
  }

  async stat(path: string): Promise<Entry> {
    const text = await this.callTool("openfs_stat", { path });
    return JSON.parse(text) as Entry;
  }

  async exists(path: string): Promise<boolean> {
    const text = await this.callTool("openfs_exists", { path });
    const result = JSON.parse(text) as { exists: boolean };
    return result.exists;
  }

  async rename(from: string, to: string): Promise<void> {
    await this.callTool("openfs_rename", { from, to });
  }

  async grep(pattern: string, path?: string): Promise<GrepMatch[]> {
    const args: Record<string, unknown> = { pattern };
    if (path) args.path = path;
    const text = await this.callTool("openfs_grep", args);
    return JSON.parse(text) as GrepMatch[];
  }

  async search(query: string, limit?: number): Promise<SearchResult[]> {
    const args: Record<string, unknown> = { query };
    if (limit !== undefined) args.limit = limit;
    const text = await this.callTool("openfs_search", args);
    if (text === "No results found.") return [];
    return parseSearchOutput(text);
  }

  async readBatch(paths: string[]): Promise<Map<string, string>> {
    const text = await this.callTool("openfs_read_batch", { paths });
    const parsed = JSON.parse(text) as { results: BatchReadResult[] };
    const map = new Map<string, string>();
    for (const r of parsed.results) {
      if (r.content !== undefined) {
        map.set(r.path, r.content);
      }
    }
    return map;
  }

  async writeBatch(files: { path: string; content: string }[]): Promise<void> {
    await this.callTool("openfs_write_batch", { files });
  }

  async deleteBatch(paths: string[]): Promise<void> {
    await this.callTool("openfs_delete_batch", { paths });
  }

  async cacheStats(): Promise<CacheStats> {
    const text = await this.callTool("openfs_cache_stats", {});
    return JSON.parse(text) as CacheStats;
  }

  async prefetch(paths: string[]): Promise<{ prefetched: number; errors: number }> {
    const text = await this.callTool("openfs_prefetch", { paths });
    return JSON.parse(text) as { prefetched: number; errors: number };
  }

  async close(): Promise<void> {
    if (this.rl) {
      this.rl.close();
      this.rl = null;
    }
    if (this.proc) {
      this.proc.kill();
      this.proc = null;
    }
    this.rejectAll(new Error("client closed"));
  }

  // --- MCP transport ---

  private async callTool(
    name: string,
    args: Record<string, unknown>,
  ): Promise<string> {
    const result = (await this.sendRequest("tools/call", {
      name,
      arguments: args,
    })) as McpToolResult;

    if (result.isError) {
      const text = result.content?.[0]?.text ?? "unknown error";
      throw mcpErrorToVfsError(text, args.path as string | undefined);
    }

    return result.content?.[0]?.text ?? "";
  }

  private sendRequest(method: string, params?: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      if (!this.proc?.stdin?.writable) {
        reject(eio("MCP connection not open"));
        return;
      }

      const id = this.nextId++;
      const req: JsonRpcRequest = { jsonrpc: "2.0", id, method, params };
      this.pending.set(id, { resolve, reject });
      this.proc.stdin.write(`${JSON.stringify(req)}\n`);
    });
  }

  private sendNotification(method: string, params?: unknown): void {
    if (!this.proc?.stdin?.writable) return;
    const notification = { jsonrpc: "2.0" as const, method, params };
    this.proc.stdin.write(`${JSON.stringify(notification)}\n`);
  }

  private handleLine(line: string): void {
    const trimmed = line.trim();
    if (!trimmed) return;

    let msg: JsonRpcResponse;
    try {
      msg = JSON.parse(trimmed) as JsonRpcResponse;
    } catch {
      return;
    }

    if (msg.id === undefined || msg.id === null) return;

    const handler = this.pending.get(msg.id);
    if (!handler) return;
    this.pending.delete(msg.id);

    if (msg.error) {
      handler.reject(eio(`MCP error ${msg.error.code}: ${msg.error.message}`));
    } else {
      handler.resolve(msg.result);
    }
  }

  private rejectAll(err: Error): void {
    for (const handler of this.pending.values()) {
      handler.reject(err);
    }
    this.pending.clear();
  }
}

// --- Output parsers ---

function parseSearchOutput(text: string): SearchResult[] {
  const results: SearchResult[] = [];
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    const match = line.match(/^\[([^\]]+)\]\s+(\S+)\s+(.*)/);
    if (match) {
      results.push({
        score: Number.parseFloat(match[1]),
        source: match[2],
        snippet: match[3],
      });
    }
  }
  return results;
}

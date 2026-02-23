import { eisdir, enoent, enotsup } from "./errors.js";
import type { CacheStats, Entry, GrepMatch, SearchResult, Vfs } from "./types.js";

function normalizePath(p: string): string {
  const parts = p.split("/").filter(Boolean);
  const resolved: string[] = [];
  for (const part of parts) {
    if (part === "..") resolved.pop();
    else if (part !== ".") resolved.push(part);
  }
  return `/${resolved.join("/")}`;
}

export class MemoryVfs implements Vfs {
  private files = new Map<string, string>();

  async read(path: string): Promise<string> {
    const norm = normalizePath(path);
    const content = this.files.get(norm);
    if (content === undefined) {
      if (this.isDir(norm)) throw eisdir(norm);
      throw enoent(norm);
    }
    return content;
  }

  async write(path: string, content: string): Promise<void> {
    this.files.set(normalizePath(path), content);
  }

  async append(path: string, content: string): Promise<void> {
    const norm = normalizePath(path);
    const existing = this.files.get(norm) ?? "";
    this.files.set(norm, existing + content);
  }

  async delete(path: string): Promise<void> {
    const norm = normalizePath(path);
    this.files.delete(norm);
    const prefix = `${norm}/`;
    for (const key of [...this.files.keys()]) {
      if (key.startsWith(prefix)) this.files.delete(key);
    }
  }

  async list(path: string): Promise<Entry[]> {
    const norm = normalizePath(path);
    const prefix = norm === "/" ? "/" : `${norm}/`;
    const seen = new Set<string>();
    const entries: Entry[] = [];

    for (const key of this.files.keys()) {
      if (!key.startsWith(prefix)) continue;
      const rest = key.slice(prefix.length);
      const slashIdx = rest.indexOf("/");
      const childName = slashIdx === -1 ? rest : rest.slice(0, slashIdx);
      if (!childName || seen.has(childName)) continue;
      seen.add(childName);

      const childPath = `${prefix}${childName}`;
      const childIsDir = slashIdx !== -1 || this.isDir(childPath);

      entries.push({
        path: normalizePath(childPath),
        name: childName,
        is_dir: childIsDir,
        size: childIsDir ? null : this.files.get(key)!.length,
        modified: null,
      });
    }

    return entries.sort((a, b) => a.name.localeCompare(b.name));
  }

  async stat(path: string): Promise<Entry> {
    const norm = normalizePath(path);
    if (norm === "/") {
      return { path: "/", name: "/", is_dir: true, size: null, modified: null };
    }
    if (this.files.has(norm)) {
      return {
        path: norm,
        name: norm.split("/").pop()!,
        is_dir: false,
        size: this.files.get(norm)!.length,
        modified: null,
      };
    }
    if (this.isDir(norm)) {
      return {
        path: norm,
        name: norm.split("/").pop()!,
        is_dir: true,
        size: null,
        modified: null,
      };
    }
    throw enoent(norm);
  }

  async exists(path: string): Promise<boolean> {
    const norm = normalizePath(path);
    if (norm === "/") return true;
    return this.files.has(norm) || this.isDir(norm);
  }

  async rename(from: string, to: string): Promise<void> {
    const normFrom = normalizePath(from);
    const normTo = normalizePath(to);
    const content = this.files.get(normFrom);
    if (content === undefined) throw enoent(normFrom);
    this.files.set(normTo, content);
    this.files.delete(normFrom);
  }

  async grep(pattern: string, path?: string): Promise<GrepMatch[]> {
    const re = new RegExp(pattern);
    const matches: GrepMatch[] = [];
    const searchPrefix = path ? normalizePath(path) : "/";

    for (const [filePath, content] of this.files) {
      if (!filePath.startsWith(searchPrefix) && filePath !== searchPrefix)
        continue;
      const lines = content.split("\n");
      for (let i = 0; i < lines.length; i++) {
        if (re.test(lines[i])) {
          matches.push({ path: filePath, line_number: i + 1, line: lines[i] });
        }
      }
    }
    return matches;
  }

  async search(_query: string, _limit?: number): Promise<SearchResult[]> {
    return [];
  }

  async readBatch(paths: string[]): Promise<Map<string, string>> {
    const map = new Map<string, string>();
    for (const p of paths) {
      try {
        map.set(p, await this.read(p));
      } catch {
        // Skip failed reads
      }
    }
    return map;
  }

  async writeBatch(files: { path: string; content: string }[]): Promise<void> {
    for (const f of files) {
      await this.write(f.path, f.content);
    }
  }

  async deleteBatch(paths: string[]): Promise<void> {
    for (const p of paths) {
      await this.delete(p);
    }
  }

  async cacheStats(): Promise<CacheStats> {
    return { hits: 0, misses: 0, hit_rate: 0, entries: 0, size: 0, evictions: 0 };
  }

  async prefetch(_paths: string[]): Promise<{ prefetched: number; errors: number }> {
    return { prefetched: 0, errors: 0 };
  }

  async close(): Promise<void> {
    this.files.clear();
  }

  private isDir(path: string): boolean {
    const prefix = path === "/" ? "/" : `${path}/`;
    for (const key of this.files.keys()) {
      if (key.startsWith(prefix)) return true;
    }
    return false;
  }
}

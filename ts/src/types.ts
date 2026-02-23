export interface Entry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number | null;
  modified: string | null;
}

export interface GrepMatch {
  path: string;
  line_number: number;
  line: string;
}

export interface SearchResult {
  score: number;
  source: string;
  snippet: string;
}

export interface BackendConfig {
  type: "fs" | "memory" | "s3" | "postgres" | "chroma";
  [key: string]: unknown;
}

export interface MountConfig {
  path: string;
  backend: string;
  mode?: "read-write" | "read-only" | "append-only";
  [key: string]: unknown;
}

export interface VfsConfig {
  backends: Record<string, BackendConfig>;
  mounts: MountConfig[];
  defaults?: Record<string, unknown>;
}

export interface BatchReadResult {
  path: string;
  content?: string;
  error?: string;
}

export interface BatchWriteResult {
  path: string;
  status: "ok" | "error";
  error?: string;
}

export interface CacheStats {
  hits: number;
  misses: number;
  hit_rate: number;
  entries: number;
  size: number;
  evictions: number;
}

export interface Vfs {
  read(path: string): Promise<string>;
  write(path: string, content: string): Promise<void>;
  append(path: string, content: string): Promise<void>;
  delete(path: string): Promise<void>;
  list(path: string): Promise<Entry[]>;
  stat(path: string): Promise<Entry>;
  exists(path: string): Promise<boolean>;
  rename(from: string, to: string): Promise<void>;
  grep(pattern: string, path?: string): Promise<GrepMatch[]>;
  search(query: string, limit?: number): Promise<SearchResult[]>;
  readBatch(paths: string[]): Promise<Map<string, string>>;
  writeBatch(files: { path: string; content: string }[]): Promise<void>;
  deleteBatch(paths: string[]): Promise<void>;
  cacheStats(): Promise<CacheStats>;
  prefetch(paths: string[]): Promise<{ prefetched: number; errors: number }>;
  close(): Promise<void>;
}

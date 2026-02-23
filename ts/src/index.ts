export type {
  Entry,
  GrepMatch,
  SearchResult,
  BatchReadResult,
  BatchWriteResult,
  CacheStats,
  BackendConfig,
  MountConfig,
  VfsConfig,
  Vfs,
} from "./types.js";

export { enoent, eisdir, enotdir, eio, enotsup, eexist, mcpErrorToVfsError } from "./errors.js";
export type { VfsError } from "./errors.js";

export { loadConfig } from "./config.js";

export { MemoryVfs } from "./memory.js";

export { SubprocessVfs } from "./vfs.js";
export type { SubprocessVfsOptions } from "./vfs.js";

import { MemoryVfs } from "./memory.js";
import { SubprocessVfs, type SubprocessVfsOptions } from "./vfs.js";
import type { Vfs } from "./types.js";

/**
 * Create a Vfs backed by the real OpenFS Rust binary.
 * Spawns `openfs mcp` as a subprocess and returns a ready-to-use Vfs.
 */
export async function createVfs(options: SubprocessVfsOptions = {}): Promise<Vfs> {
  const vfs = new SubprocessVfs(options);
  await vfs.connect();
  return vfs;
}

/**
 * Create an in-memory Vfs for dev/testing. No subprocess needed.
 */
export function createMemoryVfs(): Vfs {
  return new MemoryVfs();
}

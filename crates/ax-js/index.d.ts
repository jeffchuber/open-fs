/**
 * AX Virtual Filesystem - TypeScript type definitions
 */

/** A file or directory entry */
export interface JsEntry {
  /** Full path to the entry */
  path: string;
  /** Filename or directory name */
  name: string;
  /** True if this is a directory */
  isDir: boolean;
  /** File size in bytes (null for directories) */
  size: number | null;
}

/** A single grep match */
export interface JsGrepMatch {
  /** Path to the file containing the match */
  path: string;
  /** Line number (1-based) */
  lineNumber: number;
  /** Matching line content */
  line: string;
}

/** AX Virtual Filesystem */
export class JsVfs {
  /**
   * Create a new VFS from a YAML configuration string.
   * @param yaml - YAML configuration string
   */
  static fromYaml(yaml: string): JsVfs;

  /**
   * Create a new VFS from a YAML configuration file.
   * @param path - Path to the configuration file
   */
  static fromFile(path: string): JsVfs;

  /**
   * Read the contents of a file as a Buffer.
   * @param path - Path to the file
   */
  read(path: string): Buffer;

  /**
   * Read the contents of a file as a string.
   * @param path - Path to the file
   */
  readText(path: string): string;

  /**
   * Write content to a file.
   * @param path - Path to the file
   * @param content - Content to write
   */
  write(path: string, content: Buffer): void;

  /**
   * Write a string to a file.
   * @param path - Path to the file
   * @param content - Content to write
   */
  writeText(path: string, content: string): void;

  /**
   * Append content to a file.
   * @param path - Path to the file
   * @param content - Content to append
   */
  append(path: string, content: Buffer): void;

  /**
   * Append a string to a file.
   * @param path - Path to the file
   * @param content - Content to append
   */
  appendText(path: string, content: string): void;

  /**
   * Delete a file.
   * @param path - Path to the file
   */
  delete(path: string): void;

  /**
   * List files in a directory.
   * @param path - Path to the directory
   */
  list(path: string): JsEntry[];

  /**
   * Check if a path exists.
   * @param path - Path to check
   */
  exists(path: string): boolean;

  /**
   * Get metadata for a path.
   * @param path - Path to the file or directory
   */
  stat(path: string): JsEntry;

  /**
   * Generate tool definitions in JSON format.
   * @param format - Output format: 'json', 'mcp', or 'openai'
   */
  tools(format?: string): string;

  /**
   * Get the VFS name from configuration.
   */
  name(): string | null;

  /**
   * Get mount paths.
   */
  mounts(): string[];

  /**
   * Rename/move a file.
   * @param from - Source path
   * @param to - Destination path
   */
  rename(from: string, to: string): void;

  /**
   * Copy a file. Returns the number of bytes copied.
   * @param src - Source path
   * @param dst - Destination path
   */
  copy(src: string, dst: string): number;

  /**
   * Search files for lines matching a regex pattern.
   * @param pattern - Regular expression pattern
   * @param path - Path to search (default: "/")
   * @param recursive - Whether to search recursively (default: false)
   */
  grep(pattern: string, path?: string, recursive?: boolean): JsGrepMatch[];
}

/**
 * Parse a YAML configuration string and return a VFS.
 * @param yaml - YAML configuration string
 */
export function loadConfig(yaml: string): JsVfs;

/**
 * Load a VFS from a configuration file.
 * @param path - Path to the configuration file
 */
export function loadConfigFile(path: string): JsVfs;

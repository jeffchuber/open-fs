export interface VfsError extends Error {
  code: string;
  path?: string;
}

function vfsError(code: string, message: string, path?: string): VfsError {
  const err = new Error(message) as VfsError;
  err.code = code;
  if (path) err.path = path;
  return err;
}

export function enoent(path: string): VfsError {
  return vfsError("ENOENT", `no such file or directory: ${path}`, path);
}

export function eisdir(path: string): VfsError {
  return vfsError("EISDIR", `illegal operation on a directory: ${path}`, path);
}

export function enotdir(path: string): VfsError {
  return vfsError("ENOTDIR", `not a directory: ${path}`, path);
}

export function eio(message: string, path?: string): VfsError {
  return vfsError("EIO", message, path);
}

export function enotsup(op: string): VfsError {
  return vfsError("ENOTSUP", `operation not supported: ${op}`);
}

export function eexist(path: string): VfsError {
  return vfsError("EEXIST", `file already exists: ${path}`, path);
}

export function mcpErrorToVfsError(message: string, path?: string): VfsError {
  const lower = message.toLowerCase();
  if (lower.includes("not found") || lower.includes("no such")) {
    return enoent(path ?? "unknown");
  }
  if (lower.includes("is a directory")) {
    return eisdir(path ?? "unknown");
  }
  if (lower.includes("not a directory")) {
    return enotdir(path ?? "unknown");
  }
  if (lower.includes("not supported")) {
    return enotsup(message);
  }
  return eio(message, path);
}

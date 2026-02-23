import { describe, it, expect } from "vitest";
import { SubprocessVfs } from "../src/vfs.js";
import { createVfs, createMemoryVfs, type Vfs, type Entry } from "../src/index.js";

describe("SubprocessVfs", () => {
  it("constructor sets defaults", () => {
    const vfs = new SubprocessVfs();
    // Should not throw — subprocess not started until connect()
    expect(vfs).toBeDefined();
  });

  it("constructor accepts options", () => {
    const vfs = new SubprocessVfs({
      axBinary: "/usr/local/bin/ax",
      configPath: "/tmp/ax.yaml",
      cwd: "/tmp",
    });
    expect(vfs).toBeDefined();
  });

  it("close on unconnected vfs is safe", async () => {
    const vfs = new SubprocessVfs();
    await vfs.close(); // should not throw
  });
});

describe("createMemoryVfs", () => {
  it("returns a working Vfs", async () => {
    const vfs = createMemoryVfs();
    await vfs.write("/test.txt", "hello");
    expect(await vfs.read("/test.txt")).toBe("hello");
    await vfs.close();
  });
});

describe("Vfs interface compliance", () => {
  it("MemoryVfs implements all Vfs methods", () => {
    const vfs = createMemoryVfs();
    const methods: (keyof Vfs)[] = [
      "read", "write", "append", "delete", "list",
      "stat", "exists", "rename", "grep", "search",
      "readBatch", "writeBatch", "deleteBatch",
      "cacheStats", "prefetch", "close",
    ];
    for (const method of methods) {
      expect(typeof vfs[method]).toBe("function");
    }
  });
});

import { describe, it, expect, beforeEach } from "vitest";
import { createMemoryVfs, type Vfs } from "../src/index.js";

describe("MemoryVfs", () => {
  let vfs: Vfs;

  beforeEach(() => {
    vfs = createMemoryVfs();
  });

  it("read/write round-trip", async () => {
    await vfs.write("/hello.txt", "world");
    expect(await vfs.read("/hello.txt")).toBe("world");
  });

  it("read throws ENOENT for missing file", async () => {
    await expect(vfs.read("/nope")).rejects.toThrow("no such file");
  });

  it("overwrite replaces content", async () => {
    await vfs.write("/f.txt", "a");
    await vfs.write("/f.txt", "b");
    expect(await vfs.read("/f.txt")).toBe("b");
  });

  it("append to new file", async () => {
    await vfs.append("/new.txt", "hello");
    expect(await vfs.read("/new.txt")).toBe("hello");
  });

  it("append to existing file", async () => {
    await vfs.write("/f.txt", "hello");
    await vfs.append("/f.txt", " world");
    expect(await vfs.read("/f.txt")).toBe("hello world");
  });

  it("delete removes file", async () => {
    await vfs.write("/f.txt", "data");
    await vfs.delete("/f.txt");
    expect(await vfs.exists("/f.txt")).toBe(false);
  });

  it("delete removes directory children", async () => {
    await vfs.write("/dir/a.txt", "a");
    await vfs.write("/dir/b.txt", "b");
    await vfs.delete("/dir");
    expect(await vfs.exists("/dir/a.txt")).toBe(false);
    expect(await vfs.exists("/dir/b.txt")).toBe(false);
  });

  it("list returns directory entries", async () => {
    await vfs.write("/docs/readme.md", "# Hello");
    await vfs.write("/docs/guide.md", "Guide");
    const entries = await vfs.list("/docs");
    expect(entries).toHaveLength(2);
    expect(entries.map((e) => e.name).sort()).toEqual(["guide.md", "readme.md"]);
    expect(entries.every((e) => !e.is_dir)).toBe(true);
  });

  it("list shows subdirectories", async () => {
    await vfs.write("/a/b/c.txt", "deep");
    const entries = await vfs.list("/a");
    expect(entries).toHaveLength(1);
    expect(entries[0].name).toBe("b");
    expect(entries[0].is_dir).toBe(true);
  });

  it("list root", async () => {
    await vfs.write("/x/file.txt", "x");
    await vfs.write("/y/file.txt", "y");
    const entries = await vfs.list("/");
    expect(entries.map((e) => e.name).sort()).toEqual(["x", "y"]);
  });

  it("stat file", async () => {
    await vfs.write("/f.txt", "hello");
    const entry = await vfs.stat("/f.txt");
    expect(entry.is_dir).toBe(false);
    expect(entry.size).toBe(5);
    expect(entry.name).toBe("f.txt");
  });

  it("stat directory", async () => {
    await vfs.write("/dir/f.txt", "x");
    const entry = await vfs.stat("/dir");
    expect(entry.is_dir).toBe(true);
  });

  it("stat root", async () => {
    const entry = await vfs.stat("/");
    expect(entry.is_dir).toBe(true);
  });

  it("stat throws ENOENT", async () => {
    await expect(vfs.stat("/nope")).rejects.toThrow("no such file");
  });

  it("exists", async () => {
    expect(await vfs.exists("/")).toBe(true);
    expect(await vfs.exists("/nope")).toBe(false);
    await vfs.write("/f.txt", "x");
    expect(await vfs.exists("/f.txt")).toBe(true);
  });

  it("rename", async () => {
    await vfs.write("/old.txt", "data");
    await vfs.rename("/old.txt", "/new.txt");
    expect(await vfs.exists("/old.txt")).toBe(false);
    expect(await vfs.read("/new.txt")).toBe("data");
  });

  it("rename throws ENOENT for missing source", async () => {
    await expect(vfs.rename("/nope", "/dest")).rejects.toThrow("no such file");
  });

  it("grep finds matches", async () => {
    await vfs.write("/code.ts", "const x = 1;\nconst y = 2;\nlet z = 3;");
    const matches = await vfs.grep("const");
    expect(matches).toHaveLength(2);
    expect(matches[0].line_number).toBe(1);
    expect(matches[1].line_number).toBe(2);
  });

  it("grep with path filter", async () => {
    await vfs.write("/a/file.txt", "hello world");
    await vfs.write("/b/file.txt", "hello there");
    const matches = await vfs.grep("hello", "/a");
    expect(matches).toHaveLength(1);
    expect(matches[0].path).toBe("/a/file.txt");
  });

  it("search returns empty for MemoryVfs", async () => {
    await vfs.write("/doc.txt", "some content");
    const results = await vfs.search("content");
    expect(results).toEqual([]);
  });

  it("close clears state", async () => {
    await vfs.write("/f.txt", "data");
    await vfs.close();
    expect(await vfs.exists("/f.txt")).toBe(false);
  });

  it("normalizes paths with .. and .", async () => {
    await vfs.write("/a/b/../c.txt", "data");
    expect(await vfs.read("/a/c.txt")).toBe("data");
  });
});

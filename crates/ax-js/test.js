/**
 * Comprehensive tests for AX Node.js bindings.
 *
 * These tests cover the full API surface of the AX virtual filesystem
 * Node.js bindings, including edge cases and error handling.
 */

const assert = require('assert');
const fs = require('fs');
const path = require('path');
const os = require('os');

// Try to load the native module
let ax;
try {
  ax = require('./index.js');
} catch (e) {
  console.log('Native module not built. Run `npm run build` first.');
  console.log('Skipping tests.');
  process.exit(0);
}

const { JsVfs, loadConfig, loadConfigFile } = ax;

// Test utilities
function createTempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'ax-test-'));
}

function removeTempDir(dir) {
  fs.rmSync(dir, { recursive: true, force: true });
}

function createTestConfig(dataDir) {
  return `
name: test-vfs
backends:
  local:
    type: fs
    root: ${dataDir}
mounts:
  - path: /workspace
    backend: local
`;
}

// Test runner
let passed = 0;
let failed = 0;
const failures = [];

function test(name, fn) {
  try {
    fn();
    passed++;
    console.log(`  \x1b[32m\u2713\x1b[0m ${name}`);
  } catch (e) {
    failed++;
    failures.push({ name, error: e });
    console.log(`  \x1b[31m\u2717\x1b[0m ${name}`);
    console.log(`    ${e.message}`);
  }
}

function describe(name, fn) {
  console.log(`\n${name}`);
  fn();
}

// ============================================================================
// Tests
// ============================================================================

describe('VFS Creation', () => {
  test('fromYaml with minimal config', () => {
    const tmpDir = createTempDir();
    try {
      const config = createTestConfig(tmpDir);
      const vfs = JsVfs.fromYaml(config);
      assert(vfs !== null);
      assert.strictEqual(vfs.name(), 'test-vfs');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('fromYaml with multiple mounts', () => {
    const tmpDir = createTempDir();
    const dir1 = path.join(tmpDir, 'dir1');
    const dir2 = path.join(tmpDir, 'dir2');
    fs.mkdirSync(dir1);
    fs.mkdirSync(dir2);

    try {
      const config = `
name: multi-mount
backends:
  b1:
    type: fs
    root: ${dir1}
  b2:
    type: fs
    root: ${dir2}
mounts:
  - path: /mount1
    backend: b1
  - path: /mount2
    backend: b2
`;
      const vfs = JsVfs.fromYaml(config);
      const mounts = vfs.mounts();
      assert(mounts.includes('/mount1'));
      assert(mounts.includes('/mount2'));
      assert.strictEqual(mounts.length, 2);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('fromYaml with invalid config throws', () => {
    assert.throws(() => {
      JsVfs.fromYaml('not: valid: yaml:');
    });
  });

  test('fromFile loads configuration', () => {
    const tmpDir = createTempDir();
    const dataDir = path.join(tmpDir, 'data');
    fs.mkdirSync(dataDir);
    const configPath = path.join(tmpDir, 'ax.yaml');

    try {
      const config = `
name: file-config
backends:
  local:
    type: fs
    root: ${dataDir}
mounts:
  - path: /data
    backend: local
`;
      fs.writeFileSync(configPath, config);

      const vfs = JsVfs.fromFile(configPath);
      assert.strictEqual(vfs.name(), 'file-config');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('fromFile with nonexistent file throws', () => {
    assert.throws(() => {
      JsVfs.fromFile('/nonexistent/path/config.yaml');
    });
  });

  test('loadConfig function works', () => {
    const tmpDir = createTempDir();
    try {
      const config = createTestConfig(tmpDir);
      const vfs = loadConfig(config);
      assert(vfs !== null);
    } finally {
      removeTempDir(tmpDir);
    }
  });
});

describe('File Operations', () => {
  test('writeText and readText', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/hello.txt', 'Hello, World!');
      const content = vfs.readText('/workspace/hello.txt');
      assert.strictEqual(content, 'Hello, World!');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('write and read binary', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const data = Buffer.from([0x00, 0x01, 0x02, 0xff, 0xfe]);
      vfs.write('/workspace/binary.bin', data);
      const content = vfs.read('/workspace/binary.bin');
      assert(Buffer.isBuffer(content));
      assert(data.equals(content));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('write empty file', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/empty.txt', '');
      const content = vfs.readText('/workspace/empty.txt');
      assert.strictEqual(content, '');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('write unicode content', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const text = 'Hello \u4e16\u754c \u{1F600} \u00e9\u00e8\u00ea';
      vfs.writeText('/workspace/unicode.txt', text);
      const content = vfs.readText('/workspace/unicode.txt');
      assert.strictEqual(content, text);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('write large file', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const data = Buffer.alloc(1024 * 1024, 'x'); // 1MB
      vfs.write('/workspace/large.bin', data);
      const content = vfs.read('/workspace/large.bin');
      assert.strictEqual(content.length, 1024 * 1024);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('overwrite file', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/overwrite.txt', 'original');
      vfs.writeText('/workspace/overwrite.txt', 'modified');
      const content = vfs.readText('/workspace/overwrite.txt');
      assert.strictEqual(content, 'modified');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('read nonexistent file throws', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      assert.throws(() => {
        vfs.read('/workspace/nonexistent.txt');
      });
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('appendText', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/append.txt', 'Hello');
      vfs.appendText('/workspace/append.txt', ' World');
      const content = vfs.readText('/workspace/append.txt');
      assert.strictEqual(content, 'Hello World');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('append to new file creates it', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.appendText('/workspace/new_append.txt', 'content');
      const content = vfs.readText('/workspace/new_append.txt');
      assert.strictEqual(content, 'content');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('append binary', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.write('/workspace/append.bin', Buffer.from([0x01, 0x02]));
      vfs.append('/workspace/append.bin', Buffer.from([0x03, 0x04]));
      const content = vfs.read('/workspace/append.bin');
      assert(Buffer.from([0x01, 0x02, 0x03, 0x04]).equals(content));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('delete file', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/to_delete.txt', 'delete me');
      assert(vfs.exists('/workspace/to_delete.txt'));
      vfs.delete('/workspace/to_delete.txt');
      assert(!vfs.exists('/workspace/to_delete.txt'));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('delete nonexistent file throws', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      assert.throws(() => {
        vfs.delete('/workspace/nonexistent.txt');
      });
    } finally {
      removeTempDir(tmpDir);
    }
  });
});

describe('Directory Operations', () => {
  test('list empty directory', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const entries = vfs.list('/workspace');
      assert(Array.isArray(entries));
      assert.strictEqual(entries.length, 0);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('list directory with files', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/file1.txt', 'content1');
      vfs.writeText('/workspace/file2.txt', 'content2');

      const entries = vfs.list('/workspace');
      const names = entries.map(e => e.name);
      assert(names.includes('file1.txt'));
      assert(names.includes('file2.txt'));
      assert.strictEqual(entries.length, 2);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('list shows subdirectories', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/subdir/file.txt', 'content');

      const entries = vfs.list('/workspace');
      const subdirEntry = entries.find(e => e.name === 'subdir');
      assert(subdirEntry !== undefined);
      assert(subdirEntry.isDir);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('list subdirectory', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/subdir/file1.txt', 'content1');
      vfs.writeText('/workspace/subdir/file2.txt', 'content2');

      const entries = vfs.list('/workspace/subdir');
      const names = entries.map(e => e.name);
      assert(names.includes('file1.txt'));
      assert(names.includes('file2.txt'));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('exists returns true for file', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/exists.txt', 'content');
      assert(vfs.exists('/workspace/exists.txt'));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('exists returns true for directory', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/subdir/file.txt', 'content');
      assert(vfs.exists('/workspace/subdir'));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('exists returns false for nonexistent', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      assert(!vfs.exists('/workspace/nonexistent'));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('stat file', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/stat.txt', 'hello');
      const entry = vfs.stat('/workspace/stat.txt');
      assert.strictEqual(entry.name, 'stat.txt');
      assert(!entry.isDir);
      assert.strictEqual(entry.size, 5);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('stat directory', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/subdir/file.txt', 'content');
      const entry = vfs.stat('/workspace/subdir');
      assert.strictEqual(entry.name, 'subdir');
      assert(entry.isDir);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('stat nonexistent throws', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      assert.throws(() => {
        vfs.stat('/workspace/nonexistent');
      });
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('write creates parent directories', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/a/b/c/deep.txt', 'deep content');
      const content = vfs.readText('/workspace/a/b/c/deep.txt');
      assert.strictEqual(content, 'deep content');
      assert(vfs.exists('/workspace/a/b/c'));
      assert(vfs.exists('/workspace/a/b'));
      assert(vfs.exists('/workspace/a'));
    } finally {
      removeTempDir(tmpDir);
    }
  });
});

describe('Path Handling', () => {
  test('path with trailing slash', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/file.txt', 'content');
      const entries = vfs.list('/workspace/');
      const names = entries.map(e => e.name);
      assert(names.includes('file.txt'));
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('special characters in filename', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/file-with_special.chars.txt', 'content');
      const content = vfs.readText('/workspace/file-with_special.chars.txt');
      assert.strictEqual(content, 'content');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('spaces in filename', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/file with spaces.txt', 'content');
      const content = vfs.readText('/workspace/file with spaces.txt');
      assert.strictEqual(content, 'content');
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('no mount error', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      assert.throws(() => {
        vfs.read('/nonexistent_mount/file.txt');
      });
    } finally {
      removeTempDir(tmpDir);
    }
  });
});

describe('Tools Generation', () => {
  test('tools with json format', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const toolsJson = vfs.tools('json');
      const tools = JSON.parse(toolsJson);
      assert(tools.tools !== undefined);
      assert(tools.tools.length > 0);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('tools with mcp format', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const toolsJson = vfs.tools('mcp');
      const tools = JSON.parse(toolsJson);
      assert(tools.tools !== undefined);
      // MCP format should have input_schema
      for (const tool of tools.tools) {
        assert(tool.name !== undefined);
        assert(tool.input_schema !== undefined);
      }
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('tools with openai format', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const toolsJson = vfs.tools('openai');
      const tools = JSON.parse(toolsJson);
      assert(tools.tools !== undefined);
      // OpenAI format should have type: function
      for (const tool of tools.tools) {
        assert.strictEqual(tool.type, 'function');
        assert(tool.function !== undefined);
      }
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('tools with default format', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      const toolsJson = vfs.tools();
      const tools = JSON.parse(toolsJson);
      assert(tools.tools !== undefined);
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('tools with invalid format throws', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      assert.throws(() => {
        vfs.tools('invalid_format');
      });
    } finally {
      removeTempDir(tmpDir);
    }
  });
});

describe('Entry Object', () => {
  test('entry has required properties', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/test.txt', 'content');
      const entry = vfs.stat('/workspace/test.txt');

      assert('path' in entry);
      assert('name' in entry);
      assert('isDir' in entry);
      assert('size' in entry);

      assert.strictEqual(entry.name, 'test.txt');
      assert.strictEqual(entry.isDir, false);
      assert.strictEqual(entry.size, 7);
    } finally {
      removeTempDir(tmpDir);
    }
  });
});

describe('Multiple Operations', () => {
  test('many sequential operations', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));

      for (let i = 0; i < 100; i++) {
        vfs.writeText(`/workspace/file_${i}.txt`, `content_${i}`);
      }

      for (let i = 0; i < 100; i++) {
        const content = vfs.readText(`/workspace/file_${i}.txt`);
        assert.strictEqual(content, `content_${i}`);
      }
    } finally {
      removeTempDir(tmpDir);
    }
  });

  test('interleaved read write', () => {
    const tmpDir = createTempDir();
    try {
      const vfs = JsVfs.fromYaml(createTestConfig(tmpDir));
      vfs.writeText('/workspace/interleaved.txt', 'initial');

      for (let i = 0; i < 50; i++) {
        vfs.readText('/workspace/interleaved.txt');
        vfs.writeText('/workspace/interleaved.txt', `iteration_${i}`);
      }

      const final = vfs.readText('/workspace/interleaved.txt');
      assert.strictEqual(final, 'iteration_49');
    } finally {
      removeTempDir(tmpDir);
    }
  });
});

// ============================================================================
// Run tests and report
// ============================================================================

console.log('\n' + '='.repeat(60));
console.log(`Tests complete: ${passed} passed, ${failed} failed`);

if (failed > 0) {
  console.log('\nFailures:');
  for (const { name, error } of failures) {
    console.log(`\n  ${name}:`);
    console.log(`    ${error.stack}`);
  }
  process.exit(1);
}

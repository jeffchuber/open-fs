/**
 * Example usage of AX TypeScript bindings.
 *
 * Build the bindings first:
 *     cd crates/ax-js
 *     npm install
 *     npm run build
 *
 * Then run this script:
 *     npx ts-node examples/typescript_example.ts
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// Import the ax module (built with napi-rs)
// Note: Adjust the import path based on your setup
let ax: any;
try {
  ax = require('../crates/ax-js');
} catch (e) {
  console.error('Error: ax-vfs module not found.');
  console.error('Build it first with: cd crates/ax-js && npm install && npm run build');
  process.exit(1);
}

async function main() {
  // Create a temporary directory for our workspace
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ax-example-'));
  const dataDir = path.join(tmpDir, 'data');
  fs.mkdirSync(dataDir, { recursive: true });

  try {
    // Create a VFS configuration
    const config = `
name: typescript-example
backends:
  local:
    type: fs
    root: ${dataDir}
mounts:
  - path: /workspace
    backend: local
`;

    // Initialize the VFS
    console.log('Initializing VFS...');
    const vfs = ax.loadConfig(config);
    console.log(`VFS Name: ${vfs.name()}`);
    console.log(`Mounts: ${JSON.stringify(vfs.mounts())}`);
    console.log();

    // Write some files
    console.log('Writing files...');
    vfs.writeText('/workspace/hello.txt', 'Hello from TypeScript!');
    vfs.writeText('/workspace/data.json', JSON.stringify({ key: 'value', count: 42 }));
    vfs.writeText('/workspace/notes/todo.txt', '- Learn AX\n- Build something cool');
    console.log('Files written successfully');
    console.log();

    // Read files
    console.log('Reading files...');
    const content = vfs.readText('/workspace/hello.txt');
    console.log(`  hello.txt: ${content}`);

    const data = JSON.parse(vfs.readText('/workspace/data.json'));
    console.log(`  data.json: ${JSON.stringify(data)}`);
    console.log();

    // List directory
    console.log('Listing /workspace:');
    const entries = vfs.list('/workspace');
    for (const entry of entries) {
      const entryType = entry.isDir ? 'DIR ' : 'FILE';
      const size = entry.size ? `(${entry.size} bytes)` : '';
      console.log(`  [${entryType}] ${entry.name} ${size}`);
    }
    console.log();

    // Check existence
    console.log('Checking existence...');
    console.log(`  /workspace/hello.txt exists: ${vfs.exists('/workspace/hello.txt')}`);
    console.log(`  /workspace/missing.txt exists: ${vfs.exists('/workspace/missing.txt')}`);
    console.log();

    // Get file stats
    console.log('File stats for /workspace/hello.txt:');
    const stat = vfs.stat('/workspace/hello.txt');
    console.log(`  Path: ${stat.path}`);
    console.log(`  Name: ${stat.name}`);
    console.log(`  Size: ${stat.size} bytes`);
    console.log(`  Is Dir: ${stat.isDir}`);
    console.log();

    // Generate AI tools
    console.log('Generating AI tool definitions...');
    const toolsJson = vfs.tools('openai');
    const tools = JSON.parse(toolsJson);
    console.log(`  Generated ${tools.length} tools:`);
    for (const tool of tools.slice(0, 3)) {
      const name = tool.name || tool.function?.name || 'unknown';
      console.log(`    - ${name}`);
    }
    console.log('    ...');
    console.log();

    // Append to a file
    console.log('Appending to file...');
    vfs.appendText('/workspace/notes/todo.txt', '\n- Deploy to production');
    const updatedContent = vfs.readText('/workspace/notes/todo.txt');
    console.log(`  Updated todo.txt:\n${updatedContent}`);
    console.log();

    // Delete a file
    console.log('Deleting /workspace/hello.txt...');
    vfs.delete('/workspace/hello.txt');
    console.log(`  File exists after delete: ${vfs.exists('/workspace/hello.txt')}`);
    console.log();

    console.log('Example complete!');

  } finally {
    // Cleanup
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

main().catch(console.error);

#!/usr/bin/env python3
"""
Example usage of AX Python bindings.

Build the bindings first:
    cd crates/ax-ffi
    maturin develop

Then run this script:
    python examples/python_example.py
"""

import json
import os
import tempfile

# Import the ax module (built with maturin)
try:
    import ax
except ImportError:
    print("Error: ax module not found.")
    print("Build it first with: cd crates/ax-ffi && maturin develop")
    exit(1)


def main():
    # Create a temporary directory for our workspace
    with tempfile.TemporaryDirectory() as tmpdir:
        data_dir = os.path.join(tmpdir, "data")
        os.makedirs(data_dir)

        # Create a VFS configuration
        config = f"""
name: python-example
backends:
  local:
    type: fs
    root: {data_dir}
mounts:
  - path: /workspace
    backend: local
"""

        # Initialize the VFS
        print("Initializing VFS...")
        vfs = ax.load_config(config)
        print(f"VFS Name: {vfs.name()}")
        print(f"Mounts: {vfs.mounts()}")
        print()

        # Write some files
        print("Writing files...")
        vfs.write_text("/workspace/hello.txt", "Hello from Python!")
        vfs.write_text("/workspace/data.json", json.dumps({"key": "value", "count": 42}))
        vfs.write_text("/workspace/notes/todo.txt", "- Learn AX\n- Build something cool")
        print("Files written successfully")
        print()

        # Read files
        print("Reading files...")
        content = vfs.read_text("/workspace/hello.txt")
        print(f"  hello.txt: {content}")

        data = json.loads(vfs.read_text("/workspace/data.json"))
        print(f"  data.json: {data}")
        print()

        # List directory
        print("Listing /workspace:")
        for entry in vfs.list("/workspace"):
            entry_type = "DIR " if entry.is_dir else "FILE"
            size = f"({entry.size} bytes)" if entry.size else ""
            print(f"  [{entry_type}] {entry.name} {size}")
        print()

        # Check existence
        print("Checking existence...")
        print(f"  /workspace/hello.txt exists: {vfs.exists('/workspace/hello.txt')}")
        print(f"  /workspace/missing.txt exists: {vfs.exists('/workspace/missing.txt')}")
        print()

        # Get file stats
        print("File stats for /workspace/hello.txt:")
        stat = vfs.stat("/workspace/hello.txt")
        print(f"  Path: {stat.path}")
        print(f"  Name: {stat.name}")
        print(f"  Size: {stat.size} bytes")
        print(f"  Is Dir: {stat.is_dir}")
        print()

        # Generate AI tools
        print("Generating AI tool definitions...")
        tools_json = vfs.tools(format="openai")
        tools = json.loads(tools_json)
        print(f"  Generated {len(tools)} tools:")
        for tool in tools[:3]:  # Show first 3
            print(f"    - {tool.get('name', tool.get('function', {}).get('name', 'unknown'))}")
        print("    ...")
        print()

        # Append to a file
        print("Appending to file...")
        vfs.append_text("/workspace/notes/todo.txt", "\n- Deploy to production")
        content = vfs.read_text("/workspace/notes/todo.txt")
        print(f"  Updated todo.txt:\n{content}")
        print()

        # Delete a file
        print("Deleting /workspace/hello.txt...")
        vfs.delete("/workspace/hello.txt")
        print(f"  File exists after delete: {vfs.exists('/workspace/hello.txt')}")
        print()

        print("Example complete!")


if __name__ == "__main__":
    main()

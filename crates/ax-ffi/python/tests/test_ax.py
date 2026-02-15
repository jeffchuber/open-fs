"""Comprehensive tests for AX Python bindings.

These tests cover the full API surface of the AX virtual filesystem
Python bindings, including edge cases and error handling.
"""

import os
import tempfile
import pytest
from pathlib import Path


# Skip all tests if ax module is not built
pytest.importorskip("ax")

import ax


class TestVfsCreation:
    """Tests for VFS creation and configuration."""

    def test_from_yaml_minimal_config(self):
        """Test creating VFS with minimal configuration."""
        with tempfile.TemporaryDirectory() as tmpdir:
            config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {tmpdir}
mounts:
  - path: /data
    backend: local
"""
            vfs = ax.load_config(config)
            assert vfs is not None
            assert vfs.name() == "test-vfs"

    def test_from_yaml_multiple_mounts(self):
        """Test creating VFS with multiple mount points."""
        with tempfile.TemporaryDirectory() as tmpdir:
            dir1 = os.path.join(tmpdir, "dir1")
            dir2 = os.path.join(tmpdir, "dir2")
            os.makedirs(dir1)
            os.makedirs(dir2)

            config = f"""
name: multi-mount-vfs
backends:
  backend1:
    type: fs
    root: {dir1}
  backend2:
    type: fs
    root: {dir2}
mounts:
  - path: /mount1
    backend: backend1
  - path: /mount2
    backend: backend2
"""
            vfs = ax.load_config(config)
            mounts = vfs.mounts()
            assert "/mount1" in mounts
            assert "/mount2" in mounts
            assert len(mounts) == 2

    def test_from_yaml_invalid_config(self):
        """Test that invalid YAML raises an error."""
        with pytest.raises(ValueError):
            ax.load_config("not: valid: yaml: config:")

    def test_from_yaml_missing_backends(self):
        """Test that missing backends raise an error."""
        config = """
name: invalid
mounts:
  - path: /data
    backend: nonexistent
"""
        with pytest.raises((ValueError, IOError)):
            ax.load_config(config)

    def test_from_file(self):
        """Test loading configuration from file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            config_path = os.path.join(tmpdir, "ax.yaml")
            data_dir = os.path.join(tmpdir, "data")
            os.makedirs(data_dir)

            config = f"""
name: file-config-vfs
backends:
  local:
    type: fs
    root: {data_dir}
mounts:
  - path: /workspace
    backend: local
"""
            with open(config_path, "w") as f:
                f.write(config)

            vfs = ax.load_config_file(config_path)
            assert vfs.name() == "file-config-vfs"

    def test_from_file_not_found(self):
        """Test that missing config file raises an error."""
        with pytest.raises((ValueError, IOError, FileNotFoundError)):
            ax.load_config_file("/nonexistent/path/to/config.yaml")


class TestVfsOperations:
    """Tests for VFS file operations."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        # Cleanup
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_write_and_read_text(self, vfs):
        """Test writing and reading text content."""
        vfs.write_text("/workspace/hello.txt", "Hello, World!")
        content = vfs.read_text("/workspace/hello.txt")
        assert content == "Hello, World!"

    def test_write_and_read_binary(self, vfs):
        """Test writing and reading binary content."""
        data = b"\x00\x01\x02\x03\xff\xfe\xfd"
        vfs.write("/workspace/binary.bin", data)
        content = vfs.read("/workspace/binary.bin")
        assert content == data

    def test_write_empty_file(self, vfs):
        """Test writing an empty file."""
        vfs.write_text("/workspace/empty.txt", "")
        content = vfs.read_text("/workspace/empty.txt")
        assert content == ""

    def test_write_unicode_content(self, vfs):
        """Test writing Unicode content."""
        text = "Hello \u4e16\u754c \U0001F600 \u00e9\u00e8\u00ea"
        vfs.write_text("/workspace/unicode.txt", text)
        content = vfs.read_text("/workspace/unicode.txt")
        assert content == text

    def test_write_large_file(self, vfs):
        """Test writing a large file."""
        # 1MB of data
        data = b"x" * (1024 * 1024)
        vfs.write("/workspace/large.bin", data)
        content = vfs.read("/workspace/large.bin")
        assert len(content) == 1024 * 1024
        assert content == data

    def test_overwrite_file(self, vfs):
        """Test overwriting an existing file."""
        vfs.write_text("/workspace/overwrite.txt", "original")
        vfs.write_text("/workspace/overwrite.txt", "modified")
        content = vfs.read_text("/workspace/overwrite.txt")
        assert content == "modified"

    def test_read_nonexistent_file(self, vfs):
        """Test reading a file that doesn't exist."""
        with pytest.raises(IOError):
            vfs.read("/workspace/nonexistent.txt")

    def test_append_text(self, vfs):
        """Test appending text to a file."""
        vfs.write_text("/workspace/append.txt", "Hello")
        vfs.append_text("/workspace/append.txt", " World")
        content = vfs.read_text("/workspace/append.txt")
        assert content == "Hello World"

    def test_append_to_new_file(self, vfs):
        """Test appending to a non-existent file creates it."""
        vfs.append_text("/workspace/new_append.txt", "content")
        content = vfs.read_text("/workspace/new_append.txt")
        assert content == "content"

    def test_append_binary(self, vfs):
        """Test appending binary data."""
        vfs.write("/workspace/append.bin", b"\x01\x02")
        vfs.append("/workspace/append.bin", b"\x03\x04")
        content = vfs.read("/workspace/append.bin")
        assert content == b"\x01\x02\x03\x04"

    def test_delete_file(self, vfs):
        """Test deleting a file."""
        vfs.write_text("/workspace/to_delete.txt", "delete me")
        assert vfs.exists("/workspace/to_delete.txt")
        vfs.delete("/workspace/to_delete.txt")
        assert not vfs.exists("/workspace/to_delete.txt")

    def test_delete_nonexistent_file(self, vfs):
        """Test deleting a file that doesn't exist."""
        with pytest.raises(IOError):
            vfs.delete("/workspace/nonexistent.txt")


class TestVfsDirectoryOperations:
    """Tests for VFS directory operations."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_list_empty_directory(self, vfs):
        """Test listing an empty directory."""
        entries = vfs.list("/workspace")
        assert entries == []

    def test_list_directory_with_files(self, vfs):
        """Test listing a directory with files."""
        vfs.write_text("/workspace/file1.txt", "content1")
        vfs.write_text("/workspace/file2.txt", "content2")

        entries = vfs.list("/workspace")
        names = [e.name for e in entries]
        assert "file1.txt" in names
        assert "file2.txt" in names
        assert len(entries) == 2

    def test_list_nested_directory(self, vfs):
        """Test listing shows subdirectories."""
        vfs.write_text("/workspace/subdir/file.txt", "content")

        entries = vfs.list("/workspace")
        names = [e.name for e in entries]
        assert "subdir" in names

        # Check that subdir is marked as directory
        subdir_entry = next(e for e in entries if e.name == "subdir")
        assert subdir_entry.is_dir

    def test_list_subdirectory(self, vfs):
        """Test listing a subdirectory."""
        vfs.write_text("/workspace/subdir/file1.txt", "content1")
        vfs.write_text("/workspace/subdir/file2.txt", "content2")

        entries = vfs.list("/workspace/subdir")
        names = [e.name for e in entries]
        assert "file1.txt" in names
        assert "file2.txt" in names
        assert len(entries) == 2

    def test_exists_file(self, vfs):
        """Test exists returns True for existing file."""
        vfs.write_text("/workspace/exists.txt", "content")
        assert vfs.exists("/workspace/exists.txt")

    def test_exists_directory(self, vfs):
        """Test exists returns True for existing directory."""
        vfs.write_text("/workspace/subdir/file.txt", "content")
        assert vfs.exists("/workspace/subdir")

    def test_not_exists(self, vfs):
        """Test exists returns False for non-existent path."""
        assert not vfs.exists("/workspace/nonexistent")

    def test_stat_file(self, vfs):
        """Test stat on a file."""
        vfs.write_text("/workspace/stat_test.txt", "hello")
        entry = vfs.stat("/workspace/stat_test.txt")
        assert entry.name == "stat_test.txt"
        assert not entry.is_dir
        assert entry.size == 5

    def test_stat_directory(self, vfs):
        """Test stat on a directory."""
        vfs.write_text("/workspace/subdir/file.txt", "content")
        entry = vfs.stat("/workspace/subdir")
        assert entry.name == "subdir"
        assert entry.is_dir

    def test_stat_nonexistent(self, vfs):
        """Test stat on non-existent path raises error."""
        with pytest.raises(IOError):
            vfs.stat("/workspace/nonexistent")

    def test_write_creates_parent_directories(self, vfs):
        """Test that writing creates parent directories."""
        vfs.write_text("/workspace/a/b/c/deep.txt", "deep content")
        content = vfs.read_text("/workspace/a/b/c/deep.txt")
        assert content == "deep content"
        assert vfs.exists("/workspace/a/b/c")
        assert vfs.exists("/workspace/a/b")
        assert vfs.exists("/workspace/a")


class TestVfsPathHandling:
    """Tests for path handling edge cases."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_path_with_trailing_slash(self, vfs):
        """Test paths with trailing slashes."""
        vfs.write_text("/workspace/file.txt", "content")
        entries = vfs.list("/workspace/")
        names = [e.name for e in entries]
        assert "file.txt" in names

    def test_path_normalization(self, vfs):
        """Test path normalization with double slashes."""
        vfs.write_text("/workspace//file.txt", "content")
        content = vfs.read_text("/workspace/file.txt")
        assert content == "content"

    def test_special_characters_in_filename(self, vfs):
        """Test filenames with special characters."""
        vfs.write_text("/workspace/file-with_special.chars.txt", "content")
        content = vfs.read_text("/workspace/file-with_special.chars.txt")
        assert content == "content"

    def test_spaces_in_filename(self, vfs):
        """Test filenames with spaces."""
        vfs.write_text("/workspace/file with spaces.txt", "content")
        content = vfs.read_text("/workspace/file with spaces.txt")
        assert content == "content"

    def test_no_mount_error(self, vfs):
        """Test accessing path outside mounts raises error."""
        with pytest.raises(IOError):
            vfs.read("/nonexistent_mount/file.txt")


class TestVfsTools:
    """Tests for VFS tool generation."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_tools_json_format(self, vfs):
        """Test generating tools in JSON format."""
        import json
        tools_json = vfs.tools("json")
        tools = json.loads(tools_json)
        assert "tools" in tools
        assert len(tools["tools"]) > 0

    def test_tools_mcp_format(self, vfs):
        """Test generating tools in MCP format."""
        import json
        tools_json = vfs.tools("mcp")
        tools = json.loads(tools_json)
        assert "tools" in tools
        # MCP format should have input_schema
        for tool in tools["tools"]:
            assert "name" in tool
            assert "input_schema" in tool

    def test_tools_openai_format(self, vfs):
        """Test generating tools in OpenAI format."""
        import json
        tools_json = vfs.tools("openai")
        tools = json.loads(tools_json)
        assert "tools" in tools
        # OpenAI format should have function
        for tool in tools["tools"]:
            assert tool["type"] == "function"
            assert "function" in tool

    def test_tools_default_format(self, vfs):
        """Test generating tools with default format."""
        import json
        tools_json = vfs.tools()
        tools = json.loads(tools_json)
        assert "tools" in tools

    def test_tools_invalid_format(self, vfs):
        """Test that invalid format raises error."""
        with pytest.raises(ValueError):
            vfs.tools("invalid_format")


class TestEntry:
    """Tests for Entry class."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_entry_file_attributes(self, vfs):
        """Test Entry attributes for a file."""
        vfs.write_text("/workspace/test.txt", "content")
        entry = vfs.stat("/workspace/test.txt")

        assert hasattr(entry, "path")
        assert hasattr(entry, "name")
        assert hasattr(entry, "is_dir")
        assert hasattr(entry, "size")

        assert entry.name == "test.txt"
        assert not entry.is_dir
        assert entry.size == 7  # "content"

    def test_entry_directory_attributes(self, vfs):
        """Test Entry attributes for a directory."""
        vfs.write_text("/workspace/subdir/file.txt", "content")
        entry = vfs.stat("/workspace/subdir")

        assert entry.name == "subdir"
        assert entry.is_dir
        # Directory size may be None or 0


class TestVfsRepr:
    """Tests for VFS string representation."""

    def test_vfs_repr(self):
        """Test VFS __repr__ method."""
        with tempfile.TemporaryDirectory() as tmpdir:
            config = f"""
name: repr-test
backends:
  local:
    type: fs
    root: {tmpdir}
mounts:
  - path: /data
    backend: local
"""
            vfs = ax.load_config(config)
            repr_str = repr(vfs)
            assert "repr-test" in repr_str
            assert "/data" in repr_str


class TestConcurrency:
    """Tests for concurrent access patterns."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_multiple_sequential_operations(self, vfs):
        """Test many sequential operations."""
        for i in range(100):
            vfs.write_text(f"/workspace/file_{i}.txt", f"content_{i}")

        for i in range(100):
            content = vfs.read_text(f"/workspace/file_{i}.txt")
            assert content == f"content_{i}"

    def test_interleaved_read_write(self, vfs):
        """Test interleaved read and write operations."""
        vfs.write_text("/workspace/interleaved.txt", "initial")

        for i in range(50):
            content = vfs.read_text("/workspace/interleaved.txt")
            vfs.write_text("/workspace/interleaved.txt", f"iteration_{i}")

        final = vfs.read_text("/workspace/interleaved.txt")
        assert final == "iteration_49"


class TestRenameAndCopy:
    """Tests for rename and copy operations."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_rename_file(self, vfs):
        """Test renaming a file."""
        vfs.write_text("/workspace/original.txt", "content")
        vfs.rename("/workspace/original.txt", "/workspace/renamed.txt")
        assert not vfs.exists("/workspace/original.txt")
        assert vfs.exists("/workspace/renamed.txt")
        assert vfs.read_text("/workspace/renamed.txt") == "content"

    def test_copy_file(self, vfs):
        """Test copying a file."""
        vfs.write_text("/workspace/src.txt", "copy me")
        bytes_copied = vfs.copy("/workspace/src.txt", "/workspace/dst.txt")
        assert bytes_copied == 7
        assert vfs.exists("/workspace/src.txt")
        assert vfs.exists("/workspace/dst.txt")
        assert vfs.read_text("/workspace/dst.txt") == "copy me"

    def test_copy_nonexistent(self, vfs):
        """Test copying a file that doesn't exist."""
        with pytest.raises(IOError):
            vfs.copy("/workspace/nonexistent.txt", "/workspace/dst.txt")


class TestGrep:
    """Tests for grep operations."""

    @pytest.fixture
    def vfs(self):
        """Create a VFS instance for testing."""
        self.tmpdir = tempfile.mkdtemp()
        config = f"""
name: test-vfs
backends:
  local:
    type: fs
    root: {self.tmpdir}
mounts:
  - path: /workspace
    backend: local
"""
        vfs = ax.load_config(config)
        yield vfs
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_grep_single_file(self, vfs):
        """Test grep on a single file."""
        vfs.write_text("/workspace/test.txt", "hello world\nfoo bar\nhello again")
        matches = vfs.grep("hello", "/workspace/test.txt")
        assert len(matches) == 2
        assert matches[0].line_number == 1
        assert matches[1].line_number == 3

    def test_grep_directory(self, vfs):
        """Test grep on a directory."""
        vfs.write_text("/workspace/a.txt", "hello world")
        vfs.write_text("/workspace/b.txt", "goodbye world")
        matches = vfs.grep("hello", "/workspace")
        assert len(matches) == 1

    def test_grep_recursive(self, vfs):
        """Test grep with recursion."""
        vfs.write_text("/workspace/a.txt", "hello top")
        vfs.write_text("/workspace/sub/b.txt", "hello nested")
        matches = vfs.grep("hello", "/workspace", True)
        assert len(matches) == 2

    def test_grep_no_matches(self, vfs):
        """Test grep with no matches."""
        vfs.write_text("/workspace/test.txt", "hello world")
        matches = vfs.grep("notfound", "/workspace/test.txt")
        assert len(matches) == 0

    def test_grep_match_attributes(self, vfs):
        """Test that GrepMatch has expected attributes."""
        vfs.write_text("/workspace/test.txt", "hello world")
        matches = vfs.grep("hello", "/workspace/test.txt")
        assert len(matches) == 1
        m = matches[0]
        assert hasattr(m, "path")
        assert hasattr(m, "line_number")
        assert hasattr(m, "line")
        assert m.path == "/workspace/test.txt"
        assert m.line_number == 1
        assert "hello" in m.line


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

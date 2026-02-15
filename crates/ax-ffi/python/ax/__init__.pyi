"""Type stubs for the AX Virtual Filesystem Python bindings."""

from typing import List, Optional

class Entry:
    """A file or directory entry."""
    path: str
    name: str
    is_dir: bool
    size: Optional[int]

class GrepMatch:
    """A single grep match."""
    path: str
    line_number: int
    line: str

class Vfs:
    """AX Virtual Filesystem."""

    @staticmethod
    def from_yaml(yaml: str) -> "Vfs":
        """Create a VFS from a YAML configuration string."""
        ...

    @staticmethod
    def from_file(path: str) -> "Vfs":
        """Create a VFS from a YAML configuration file."""
        ...

    def read(self, path: str) -> bytes:
        """Read the contents of a file."""
        ...

    def read_text(self, path: str) -> str:
        """Read the contents of a file as a string."""
        ...

    def write(self, path: str, content: bytes) -> None:
        """Write content to a file."""
        ...

    def write_text(self, path: str, content: str) -> None:
        """Write a string to a file."""
        ...

    def append(self, path: str, content: bytes) -> None:
        """Append content to a file."""
        ...

    def append_text(self, path: str, content: str) -> None:
        """Append a string to a file."""
        ...

    def delete(self, path: str) -> None:
        """Delete a file."""
        ...

    def list(self, path: str) -> List[Entry]:
        """List files in a directory."""
        ...

    def exists(self, path: str) -> bool:
        """Check if a path exists."""
        ...

    def stat(self, path: str) -> Entry:
        """Get metadata for a path."""
        ...

    def tools(self, format: Optional[str] = None) -> str:
        """Generate tool definitions in JSON format.

        Args:
            format: Output format - 'json', 'mcp', or 'openai'

        Returns:
            JSON string with tool definitions
        """
        ...

    def name(self) -> Optional[str]:
        """Get the VFS name."""
        ...

    def mounts(self) -> List[str]:
        """Get mount paths."""
        ...

    def rename(self, from_path: str, to_path: str) -> None:
        """Rename/move a file.

        Args:
            from_path: Source path
            to_path: Destination path
        """
        ...

    def copy(self, src: str, dst: str) -> int:
        """Copy a file. Returns the number of bytes copied.

        Args:
            src: Source path
            dst: Destination path
        """
        ...

    def grep(self, pattern: str, path: Optional[str] = None, recursive: Optional[bool] = None) -> List[GrepMatch]:
        """Search files for lines matching a regex pattern.

        Args:
            pattern: Regular expression pattern
            path: Path to search (default: "/")
            recursive: Whether to search recursively (default: False)
        """
        ...

def load_config(yaml: str) -> Vfs:
    """Parse a YAML configuration string and return a VFS."""
    ...

def load_config_file(path: str) -> Vfs:
    """Load a VFS from a configuration file."""
    ...

# Re-export for convenience
PyVfs = Vfs
PyEntry = Entry
PyGrepMatch = GrepMatch

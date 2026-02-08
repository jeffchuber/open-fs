"""AX - Agentic Files Virtual Filesystem.

This module provides Python bindings for the AX virtual filesystem,
which allows unified access to multiple backends (local filesystem,
S3, Chroma, etc.) with support for caching, syncing, and AI-powered
search.

Example:
    >>> import ax
    >>> vfs = ax.load_config('''
    ... name: my-workspace
    ... backends:
    ...   local:
    ...     type: fs
    ...     root: ./data
    ... mounts:
    ...   - path: /workspace
    ...     backend: local
    ... ''')
    >>> vfs.write_text('/workspace/hello.txt', 'Hello, world!')
    >>> print(vfs.read_text('/workspace/hello.txt'))
    Hello, world!
"""

from .ax import (
    Vfs as PyVfs,
    Entry as PyEntry,
    load_config,
    load_config_file,
)

# Aliases
Vfs = PyVfs
Entry = PyEntry

__all__ = [
    "Vfs",
    "Entry",
    "PyVfs",
    "PyEntry",
    "load_config",
    "load_config_file",
]

__version__ = "0.1.0"

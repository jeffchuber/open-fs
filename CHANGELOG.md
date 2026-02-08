# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2024-01-XX

### Added

#### Core Features
- Virtual filesystem with mount-based routing
- Multiple backend support (local filesystem, S3, PostgreSQL, Chroma)
- Configuration via YAML with environment variable interpolation
- Smart default inference for mount configurations

#### Backends
- **Local Filesystem**: Full POSIX-like operations with path traversal protection
- **Memory**: In-memory backend for testing
- **S3**: AWS S3 and S3-compatible storage (MinIO, DigitalOcean Spaces, etc.)
- **PostgreSQL**: Store files in PostgreSQL with automatic table creation
- **Chroma**: Vector database integration for semantic search

#### Caching & Sync
- LRU cache with configurable TTL, max entries, and max size
- Write-through sync mode for immediate remote writes
- Write-back sync mode with background flushing
- Cache statistics and metrics

#### Indexing & Search
- Text chunking strategies: fixed, recursive, semantic
- Embedding providers: Ollama, OpenAI, stub (for testing)
- BM25 sparse encoding for keyword search
- Hybrid search combining dense and sparse vectors
- PDF text extraction (optional feature)

#### AI Integration
- Tool definition generation for AI assistants
- MCP (Model Context Protocol) format support
- OpenAI function calling format support
- JSON schema generation for tools

#### CLI
- 18 commands for file operations and management
- `ls`, `cat`, `write`, `append`, `rm`, `stat`, `exists`
- `cp`, `mv`, `tree`, `find`, `grep`
- `index`, `search` for semantic search
- `config`, `status`, `watch`, `tools`

#### Language Bindings
- Python bindings via PyO3 (ax-ffi crate)
- TypeScript/Node.js bindings via napi-rs (ax-js crate)

#### Observability
- Tracing instrumentation with `tracing` crate
- Metrics collection for operations, cache, and errors
- Structured logging with span context

#### Testing
- 88+ unit tests across all crates
- Integration tests for CLI commands
- Benchmark suite for performance testing

### Security
- Path traversal protection in filesystem backend
- No hardcoded credentials (environment variable support)
- Read-only mount support

## [Unreleased]

### Planned
- WebDAV backend
- SFTP backend
- Watch mode with file change notifications
- Incremental indexing
- Vector index caching
- Multi-tenant support

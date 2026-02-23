# Write Path Architecture for OpenFS

## Context

The read/query path is solid, but the write path is incomplete. The core challenge: **local edits need to reach various backends, and different backends require different preprocessing**. A write to S3 is just raw bytes, but a write to Chroma needs chunking + embedding first. Orthogonally, writes can be immediate (write-through) or deferred (WAL → async flush). These two dimensions — **preprocessing** and **timing** — need to compose cleanly.

The infrastructure is mostly there (WAL, SyncEngine skeleton, outbox replay, IndexingPipeline, PersistentWorker), but the glue is missing.

## Design: `WriteTransform` trait

Introduce a **`WriteTransform`** trait as a separate concern from `Backend`. Backends stay pure storage; transforms are preprocessing that sits between the caller and the backend.

```
VFS Write
  → Cache (raw bytes, for local reads)
  → WAL (raw bytes, for durability)
  → WriteTransform (backend-specific preprocessing)
  → Backend.write()
```

**Key decision: store raw bytes in WAL, transform on flush.** Rationale:
- Embedding is expensive (external API call) — shouldn't block writes
- WAL stays simple (no variable-width structured data for chunks)
- Failed transforms are retryable from raw bytes
- For write-through mode, transforms still happen inline (user's explicit choice)

### The trait

Add to `openfs-core/src/traits.rs`:

```rust
#[async_trait]
pub trait WriteTransform: Send + Sync + 'static {
    async fn transform_write(&self, path: &str, content: &[u8])
        -> Result<Vec<TransformedWrite>, BackendError>;
    async fn transform_delete(&self, path: &str)
        -> Result<Vec<TransformedDelete>, BackendError>;
}
```

`TransformedWrite` carries: path, content, optional embedding vector, optional metadata.

### Two implementations

1. **`IdentityTransform`** (openfs-core) — passthrough, for fs/s3/postgres
2. **`ChunkEmbedTransform`** (openfs-local) — wraps existing `IndexingPipeline`, reuses all chunker/embedder/extractor infrastructure

### Integration point: CachedBackend + flush closure

The transform hooks into `CachedBackend` (per-mount, where sync mode branching already lives):

- **WriteThrough**: `transform_write()` → `backend.write()` for each result, inline
- **WriteBack**: raw bytes → cache + WAL, then on flush: `transform_write()` → `backend.write()`

The flush closure passed to `SyncEngine::start()` (currently just `backend.write()`) becomes `transform.transform_write() → backend.write()`. Same for outbox drain on crash recovery.

### Configuration

Add optional `transform` to `MountConfig`:

```yaml
mounts:
  - path: /workspace
    backend: local_fs
    mode: write_through
    # No transform — identity is default

  - path: /knowledge
    backend: chroma
    mode: write_back
    transform:
      type: chunk_embed
      chunker: recursive
      embedder: openai
    sync:
      interval: 10s

  - path: /agent-logs
    backend: s3_logs
    mode: write_back
    sync:
      interval: 30s
    # Identity transform, just async flush to S3
```

## What changes

| File | Change |
|------|--------|
| `openfs-core/src/traits.rs` | Add `WriteTransform` trait, `TransformedWrite`, `TransformedDelete`, `IdentityTransform` |
| `openfs-config/src/types.rs` | Add `TransformConfig` enum, optional `transform` field on `MountConfig` |
| `openfs-remote/src/cached_backend.rs` | Add `transform: Arc<dyn WriteTransform>` field, wire into WriteThrough write path |
| `openfs-remote/src/vfs.rs` | Construct transforms from config, wire into flush closures + outbox drain |
| `openfs-local/src/pipeline.rs` | Add `ChunkEmbedTransform` wrapping `IndexingPipeline`, impl `WriteTransform` |

## What does NOT change

- **Backend trait** — stays pure storage
- **WAL schema** — stays raw bytes
- **Read path** — completely unchanged
- **SyncEngine internals** — transform is applied in the flush closure, outside the engine

## Edge cases

- **Appends**: For ChunkEmbedTransform, append means re-chunk the full content (chunk boundaries shift). CachedBackend already computes full content for WriteBack appends.
- **CAS**: Only makes sense with IdentityTransform. Reject CAS + non-identity transform at config validation time.
- **Cache consistency**: Cache stores raw bytes (what the caller wrote). Backend stores transformed output. This is correct — local reads get raw content, search queries hit Chroma's transformed data.

## Verification

1. Unit tests for `IdentityTransform` and `ChunkEmbedTransform`
2. Integration test: WriteThrough + IdentityTransform → verify backend receives raw bytes
3. Integration test: WriteBack + IdentityTransform → verify WAL stores raw, flush delivers to backend
4. Integration test: WriteBack + ChunkEmbedTransform → verify chunks + embeddings land in Chroma after flush
5. Config validation test: reject CAS + non-identity transform

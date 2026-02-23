use std::sync::Arc;
use std::time::Duration;

use openfs_core::{Backend, CacheConfig, VfsError};
use openfs_remote::{
    CachedBackend, MemoryBackend, Mount, Router, SyncConfig, SyncMode, WalConfig, WriteAheadLog,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::backend_wrapper::DynBackend;
use crate::fault::{FaultConfig, FaultStats, FaultyBackend};
use crate::mock_chroma::MockChromaStore;

/// A simulated agent with its own router, mounts, and access to shared backends.
pub struct AgentVm {
    pub id: usize,
    pub router: Router,
    /// Direct handles for oracle inspection (bypass cache/router).
    pub work_backend: Arc<MemoryBackend>,
    pub indexed_backend: Arc<MemoryBackend>,
    pub shared_read: Arc<MemoryBackend>,
    pub shared_write: Arc<MemoryBackend>,
    pub chroma: Arc<MockChromaStore>,
    /// Handle for write-back cached backend (agent 1's indexed mount), for shutdown.
    pub write_back_handle: Option<Arc<CachedBackend<DynBackend>>>,
    /// Fault injection backends, if active.
    pub faulty_backends: Vec<Arc<FaultyBackend>>,
}

impl AgentVm {
    /// Shutdown the write-back sync engine if present.
    pub async fn shutdown(&self) {
        if let Some(handle) = &self.write_back_handle {
            handle.shutdown_sync().await;
        }
    }

    /// Aggregate fault statistics across all wrapped backends.
    pub fn fault_stats(&self) -> FaultStats {
        let mut fault_count = 0;
        let mut corruption_count = 0;
        for fb in &self.faulty_backends {
            let stats = fb.stats();
            fault_count += stats.fault_count;
            corruption_count += stats.corruption_count;
        }
        FaultStats {
            fault_count,
            corruption_count,
        }
    }
}

/// Build simulation agents sharing the same shared backends and chroma store.
///
/// Agent 0's indexed mount uses write-through (immediate backend writes).
/// Agent 1's indexed mount:
///   - If `enable_write_back` is false: uses with_cache (SyncMode::None + cache).
///   - If `enable_write_back` is true: uses write-back with 1-second flush interval.
/// When `include_remote_writer` is true, agents 1 and 2 share a single indexed
/// backing backend (`remote0`) to model multi-client sync through one remote store.
/// Agent 2 (optional) is a dedicated remote-writer style client with
/// write-through indexed behavior.
///
/// When `fault_config` is Some, each MemoryBackend is wrapped in a FaultyBackend
/// before being passed to CachedBackend.
pub async fn build_agents(
    shared_read: Arc<MemoryBackend>,
    shared_write: Arc<MemoryBackend>,
    chroma: Arc<MockChromaStore>,
    fault_config: Option<FaultConfig>,
    enable_write_back: bool,
    include_remote_writer: bool,
    master_rng: &mut ChaCha8Rng,
) -> Vec<AgentVm> {
    // In the remote-client topology, agents 1 and 2 share one indexed backend.
    let shared_indexed_12 = include_remote_writer.then(|| Arc::new(MemoryBackend::new()));

    let a0 = build_agent(
        0,
        shared_read.clone(),
        shared_write.clone(),
        None,
        chroma.clone(),
        fault_config.clone(),
        false, // agent 0 always write-through
        master_rng,
    )
    .await;
    let a1 = build_agent(
        1,
        shared_read.clone(),
        shared_write.clone(),
        shared_indexed_12.clone(),
        chroma.clone(),
        fault_config.clone(),
        enable_write_back,
        master_rng,
    )
    .await;
    let mut agents = vec![a0, a1];

    if include_remote_writer {
        let a2 = build_agent(
            2,
            shared_read,
            shared_write,
            shared_indexed_12,
            chroma,
            fault_config,
            false,
            master_rng,
        )
        .await;
        agents.push(a2);
    }

    agents
}

async fn build_agent(
    id: usize,
    shared_read: Arc<MemoryBackend>,
    shared_write: Arc<MemoryBackend>,
    indexed_backend_override: Option<Arc<MemoryBackend>>,
    chroma: Arc<MockChromaStore>,
    fault_config: Option<FaultConfig>,
    enable_write_back: bool,
    master_rng: &mut ChaCha8Rng,
) -> AgentVm {
    let work_backend = Arc::new(MemoryBackend::new());
    let indexed_backend =
        indexed_backend_override.unwrap_or_else(|| Arc::new(MemoryBackend::new()));

    let prefix = match id {
        0 => "/a0",
        1 => "/a1",
        2 => "/a2",
        _ => "/a1",
    };

    let mut faulty_backends: Vec<Arc<FaultyBackend>> = Vec::new();

    // Optionally wrap backends in FaultyBackend
    let work_dyn: Arc<dyn Backend> = if let Some(ref fc) = fault_config {
        use rand::Rng;
        let seed: u64 = master_rng.gen();
        let fb = Arc::new(FaultyBackend::new(
            work_backend.clone() as Arc<dyn Backend>,
            ChaCha8Rng::seed_from_u64(seed),
            fc.clone(),
        ));
        faulty_backends.push(fb.clone());
        fb as Arc<dyn Backend>
    } else {
        work_backend.clone() as Arc<dyn Backend>
    };

    let indexed_dyn: Arc<dyn Backend> = if let Some(ref fc) = fault_config {
        use rand::Rng;
        let seed: u64 = master_rng.gen();
        let fb = Arc::new(FaultyBackend::new(
            indexed_backend.clone() as Arc<dyn Backend>,
            ChaCha8Rng::seed_from_u64(seed),
            fc.clone(),
        ));
        faulty_backends.push(fb.clone());
        fb as Arc<dyn Backend>
    } else {
        indexed_backend.clone() as Arc<dyn Backend>
    };

    let shared_read_dyn: Arc<dyn Backend> = if let Some(ref fc) = fault_config {
        use rand::Rng;
        let seed: u64 = master_rng.gen();
        let fb = Arc::new(FaultyBackend::new(
            shared_read.clone() as Arc<dyn Backend>,
            ChaCha8Rng::seed_from_u64(seed),
            fc.clone(),
        ));
        faulty_backends.push(fb.clone());
        fb as Arc<dyn Backend>
    } else {
        shared_read.clone() as Arc<dyn Backend>
    };

    let shared_write_dyn: Arc<dyn Backend> = if let Some(ref fc) = fault_config {
        use rand::Rng;
        let seed: u64 = master_rng.gen();
        let fb = Arc::new(FaultyBackend::new(
            shared_write.clone() as Arc<dyn Backend>,
            ChaCha8Rng::seed_from_u64(seed),
            fc.clone(),
        ));
        faulty_backends.push(fb.clone());
        fb as Arc<dyn Backend>
    } else {
        shared_write.clone() as Arc<dyn Backend>
    };

    // --- Work mount: no cache, no sync ---
    let work_cached = CachedBackend::new(
        DynBackend(work_dyn),
        CacheConfig {
            enabled: false,
            ..Default::default()
        },
        SyncConfig::default(),
        false,
    );

    // --- Indexed mount ---
    let cache_config = CacheConfig::default();
    let mut write_back_handle: Option<Arc<CachedBackend<DynBackend>>> = None;

    let indexed_cached: Arc<CachedBackend<DynBackend>> = if enable_write_back {
        // Write-back mode: writes go to cache, background flush pushes to backend
        let inner_for_flush = indexed_backend.clone();
        let wal = Arc::new(
            WriteAheadLog::in_memory(WalConfig::default())
                .expect("in-memory WAL should initialize for simulation"),
        );
        let cb = Arc::new(CachedBackend::new_with_wal(
            DynBackend(indexed_dyn),
            cache_config,
            SyncConfig {
                mode: SyncMode::WriteBack,
                flush_interval: Duration::from_secs(1),
                ..Default::default()
            },
            false,
            wal,
        ));
        // Start sync with a flush function that writes to the inner MemoryBackend
        cb.start_sync(move |path: String, content: Vec<u8>| {
            let backend = inner_for_flush.clone();
            async move {
                backend
                    .write(&path, &content)
                    .await
                    .map_err(|e| VfsError::Backend(Box::new(e)))
            }
        })
        .await;
        write_back_handle = Some(cb.clone());
        cb
    } else if id == 0 || id == 2 {
        // Agents 0 and 2 default: write-through — writes go to backend immediately + cache
        Arc::new(CachedBackend::write_through(
            DynBackend(indexed_dyn),
            cache_config,
        ))
    } else {
        // Agent 1 default: with_cache — SyncMode::None + cache, exercises LRU cache
        Arc::new(CachedBackend::with_cache(
            DynBackend(indexed_dyn),
            cache_config,
        ))
    };

    // --- Shared read mount: cached, pull-mirror (read-only) ---
    let shared_read_cached =
        CachedBackend::pull_mirror(DynBackend(shared_read_dyn), CacheConfig::default());

    // --- Shared write mount: cached, write-through ---
    let shared_write_cached =
        CachedBackend::write_through(DynBackend(shared_write_dyn), CacheConfig::default());

    // Build router with all 4 mounts
    let mounts = vec![
        Mount {
            path: format!("{}/work", prefix),
            backend: Arc::new(work_cached),
            read_only: false,
        },
        Mount {
            path: format!("{}/indexed", prefix),
            backend: indexed_cached,
            read_only: false,
        },
        Mount {
            path: "/shared/read".to_string(),
            backend: Arc::new(shared_read_cached),
            read_only: true,
        },
        Mount {
            path: "/shared/write".to_string(),
            backend: Arc::new(shared_write_cached),
            read_only: false,
        },
    ];

    let router = Router::new(mounts);

    AgentVm {
        id,
        router,
        work_backend,
        indexed_backend,
        shared_read,
        shared_write,
        chroma,
        write_back_handle,
        faulty_backends,
    }
}

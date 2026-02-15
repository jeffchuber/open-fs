use std::collections::HashMap;
use std::time::Duration;

use ax_local::{
    IndexingPipeline, PipelineConfig, WatchEngine,
    WorkQueue, WorkQueueConfig, QueueEventType,
};
use ax_remote::Vfs;

pub async fn run(
    vfs: &Vfs,
    path: Option<String>,
    interval_secs: u64,
    poll: bool,
    auto_index: bool,
    webhook: Option<String>,
    debounce_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.unwrap_or_else(|| "/".to_string());

    // Set up work queue and pipeline if auto_index is enabled
    let mut indexer = if auto_index {
        Some(WatchIndexer::new(debounce_ms)?)
    } else {
        None
    };

    // Try native mode if not explicitly polling
    let fs_path = if !poll { vfs.resolve_fs_path(&path) } else { None };

    if let Some(ref fs_root) = fs_path {
        println!("Watching {} (native mode, fs root: {})", path, fs_root.display());
        run_native(vfs, &path, fs_root, &mut indexer, webhook).await
    } else {
        if !poll {
            println!("No local filesystem backend for '{}', falling back to polling mode", path);
        }
        println!("Watching {} for changes (polling, interval: {}s)", path, interval_secs);
        println!("Press Ctrl+C to stop");
        println!();
        run_polling(vfs, &path, interval_secs, &mut indexer, webhook).await
    }
}

/// Work queue-backed indexer for watch mode.
///
/// Events go into a SQLite-backed work queue (debounce + dedup via upsert semantics).
/// A separate processing loop drains ready items and runs them through the indexing pipeline.
struct WatchIndexer {
    pipeline: IndexingPipeline,
    queue: WorkQueue,
}

impl WatchIndexer {
    fn new(debounce_ms: u64) -> Result<Self, Box<dyn std::error::Error>> {
        let queue_path = std::path::Path::new(".").join(".ax_watch_queue.db");
        let debounce_secs = std::cmp::max(1, debounce_ms / 1000);
        let queue = WorkQueue::open(
            &queue_path,
            WorkQueueConfig {
                debounce_secs,
                max_retries: 3,
                base_backoff_secs: 2,
            },
        ).map_err(|e| format!("Failed to open work queue: {}", e))?;

        // Recover any items stuck in "processing" from a previous crash
        match queue.recover_stuck() {
            Ok(n) if n > 0 => eprintln!("Recovered {} stuck work queue items from previous run", n),
            Ok(_) => {}
            Err(e) => eprintln!("Warning: failed to recover stuck items: {}", e),
        }

        let pipeline = IndexingPipeline::new(PipelineConfig::default())?;

        Ok(WatchIndexer { pipeline, queue })
    }

    /// Enqueue a file change event (non-blocking, just writes to SQLite).
    fn enqueue(&self, path: &str, change_kind: &str) {
        let event_type = match change_kind {
            "deleted" => QueueEventType::Deleted,
            _ => QueueEventType::Changed,
        };
        if let Err(e) = self.queue.enqueue(path, event_type) {
            eprintln!("  warning: failed to enqueue {}: {}", path, e);
        }
    }

    /// Process all ready items from the work queue.
    async fn process_ready(&self, vfs: &Vfs) {
        let items = match self.queue.fetch_ready(32) {
            Ok(items) => items,
            Err(e) => {
                eprintln!("  warning: failed to fetch work queue items: {}", e);
                return;
            }
        };

        for item in items {
            match item.event_type {
                QueueEventType::Changed => {
                    match vfs.read(&item.path).await {
                        Ok(content) => {
                            match self.pipeline.index_file(&item.path, &content).await {
                                Ok(result) => {
                                    eprintln!("  indexed: {} ({} chunks)", item.path, result.chunks_created);
                                    if let Err(e) = self.queue.complete(item.id) {
                                        eprintln!("  warning: failed to complete queue item: {}", e);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("  index failed for {}: {}", item.path, e);
                                    if let Err(e2) = self.queue.fail(item.id, &e.to_string()) {
                                        eprintln!("  warning: failed to mark queue item as failed: {}", e2);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("  warning: could not read {} for indexing: {}", item.path, e);
                            if let Err(e2) = self.queue.fail(item.id, &e.to_string()) {
                                eprintln!("  warning: failed to mark queue item as failed: {}", e2);
                            }
                        }
                    }
                }
                QueueEventType::Deleted => {
                    if let Err(e) = self.pipeline.delete_file(&item.path).await {
                        eprintln!("  warning: failed to remove {} from index: {}", item.path, e);
                    }
                    if let Err(e) = self.queue.complete(item.id) {
                        eprintln!("  warning: failed to complete queue item: {}", e);
                    }
                }
            }
        }
    }
}

async fn run_native(
    vfs: &Vfs,
    vfs_path: &str,
    fs_root: &std::path::Path,
    indexer: &mut Option<WatchIndexer>,
    webhook: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = WatchEngine::new()?;
    engine.watch_path(fs_root)?;
    let mut rx = engine.take_receiver().ok_or("Failed to get watch receiver")?;

    println!("Press Ctrl+C to stop");
    println!();

    loop {
        tokio::select! {
            change = rx.recv() => {
                let change = match change {
                    Some(c) => c,
                    None => break,
                };

                // Convert fs path back to vfs path
                let relative = change.path.strip_prefix(fs_root).unwrap_or(&change.path);
                let change_vfs_path = if vfs_path == "/" {
                    format!("/{}", relative.display())
                } else {
                    format!("{}/{}", vfs_path, relative.display())
                };

                let time_str = chrono::Local::now().format("%H:%M:%S");
                println!("[{}] {}: {}", time_str, change.kind, change_vfs_path);

                handle_change(&change_vfs_path, &change.kind.to_string(), indexer, &webhook).await;
            }
            // Process work queue every 500ms
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                if let Some(ref idx) = indexer {
                    idx.process_ready(vfs).await;
                }
            }
        }
    }

    Ok(())
}

async fn run_polling(
    vfs: &Vfs,
    path: &str,
    interval_secs: u64,
    indexer: &mut Option<WatchIndexer>,
    webhook: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let interval = Duration::from_secs(interval_secs);

    // Track file states
    let mut file_states: HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)> = HashMap::new();

    // Initial scan
    scan_directory(vfs, path, &mut file_states).await?;
    println!("Initial scan: {} files", file_states.len());
    println!();

    loop {
        tokio::time::sleep(interval).await;

        let mut new_states: HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)> = HashMap::new();
        scan_directory(vfs, path, &mut new_states).await?;

        // Check for changes
        let now = chrono::Local::now().format("%H:%M:%S");

        // New or modified files
        for (file_path, (size, modified)) in &new_states {
            if let Some((old_size, old_modified)) = file_states.get(file_path) {
                if size != old_size || modified != old_modified {
                    println!("[{}] modified: {}", now, file_path);
                    handle_change(file_path, "modified", indexer, &webhook).await;
                }
            } else {
                println!("[{}] created: {}", now, file_path);
                handle_change(file_path, "created", indexer, &webhook).await;
            }
        }

        // Deleted files
        for file_path in file_states.keys() {
            if !new_states.contains_key(file_path) {
                println!("[{}] deleted: {}", now, file_path);
                handle_change(file_path, "deleted", indexer, &webhook).await;
            }
        }

        file_states = new_states;

        // Process work queue
        if let Some(ref idx) = indexer {
            idx.process_ready(vfs).await;
        }
    }
}

async fn handle_change(
    path: &str,
    change_kind: &str,
    indexer: &mut Option<WatchIndexer>,
    webhook: &Option<String>,
) {
    // Enqueue for indexing via work queue (non-blocking)
    if let Some(ref idx) = indexer {
        idx.enqueue(path, change_kind);
    }

    // Webhook POST
    if let Some(ref url) = webhook {
        let url = url.clone();
        let path = path.to_string();
        let kind = change_kind.to_string();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let payload = serde_json::json!({
                "path": path,
                "change": kind,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });

            match tokio::time::timeout(
                Duration::from_secs(5),
                client.post(&url).json(&payload).send(),
            )
            .await
            {
                Ok(Ok(resp)) => {
                    if !resp.status().is_success() {
                        eprintln!("  webhook returned {}: {}", resp.status(), url);
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("  webhook failed for {}: {}", url, e);
                }
                Err(_) => {
                    eprintln!("  webhook timed out for {}", url);
                }
            }
        });
    }
}

#[async_recursion::async_recursion]
async fn scan_directory(
    vfs: &Vfs,
    dir_path: &str,
    states: &mut HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = vfs.list(dir_path).await?;

    for entry in entries {
        if entry.is_dir {
            scan_directory(vfs, &entry.path, states).await?;
        } else {
            let size = entry.size.unwrap_or(0);
            states.insert(entry.path.clone(), (size, entry.modified));
        }
    }

    Ok(())
}

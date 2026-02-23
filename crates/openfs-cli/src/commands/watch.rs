use std::collections::HashMap;
use std::time::Duration;

use openfs_config::{VfsConfig, WatchConfig};
use openfs_local::{
    IndexingPipeline, PipelineConfig, QueueEventType, WatchEngine, WorkQueue, WorkQueueConfig,
};
use openfs_remote::Vfs;
use regex::Regex;

#[derive(Clone)]
struct PathFilters {
    includes: Vec<Regex>,
    excludes: Vec<Regex>,
}

impl PathFilters {
    fn from_watch_config(watch: Option<&WatchConfig>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut includes = Vec::new();
        let mut excludes = Vec::new();

        if let Some(watch) = watch {
            for pattern in &watch.include {
                includes
                    .push(Regex::new(pattern).map_err(|e| {
                        format!("Invalid watch.include regex '{}': {}", pattern, e)
                    })?);
            }
            for pattern in &watch.exclude {
                excludes
                    .push(Regex::new(pattern).map_err(|e| {
                        format!("Invalid watch.exclude regex '{}': {}", pattern, e)
                    })?);
            }
        }

        Ok(Self { includes, excludes })
    }

    fn matches(&self, path: &str) -> bool {
        let included = if self.includes.is_empty() {
            true
        } else {
            self.includes.iter().any(|re| re.is_match(path))
        };
        included && !self.excludes.iter().any(|re| re.is_match(path))
    }
}

struct ResolvedWatchSettings {
    interval_secs: u64,
    poll: bool,
    auto_index: bool,
    webhook: Option<String>,
    debounce_ms: u64,
    filters: PathFilters,
}

fn normalize_watch_path(path: &str) -> String {
    let mut normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

fn path_matches_mount(path: &str, mount_path: &str) -> bool {
    mount_path == "/" || path == mount_path || path.starts_with(&format!("{}/", mount_path))
}

fn watch_config_for_path<'a>(config: &'a VfsConfig, path: &str) -> Option<&'a WatchConfig> {
    let mut best: Option<&WatchConfig> = None;
    let mut best_mount_len = 0usize;

    for mount in &config.mounts {
        let watch = match mount.watch.as_ref() {
            Some(watch) => watch,
            None => continue,
        };
        let mount_path = normalize_watch_path(&mount.path);
        if path_matches_mount(path, &mount_path) && mount_path.len() > best_mount_len {
            best = Some(watch);
            best_mount_len = mount_path.len();
        }
    }

    best.or_else(|| config.defaults.as_ref().and_then(|d| d.watch.as_ref()))
}

fn duration_to_ceil_secs(duration: Duration) -> u64 {
    let secs = duration.as_secs();
    if duration.subsec_nanos() == 0 {
        secs
    } else {
        secs.saturating_add(1)
    }
}

fn duration_to_millis(duration: Duration) -> u64 {
    let millis = duration.as_millis();
    if millis > u128::from(u64::MAX) {
        u64::MAX
    } else {
        millis as u64
    }
}

fn resolve_watch_settings(
    vfs: &Vfs,
    path: &str,
    interval_secs: Option<u64>,
    poll: bool,
    auto_index: bool,
    webhook: Option<String>,
    debounce_ms: Option<u64>,
) -> Result<ResolvedWatchSettings, Box<dyn std::error::Error>> {
    let effective = vfs.effective_config();
    let watch_cfg = watch_config_for_path(effective, path);

    let interval_from_config = watch_cfg
        .and_then(|watch| watch.poll_interval.as_ref())
        .map(|d| duration_to_ceil_secs(d.as_duration()));
    let debounce_from_config =
        watch_cfg.map(|watch| duration_to_millis(watch.debounce.as_duration()));

    let interval_secs = interval_secs.or(interval_from_config).unwrap_or(2);
    if interval_secs == 0 {
        return Err("Watch interval must be greater than 0 seconds".into());
    }

    let debounce_ms = debounce_ms.or(debounce_from_config).unwrap_or(500);
    if debounce_ms == 0 {
        return Err("Watch debounce must be greater than 0 milliseconds".into());
    }

    let poll = poll || watch_cfg.map(|watch| !watch.native).unwrap_or(false);
    let auto_index = auto_index || watch_cfg.map(|watch| watch.auto_index).unwrap_or(false);
    let webhook = webhook.or_else(|| watch_cfg.and_then(|watch| watch.webhook_url.clone()));
    let filters = PathFilters::from_watch_config(watch_cfg)?;

    Ok(ResolvedWatchSettings {
        interval_secs,
        poll,
        auto_index,
        webhook,
        debounce_ms,
        filters,
    })
}

pub async fn run(
    vfs: &Vfs,
    path: Option<String>,
    interval_secs: Option<u64>,
    poll: bool,
    auto_index: bool,
    webhook: Option<String>,
    debounce_ms: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = normalize_watch_path(path.as_deref().unwrap_or("/"));
    let settings = resolve_watch_settings(
        vfs,
        &path,
        interval_secs,
        poll,
        auto_index,
        webhook,
        debounce_ms,
    )?;

    // Set up work queue and pipeline if auto_index is enabled
    let mut indexer = if settings.auto_index {
        Some(WatchIndexer::new(settings.debounce_ms)?)
    } else {
        None
    };

    // Try native mode if not explicitly polling
    let fs_path = if !settings.poll {
        vfs.resolve_fs_path(&path)
    } else {
        None
    };

    if let Some(ref fs_root) = fs_path {
        println!(
            "Watching {} (native mode, fs root: {})",
            path,
            fs_root.display()
        );
        run_native(
            vfs,
            &path,
            fs_root,
            &mut indexer,
            settings.webhook.clone(),
            settings.filters.clone(),
        )
        .await
    } else {
        if !settings.poll {
            println!(
                "No local filesystem backend for '{}', falling back to polling mode",
                path
            );
        }
        println!(
            "Watching {} for changes (polling, interval: {}s)",
            path, settings.interval_secs
        );
        println!("Press Ctrl+C to stop");
        println!();
        run_polling(
            vfs,
            &path,
            settings.interval_secs,
            &mut indexer,
            settings.webhook,
            settings.filters,
        )
        .await
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
        let queue_path = std::path::Path::new(".").join(".openfs_watch_queue.db");
        let debounce_secs = std::cmp::max(1, debounce_ms / 1000);
        let queue = WorkQueue::open(
            &queue_path,
            WorkQueueConfig {
                debounce_secs,
                max_retries: 3,
                base_backoff_secs: 2,
            },
        )
        .map_err(|e| format!("Failed to open work queue: {}", e))?;

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
                QueueEventType::Changed => match vfs.read(&item.path).await {
                    Ok(content) => match self.pipeline.index_file(&item.path, &content).await {
                        Ok(result) => {
                            eprintln!(
                                "  indexed: {} ({} chunks)",
                                item.path, result.chunks_created
                            );
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
                    },
                    Err(e) => {
                        eprintln!(
                            "  warning: could not read {} for indexing: {}",
                            item.path, e
                        );
                        if let Err(e2) = self.queue.fail(item.id, &e.to_string()) {
                            eprintln!("  warning: failed to mark queue item as failed: {}", e2);
                        }
                    }
                },
                QueueEventType::Deleted => {
                    if let Err(e) = self.pipeline.delete_file(&item.path).await {
                        eprintln!(
                            "  warning: failed to remove {} from index: {}",
                            item.path, e
                        );
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
    filters: PathFilters,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = WatchEngine::new()?;
    engine.watch_path(fs_root)?;
    let mut rx = engine
        .take_receiver()
        .ok_or("Failed to get watch receiver")?;

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
                if !filters.matches(&change_vfs_path) {
                    continue;
                }

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
    filters: PathFilters,
) -> Result<(), Box<dyn std::error::Error>> {
    let interval = Duration::from_secs(interval_secs);

    // Track file states
    let mut file_states: HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)> =
        HashMap::new();

    // Initial scan
    scan_directory(vfs, path, &mut file_states, &filters).await?;
    println!("Initial scan: {} files", file_states.len());
    println!();

    loop {
        tokio::time::sleep(interval).await;

        let mut new_states: HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)> =
            HashMap::new();
        scan_directory(vfs, path, &mut new_states, &filters).await?;

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
    filters: &PathFilters,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = vfs.list(dir_path).await?;

    for entry in entries {
        if entry.is_dir {
            scan_directory(vfs, &entry.path, states, filters).await?;
        } else {
            let size = entry.size.unwrap_or(0);
            if filters.matches(&entry.path) {
                states.insert(entry.path.clone(), (size, entry.modified));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfs_config::{DefaultsConfig, MountConfig};

    fn watch(native: bool, auto_index: bool, include: &[&str], exclude: &[&str]) -> WatchConfig {
        WatchConfig {
            native,
            poll_interval: None,
            debounce: openfs_config::HumanDuration(std::time::Duration::from_millis(500)),
            auto_index,
            webhook_url: None,
            include: include.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn mount(path: &str, watch: Option<WatchConfig>) -> MountConfig {
        MountConfig {
            path: path.to_string(),
            backend: None,
            collection: None,
            mode: None,
            read_only: false,
            index: None,
            sync: None,
            watch,
        }
    }

    #[test]
    fn test_normalize_watch_path() {
        assert_eq!(normalize_watch_path("/workspace/"), "/workspace");
        assert_eq!(normalize_watch_path("workspace"), "/workspace");
        assert_eq!(normalize_watch_path("/"), "/");
    }

    #[test]
    fn test_watch_config_for_path_prefers_longest_matching_mount() {
        let root_watch = watch(true, false, &[], &[]);
        let nested_watch = watch(false, true, &[], &[]);
        let cfg = VfsConfig {
            mounts: vec![
                mount("/workspace", Some(root_watch)),
                mount("/workspace/sub", Some(nested_watch.clone())),
            ],
            defaults: Some(DefaultsConfig {
                watch: Some(watch(true, false, &[], &[])),
                ..Default::default()
            }),
            ..Default::default()
        };

        let selected = watch_config_for_path(&cfg, "/workspace/sub/file.txt")
            .expect("expected matching watch config");
        assert_eq!(selected.native, nested_watch.native);
        assert_eq!(selected.auto_index, nested_watch.auto_index);
    }

    #[test]
    fn test_path_filters_include_and_exclude() {
        let cfg = watch(true, false, &["^/workspace/.*\\.rs$"], &["/target/"]);
        let filters = PathFilters::from_watch_config(Some(&cfg)).expect("filters should compile");

        assert!(filters.matches("/workspace/src/main.rs"));
        assert!(!filters.matches("/workspace/src/main.txt"));
        assert!(!filters.matches("/workspace/target/gen.rs"));
    }

    #[test]
    fn test_duration_to_ceil_secs_rounds_up() {
        assert_eq!(duration_to_ceil_secs(Duration::from_millis(1)), 1);
        assert_eq!(duration_to_ceil_secs(Duration::from_secs(2)), 2);
    }
}

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use notify::{Config, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use openfs_core::VfsError;

/// The kind of file change detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeKind::Created => write!(f, "created"),
            ChangeKind::Modified => write!(f, "modified"),
            ChangeKind::Deleted => write!(f, "deleted"),
            ChangeKind::Renamed => write!(f, "renamed"),
        }
    }
}

/// A file change event.
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Path of the changed file.
    pub path: PathBuf,
    /// Kind of change.
    pub kind: ChangeKind,
    /// Timestamp of the change.
    pub timestamp: SystemTime,
}

/// Engine for watching filesystem changes using native OS notifications.
pub struct WatchEngine {
    watcher: Option<WatcherImpl>,
    tx: mpsc::Sender<FileChange>,
    rx: Option<mpsc::Receiver<FileChange>>,
}

enum WatcherImpl {
    Recommended(RecommendedWatcher),
    Poll(PollWatcher),
}

impl WatcherImpl {
    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        match self {
            WatcherImpl::Recommended(watcher) => watcher.watch(path, mode),
            WatcherImpl::Poll(watcher) => watcher.watch(path, mode),
        }
    }
}

impl WatchEngine {
    /// Create a new WatchEngine.
    pub fn new() -> Result<Self, VfsError> {
        let (tx, rx) = mpsc::channel::<FileChange>(1024);

        Ok(WatchEngine {
            watcher: None,
            tx,
            rx: Some(rx),
        })
    }

    /// Start watching a filesystem path.
    pub fn watch_path(&mut self, fs_root: &Path) -> Result<(), VfsError> {
        let tx = self.tx.clone();

        let handler = move |result: Result<Event, notify::Error>| match result {
            Ok(event) => {
                if let Some(change) = convert_event(&event) {
                    if let Err(e) = tx.blocking_send(change) {
                        warn!("Failed to send file change event: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Watch error: {}", e);
            }
        };

        let mut watcher =
            if let Some(poll_interval) = poll_interval_from_env() {
                let config = Config::default()
                    .with_poll_interval(poll_interval)
                    .with_compare_contents(true);
                WatcherImpl::Poll(PollWatcher::new(handler, config).map_err(|e| {
                    VfsError::Watch(format!("Failed to create poll watcher: {}", e))
                })?)
            } else {
                let config = Config::default();
                WatcherImpl::Recommended(
                    RecommendedWatcher::new(handler, config)
                        .map_err(|e| VfsError::Watch(format!("Failed to create watcher: {}", e)))?,
                )
            };

        watcher
            .watch(fs_root, RecursiveMode::Recursive)
            .map_err(|e| {
                VfsError::Watch(format!(
                    "Failed to watch path '{}': {}",
                    fs_root.display(),
                    e
                ))
            })?;

        debug!("Watching path: {}", fs_root.display());
        self.watcher = Some(watcher);

        Ok(())
    }

    /// Take the event receiver. Can only be called once.
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<FileChange>> {
        self.rx.take()
    }
}

fn poll_interval_from_env() -> Option<Duration> {
    let value = std::env::var("OPENFS_WATCH_POLL_INTERVAL_MS").ok()?;
    let millis: u64 = value.parse().ok()?;
    if millis == 0 {
        return None;
    }
    Some(Duration::from_millis(millis))
}

/// Convert a notify event to a FileChange.
fn convert_event(event: &Event) -> Option<FileChange> {
    let path = event.paths.first()?.clone();
    let kind = match event.kind {
        EventKind::Create(_) => ChangeKind::Created,
        EventKind::Modify(_) => ChangeKind::Modified,
        EventKind::Remove(_) => ChangeKind::Deleted,
        EventKind::Other => return None,
        EventKind::Access(_) => return None,
        EventKind::Any => return None,
    };

    Some(FileChange {
        path,
        kind,
        timestamp: SystemTime::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;
    use tempfile::TempDir;
    use tokio::time::{timeout, Duration};

    fn ensure_polling() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            std::env::set_var("OPENFS_WATCH_POLL_INTERVAL_MS", "50");
        });
    }

    fn normalize_test_path(path: &Path) -> PathBuf {
        let path_str = path.to_string_lossy();
        if let Some(stripped) = path_str.strip_prefix("/private") {
            return PathBuf::from(stripped);
        }
        path.to_path_buf()
    }

    fn paths_equivalent(change: &Path, expected: &Path) -> bool {
        if change == expected {
            return true;
        }
        if change.file_name() != expected.file_name() {
            return false;
        }
        normalize_test_path(change) == normalize_test_path(expected)
    }

    #[tokio::test]
    async fn test_watch_file_create() {
        ensure_polling();
        let temp_dir = TempDir::new().unwrap();
        // Canonicalize to handle macOS /var -> /private/var symlink
        let canonical_dir = temp_dir.path().canonicalize().unwrap();
        let mut engine = WatchEngine::new().unwrap();
        engine.watch_path(&canonical_dir).unwrap();
        let mut rx = engine.take_receiver().unwrap();

        // Allow the watcher time to register before creating files.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Create a file
        let file_path = canonical_dir.join("test.txt");
        tokio::fs::write(&file_path, "hello").await.unwrap();

        // Drain events until we find one for our file
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut found = false;
        while tokio::time::Instant::now() < deadline {
            match timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(change)) => {
                    if paths_equivalent(&change.path, &file_path) {
                        assert!(
                            change.kind == ChangeKind::Created
                                || change.kind == ChangeKind::Modified,
                            "expected Created or Modified, got {:?}",
                            change.kind
                        );
                        found = true;
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }
        assert!(found, "expected an event for {:?}", file_path);
    }
}

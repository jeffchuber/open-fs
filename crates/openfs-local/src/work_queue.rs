//! SQLite-backed persistent work queue for indexing operations.
//!
//! Inspired by chroma-fs Sentinel's work queue:
//! - Upsert semantics (latest event per path wins -- natural debounce)
//! - Configurable debounce window before processing
//! - Retry with exponential backoff
//! - Dead letter queue for permanently failed items
//! - WAL mode for write performance
//! - Survives process crashes

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use tracing::{debug, warn};

/// Status of a work queue item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueItemStatus {
    /// Pending processing.
    Pending,
    /// Currently being processed.
    Processing,
    /// Failed permanently -- in dead letter queue.
    DeadLetter,
}

impl QueueItemStatus {
    #[cfg(test)]
    fn as_str(&self) -> &'static str {
        match self {
            QueueItemStatus::Pending => "pending",
            QueueItemStatus::Processing => "processing",
            QueueItemStatus::DeadLetter => "dead_letter",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "processing" => QueueItemStatus::Processing,
            "dead_letter" => QueueItemStatus::DeadLetter,
            _ => QueueItemStatus::Pending,
        }
    }
}

/// Type of work queue event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueEventType {
    /// File changed -- needs (re-)indexing.
    Changed,
    /// File deleted -- remove from index.
    Deleted,
}

impl QueueEventType {
    fn as_str(&self) -> &'static str {
        match self {
            QueueEventType::Changed => "changed",
            QueueEventType::Deleted => "deleted",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "deleted" => QueueEventType::Deleted,
            _ => QueueEventType::Changed,
        }
    }
}

/// A single item in the work queue.
#[derive(Debug, Clone)]
pub struct QueueItem {
    pub id: i64,
    pub path: String,
    pub event_type: QueueEventType,
    pub status: QueueItemStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Earliest time this item should be processed (for debounce/backoff).
    pub process_after: u64,
}

/// Configuration for the work queue.
#[derive(Debug, Clone)]
pub struct WorkQueueConfig {
    /// Debounce window -- items won't be processed until this many seconds after last upsert.
    pub debounce_secs: u64,
    /// Maximum retry attempts before moving to dead letter.
    pub max_retries: u32,
    /// Base backoff in seconds (doubles each retry: 2, 4, 8, ...).
    pub base_backoff_secs: u64,
}

impl Default for WorkQueueConfig {
    fn default() -> Self {
        WorkQueueConfig {
            debounce_secs: 1,
            max_retries: 3,
            base_backoff_secs: 2,
        }
    }
}

/// SQLite-backed persistent work queue.
pub struct WorkQueue {
    conn: Connection,
    config: WorkQueueConfig,
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl WorkQueue {
    /// Open (or create) a persistent work queue at the given path.
    pub fn open(db_path: &Path, config: WorkQueueConfig) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open work queue DB: {}", e))?;

        // Enable WAL mode for concurrent reads + write performance
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS work_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                event_type TEXT NOT NULL DEFAULT 'changed',
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                process_after INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_wq_status_process
                ON work_queue(status, process_after);
            CREATE TABLE IF NOT EXISTS dead_letter (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                event_type TEXT NOT NULL,
                attempts INTEGER NOT NULL,
                last_error TEXT,
                created_at INTEGER NOT NULL,
                dead_at INTEGER NOT NULL
            );",
        )
        .map_err(|e| format!("Failed to create tables: {}", e))?;

        Ok(WorkQueue { conn, config })
    }

    /// Open an in-memory work queue (for testing).
    pub fn open_memory(config: WorkQueueConfig) -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory DB: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS work_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                event_type TEXT NOT NULL DEFAULT 'changed',
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                process_after INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_wq_status_process
                ON work_queue(status, process_after);
            CREATE TABLE IF NOT EXISTS dead_letter (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                event_type TEXT NOT NULL,
                attempts INTEGER NOT NULL,
                last_error TEXT,
                created_at INTEGER NOT NULL,
                dead_at INTEGER NOT NULL
            );",
        )
        .map_err(|e| format!("Failed to create tables: {}", e))?;

        Ok(WorkQueue { conn, config })
    }

    /// Enqueue or update an event for a path (upsert semantics).
    ///
    /// If the path already exists in the queue, updates the event type and resets
    /// the debounce window. This provides natural deduplication.
    pub fn enqueue(&self, path: &str, event_type: QueueEventType) -> Result<(), String> {
        let now = now_epoch();
        let process_after = now + self.config.debounce_secs;

        self.conn
            .execute(
                "INSERT INTO work_queue (path, event_type, status, attempts, created_at, updated_at, process_after)
                 VALUES (?1, ?2, 'pending', 0, ?3, ?3, ?4)
                 ON CONFLICT(path) DO UPDATE SET
                     event_type = ?2,
                     status = 'pending',
                     attempts = 0,
                     last_error = NULL,
                     updated_at = ?3,
                     process_after = ?4",
                params![path, event_type.as_str(), now, process_after],
            )
            .map_err(|e| format!("Failed to enqueue: {}", e))?;

        debug!("Enqueued {} for {}", event_type.as_str(), path);
        Ok(())
    }

    /// Fetch the next batch of items ready for processing.
    ///
    /// Returns items whose debounce window has elapsed and that are in pending status.
    /// Atomically marks them as "processing".
    pub fn fetch_ready(&self, batch_size: usize) -> Result<Vec<QueueItem>, String> {
        let now = now_epoch();

        // Select and mark as processing in one step
        let mut stmt = self.conn
            .prepare(
                "SELECT id, path, event_type, status, attempts, last_error, created_at, updated_at, process_after
                 FROM work_queue
                 WHERE status = 'pending' AND process_after <= ?1
                 ORDER BY process_after ASC
                 LIMIT ?2"
            )
            .map_err(|e| format!("Failed to prepare fetch: {}", e))?;

        let items: Vec<QueueItem> = stmt
            .query_map(params![now, batch_size as i64], |row| {
                Ok(QueueItem {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    event_type: QueueEventType::from_str(&row.get::<_, String>(2)?),
                    status: QueueItemStatus::from_str(&row.get::<_, String>(3)?),
                    attempts: row.get::<_, u32>(4)?,
                    last_error: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    process_after: row.get(8)?,
                })
            })
            .map_err(|e| format!("Failed to fetch: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        // Mark as processing
        for item in &items {
            self.conn
                .execute(
                    "UPDATE work_queue SET status = 'processing', updated_at = ?1 WHERE id = ?2",
                    params![now, item.id],
                )
                .map_err(|e| format!("Failed to mark processing: {}", e))?;
        }

        Ok(items)
    }

    /// Mark an item as successfully completed (removes from queue).
    pub fn complete(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM work_queue WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to complete: {}", e))?;
        Ok(())
    }

    /// Mark an item as failed, scheduling for retry or moving to dead letter.
    pub fn fail(&self, id: i64, error: &str) -> Result<(), String> {
        let now = now_epoch();

        // Get current attempt count
        let attempts: u32 = self
            .conn
            .query_row(
                "SELECT attempts FROM work_queue WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to get attempts: {}", e))?;

        let new_attempts = attempts + 1;

        if new_attempts >= self.config.max_retries {
            // Move to dead letter queue
            self.conn.execute(
                "INSERT INTO dead_letter (path, event_type, attempts, last_error, created_at, dead_at)
                 SELECT path, event_type, ?1, ?2, created_at, ?3 FROM work_queue WHERE id = ?4",
                params![new_attempts, error, now, id],
            ).map_err(|e| format!("Failed to dead-letter: {}", e))?;

            self.conn
                .execute("DELETE FROM work_queue WHERE id = ?1", params![id])
                .map_err(|e| format!("Failed to remove after dead-letter: {}", e))?;

            warn!(
                "Moved item {} to dead letter after {} attempts",
                id, new_attempts
            );
        } else {
            // Schedule retry with exponential backoff
            let backoff = self.config.base_backoff_secs * 2u64.pow(attempts);
            let process_after = now + backoff;

            self.conn.execute(
                "UPDATE work_queue SET status = 'pending', attempts = ?1, last_error = ?2, updated_at = ?3, process_after = ?4 WHERE id = ?5",
                params![new_attempts, error, now, process_after, id],
            ).map_err(|e| format!("Failed to schedule retry: {}", e))?;

            debug!(
                "Scheduled retry for item {} in {}s (attempt {})",
                id, backoff, new_attempts
            );
        }

        Ok(())
    }

    /// Get the number of pending items.
    pub fn pending_count(&self) -> Result<usize, String> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM work_queue WHERE status = 'pending'",
                [],
                |row| row.get::<_, usize>(0),
            )
            .map_err(|e| format!("Failed to count pending: {}", e))
    }

    /// Get the number of items in the dead letter queue.
    pub fn dead_letter_count(&self) -> Result<usize, String> {
        self.conn
            .query_row("SELECT COUNT(*) FROM dead_letter", [], |row| {
                row.get::<_, usize>(0)
            })
            .map_err(|e| format!("Failed to count dead letters: {}", e))
    }

    /// Get all dead letter items.
    pub fn dead_letters(&self) -> Result<Vec<(String, String, u32, Option<String>)>, String> {
        let mut stmt = self.conn
            .prepare("SELECT path, event_type, attempts, last_error FROM dead_letter ORDER BY dead_at DESC")
            .map_err(|e| format!("Failed to prepare dead letter query: {}", e))?;

        let items = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| format!("Failed to query dead letters: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(items)
    }

    /// Clear the dead letter queue.
    pub fn clear_dead_letters(&self) -> Result<usize, String> {
        let count = self
            .conn
            .execute("DELETE FROM dead_letter", [])
            .map_err(|e| format!("Failed to clear dead letters: {}", e))?;
        Ok(count)
    }

    /// Recover items stuck in "processing" state (e.g., after a crash).
    /// Resets them to "pending" so they get retried.
    pub fn recover_stuck(&self) -> Result<usize, String> {
        let now = now_epoch();
        let count = self.conn
            .execute(
                "UPDATE work_queue SET status = 'pending', updated_at = ?1, process_after = ?1 WHERE status = 'processing'",
                params![now],
            )
            .map_err(|e| format!("Failed to recover stuck items: {}", e))?;

        if count > 0 {
            debug!("Recovered {} stuck items", count);
        }
        Ok(count)
    }

    /// Get total queue size (pending + processing).
    pub fn queue_size(&self) -> Result<usize, String> {
        self.conn
            .query_row("SELECT COUNT(*) FROM work_queue", [], |row| {
                row.get::<_, usize>(0)
            })
            .map_err(|e| format!("Failed to count queue: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_queue() -> WorkQueue {
        WorkQueue::open_memory(WorkQueueConfig {
            debounce_secs: 0, // No debounce for tests
            max_retries: 3,
            base_backoff_secs: 1,
        })
        .unwrap()
    }

    #[test]
    fn test_enqueue_and_fetch() {
        let q = make_queue();
        q.enqueue("/file1.txt", QueueEventType::Changed).unwrap();
        q.enqueue("/file2.txt", QueueEventType::Deleted).unwrap();

        assert_eq!(q.pending_count().unwrap(), 2);

        let items = q.fetch_ready(10).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "/file1.txt");
        assert_eq!(items[0].event_type, QueueEventType::Changed);
        assert_eq!(items[1].path, "/file2.txt");
        assert_eq!(items[1].event_type, QueueEventType::Deleted);
    }

    #[test]
    fn test_upsert_deduplication() {
        let q = make_queue();
        q.enqueue("/file.txt", QueueEventType::Changed).unwrap();
        q.enqueue("/file.txt", QueueEventType::Changed).unwrap();
        q.enqueue("/file.txt", QueueEventType::Deleted).unwrap();

        // Only one item should exist, with latest event type
        assert_eq!(q.queue_size().unwrap(), 1);
        let items = q.fetch_ready(10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].event_type, QueueEventType::Deleted);
    }

    #[test]
    fn test_complete() {
        let q = make_queue();
        q.enqueue("/file.txt", QueueEventType::Changed).unwrap();

        let items = q.fetch_ready(10).unwrap();
        assert_eq!(items.len(), 1);

        q.complete(items[0].id).unwrap();
        assert_eq!(q.queue_size().unwrap(), 0);
    }

    #[test]
    fn test_retry_on_failure() {
        let q = make_queue();
        q.enqueue("/file.txt", QueueEventType::Changed).unwrap();

        let items = q.fetch_ready(10).unwrap();
        q.fail(items[0].id, "transient error").unwrap();

        // Item should still be in queue with incremented attempts
        assert_eq!(q.queue_size().unwrap(), 1);
        assert_eq!(q.dead_letter_count().unwrap(), 0);
    }

    #[test]
    fn test_queue_item_status_roundtrip() {
        assert_eq!(
            QueueItemStatus::from_str(QueueItemStatus::Pending.as_str()),
            QueueItemStatus::Pending
        );
        assert_eq!(
            QueueItemStatus::from_str(QueueItemStatus::Processing.as_str()),
            QueueItemStatus::Processing
        );
        assert_eq!(
            QueueItemStatus::from_str(QueueItemStatus::DeadLetter.as_str()),
            QueueItemStatus::DeadLetter
        );
    }

    #[test]
    fn test_event_type_roundtrip() {
        assert_eq!(
            QueueEventType::from_str(QueueEventType::Changed.as_str()),
            QueueEventType::Changed
        );
        assert_eq!(
            QueueEventType::from_str(QueueEventType::Deleted.as_str()),
            QueueEventType::Deleted
        );
    }
}

//! Write-Ahead Log (WAL) and durable outbox for crash-safe sync.
//!
//! All write operations are logged to a SQLite WAL before being applied.
//! Pending remote sync operations are stored in a durable outbox that
//! survives process crashes. On startup, the outbox is replayed to
//! ensure no operations are lost.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use tracing::{debug, warn};

/// Type of operation in the WAL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalOpType {
    Write,
    Delete,
    Append,
}

impl WalOpType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WalOpType::Write => "write",
            WalOpType::Delete => "delete",
            WalOpType::Append => "append",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "delete" => WalOpType::Delete,
            "append" => WalOpType::Append,
            _ => WalOpType::Write,
        }
    }
}

/// Status of an outbox entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatus {
    /// Pending sync to remote.
    Pending,
    /// Currently being processed.
    Processing,
    /// Permanently failed.
    Failed,
}

impl OutboxStatus {
    #[cfg(test)]
    fn as_str(&self) -> &'static str {
        match self {
            OutboxStatus::Pending => "pending",
            OutboxStatus::Processing => "processing",
            OutboxStatus::Failed => "failed",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "processing" => OutboxStatus::Processing,
            "failed" => OutboxStatus::Failed,
            _ => OutboxStatus::Pending,
        }
    }
}

/// A WAL entry representing a logged operation.
#[derive(Debug, Clone)]
pub struct WalEntry {
    pub id: i64,
    pub op_type: WalOpType,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub mount_path: String,
    pub timestamp: i64,
    pub applied: bool,
}

/// An outbox entry representing a pending remote sync operation.
#[derive(Debug, Clone)]
pub struct OutboxEntry {
    pub id: i64,
    pub op_type: WalOpType,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub mount_path: String,
    pub status: OutboxStatus,
    pub attempts: u32,
    pub created_at: i64,
    pub last_attempt: Option<i64>,
    pub error: Option<String>,
}

/// Per-mount sync profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum SyncProfile {
    /// No syncing - local only.
    LocalOnly,
    /// Writes applied locally first, synced to remote in background.
    #[default]
    LocalFirst,
    /// Writes sent to remote first, then cached locally.
    RemoteFirst,
    /// Writes only go to remote, no local state.
    RemoteOnly,
}


impl SyncProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncProfile::LocalOnly => "local_only",
            SyncProfile::LocalFirst => "local_first",
            SyncProfile::RemoteFirst => "remote_first",
            SyncProfile::RemoteOnly => "remote_only",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "local_only" => SyncProfile::LocalOnly,
            "remote_first" => SyncProfile::RemoteFirst,
            "remote_only" => SyncProfile::RemoteOnly,
            _ => SyncProfile::LocalFirst,
        }
    }
}

/// Configuration for the WAL engine.
#[derive(Debug, Clone)]
pub struct WalConfig {
    /// Maximum retry attempts for outbox entries.
    pub max_retries: u32,
    /// Base backoff in seconds for retries.
    pub base_backoff_secs: u64,
    /// Whether to auto-recover stuck entries on startup.
    pub recover_on_startup: bool,
    /// Maximum age (seconds) for a processing entry before it's considered stuck.
    pub stuck_timeout_secs: i64,
    /// Auto-checkpoint after this many applied entries (0 = disabled).
    pub auto_checkpoint_threshold: usize,
    /// Max age (seconds) of applied WAL entries to keep during checkpoint.
    pub checkpoint_max_age_secs: i64,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_backoff_secs: 2,
            recover_on_startup: true,
            stuck_timeout_secs: 300,
            auto_checkpoint_threshold: 500,
            checkpoint_max_age_secs: 86400, // 24 hours
        }
    }
}

/// Write-Ahead Log backed by SQLite.
///
/// Provides crash-safe write operations by logging all operations before
/// applying them, and a durable outbox for pending remote sync operations.
pub struct WriteAheadLog {
    conn: Mutex<Connection>,
    config: WalConfig,
}

// Safety: WriteAheadLog is used behind a Mutex, so only one thread
// accesses the Connection at a time.
unsafe impl Send for WriteAheadLog {}
unsafe impl Sync for WriteAheadLog {}

impl WriteAheadLog {
    /// Create a new WAL at the given path.
    pub fn new(path: &Path, config: WalConfig) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("Failed to open WAL db: {}", e))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("Failed to set WAL mode: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS wal_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                op_type TEXT NOT NULL,
                path TEXT NOT NULL,
                content BLOB,
                mount_path TEXT NOT NULL DEFAULT '',
                timestamp INTEGER NOT NULL,
                applied INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS outbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                op_type TEXT NOT NULL,
                path TEXT NOT NULL,
                content BLOB,
                mount_path TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                last_attempt INTEGER,
                error TEXT
            );

            CREATE TABLE IF NOT EXISTS sync_profiles (
                mount_path TEXT PRIMARY KEY,
                profile TEXT NOT NULL DEFAULT 'local_first'
            );

            CREATE INDEX IF NOT EXISTS idx_wal_applied ON wal_log(applied);
            CREATE INDEX IF NOT EXISTS idx_outbox_status ON outbox(status);
            CREATE INDEX IF NOT EXISTS idx_outbox_path ON outbox(path);",
        )
        .map_err(|e| format!("Failed to create WAL tables: {}", e))?;

        let wal = Self {
            conn: Mutex::new(conn),
            config,
        };

        if wal.config.recover_on_startup {
            wal.recover_stuck()?;
        }

        Ok(wal)
    }

    /// Create an in-memory WAL (for testing).
    pub fn in_memory(config: WalConfig) -> Result<Self, String> {
        let conn =
            Connection::open_in_memory().map_err(|e| format!("Failed to open in-memory db: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS wal_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                op_type TEXT NOT NULL,
                path TEXT NOT NULL,
                content BLOB,
                mount_path TEXT NOT NULL DEFAULT '',
                timestamp INTEGER NOT NULL,
                applied INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS outbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                op_type TEXT NOT NULL,
                path TEXT NOT NULL,
                content BLOB,
                mount_path TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                last_attempt INTEGER,
                error TEXT
            );

            CREATE TABLE IF NOT EXISTS sync_profiles (
                mount_path TEXT PRIMARY KEY,
                profile TEXT NOT NULL DEFAULT 'local_first'
            );

            CREATE INDEX IF NOT EXISTS idx_wal_applied ON wal_log(applied);
            CREATE INDEX IF NOT EXISTS idx_outbox_status ON outbox(status);
            CREATE INDEX IF NOT EXISTS idx_outbox_path ON outbox(path);",
        )
        .map_err(|e| format!("Failed to create WAL tables: {}", e))?;

        Ok(Self {
            conn: Mutex::new(conn),
            config,
        })
    }

    /// Log a write operation to the WAL before it's applied.
    pub fn log_write(
        &self,
        op_type: WalOpType,
        path: &str,
        content: Option<&[u8]>,
        mount_path: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let now = now_unix();

        conn.execute(
            "INSERT INTO wal_log (op_type, path, content, mount_path, timestamp, applied)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![op_type.as_str(), path, content, mount_path, now],
        )
        .map_err(|e| format!("Failed to log WAL entry: {}", e))?;

        let id = conn.last_insert_rowid();
        debug!("WAL logged: id={} op={} path={}", id, op_type.as_str(), path);
        Ok(id)
    }

    /// Mark a WAL entry as applied.
    pub fn mark_applied(&self, wal_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wal_log SET applied = 1 WHERE id = ?1",
            params![wal_id],
        )
        .map_err(|e| format!("Failed to mark WAL entry applied: {}", e))?;

        // Auto-checkpoint if threshold is set
        if self.config.auto_checkpoint_threshold > 0 {
            let applied_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM wal_log WHERE applied = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if applied_count as usize >= self.config.auto_checkpoint_threshold {
                let cutoff = now_unix().saturating_sub(self.config.checkpoint_max_age_secs);
                let pruned = conn
                    .execute(
                        "DELETE FROM wal_log WHERE applied = 1 AND timestamp < ?1",
                        params![cutoff],
                    )
                    .unwrap_or(0);
                if pruned > 0 {
                    debug!("WAL auto-checkpoint: pruned {} old applied entries", pruned);
                }
            }
        }

        Ok(())
    }

    /// Run a manual checkpoint: prune all applied WAL entries older than
    /// `checkpoint_max_age_secs` and VACUUM the database.
    pub fn checkpoint(&self) -> Result<usize, String> {
        let pruned = self.prune_wal(self.config.checkpoint_max_age_secs)?;
        if pruned > 0 {
            let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(|e| format!("Failed to checkpoint WAL: {}", e))?;
            debug!("WAL checkpoint: pruned {} entries and truncated WAL", pruned);
        }
        Ok(pruned)
    }

    /// Get unapplied WAL entries (for crash recovery).
    pub fn get_unapplied(&self) -> Result<Vec<WalEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, op_type, path, content, mount_path, timestamp, applied
                 FROM wal_log WHERE applied = 0 ORDER BY id ASC",
            )
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let entries = stmt
            .query_map([], |row| {
                Ok(WalEntry {
                    id: row.get(0)?,
                    op_type: WalOpType::from_str(&row.get::<_, String>(1)?),
                    path: row.get(2)?,
                    content: row.get(3)?,
                    mount_path: row.get(4)?,
                    timestamp: row.get(5)?,
                    applied: row.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(|e| format!("Failed to query unapplied: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// Add an entry to the outbox for remote sync.
    pub fn enqueue_outbox(
        &self,
        op_type: WalOpType,
        path: &str,
        content: Option<&[u8]>,
        mount_path: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let now = now_unix();

        // Upsert: if there's already a pending entry for this path+mount, update it
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM outbox WHERE path = ?1 AND mount_path = ?2 AND status = 'pending'",
                params![path, mount_path],
                |row| row.get(0),
            )
            .ok();

        if let Some(existing_id) = existing {
            conn.execute(
                "UPDATE outbox SET op_type = ?1, content = ?2, created_at = ?3
                 WHERE id = ?4",
                params![op_type.as_str(), content, now, existing_id],
            )
            .map_err(|e| format!("Failed to update outbox entry: {}", e))?;
            debug!("Outbox updated: id={} path={}", existing_id, path);
            Ok(existing_id)
        } else {
            conn.execute(
                "INSERT INTO outbox (op_type, path, content, mount_path, status, attempts, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5)",
                params![op_type.as_str(), path, content, mount_path, now],
            )
            .map_err(|e| format!("Failed to insert outbox entry: {}", e))?;
            let id = conn.last_insert_rowid();
            debug!("Outbox enqueued: id={} path={}", id, path);
            Ok(id)
        }
    }

    /// Fetch ready outbox entries for processing.
    pub fn fetch_ready_outbox(&self, limit: usize) -> Result<Vec<OutboxEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let now = now_unix();

        let mut stmt = conn
            .prepare(
                "SELECT id, op_type, path, content, mount_path, status, attempts,
                        created_at, last_attempt, error
                 FROM outbox
                 WHERE status = 'pending'
                   AND (last_attempt IS NULL
                        OR last_attempt + (?1 * (1 << MIN(attempts, 10))) < ?2)
                 ORDER BY created_at ASC
                 LIMIT ?3",
            )
            .map_err(|e| format!("Failed to prepare outbox query: {}", e))?;

        let entries = stmt
            .query_map(
                params![self.config.base_backoff_secs as i64, now, limit as i64],
                |row| {
                    Ok(OutboxEntry {
                        id: row.get(0)?,
                        op_type: WalOpType::from_str(&row.get::<_, String>(1)?),
                        path: row.get(2)?,
                        content: row.get(3)?,
                        mount_path: row.get(4)?,
                        status: OutboxStatus::from_str(&row.get::<_, String>(5)?),
                        attempts: row.get::<_, u32>(6)?,
                        created_at: row.get(7)?,
                        last_attempt: row.get(8)?,
                        error: row.get(9)?,
                    })
                },
            )
            .map_err(|e| format!("Failed to query outbox: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// Mark an outbox entry as being processed.
    pub fn mark_processing(&self, entry_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE outbox SET status = 'processing', last_attempt = ?1 WHERE id = ?2",
            params![now_unix(), entry_id],
        )
        .map_err(|e| format!("Failed to mark processing: {}", e))?;
        Ok(())
    }

    /// Mark an outbox entry as successfully synced (removes it).
    pub fn complete_outbox(&self, entry_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute("DELETE FROM outbox WHERE id = ?1", params![entry_id])
            .map_err(|e| format!("Failed to complete outbox entry: {}", e))?;
        debug!("Outbox completed: id={}", entry_id);
        Ok(())
    }

    /// Record a failure for an outbox entry. Moves to failed if max retries exceeded.
    pub fn fail_outbox(&self, entry_id: i64, error: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let now = now_unix();

        // Get current attempts
        let attempts: u32 = conn
            .query_row(
                "SELECT attempts FROM outbox WHERE id = ?1",
                params![entry_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to get attempts: {}", e))?;

        let new_attempts = attempts + 1;
        if new_attempts >= self.config.max_retries {
            conn.execute(
                "UPDATE outbox SET status = 'failed', attempts = ?1, last_attempt = ?2, error = ?3
                 WHERE id = ?4",
                params![new_attempts, now, error, entry_id],
            )
            .map_err(|e| format!("Failed to mark outbox failed: {}", e))?;
            warn!("Outbox entry {} moved to failed after {} attempts: {}", entry_id, new_attempts, error);
        } else {
            conn.execute(
                "UPDATE outbox SET status = 'pending', attempts = ?1, last_attempt = ?2, error = ?3
                 WHERE id = ?4",
                params![new_attempts, now, error, entry_id],
            )
            .map_err(|e| format!("Failed to update outbox retry: {}", e))?;
            debug!("Outbox entry {} retry {}/{}: {}", entry_id, new_attempts, self.config.max_retries, error);
        }

        Ok(())
    }

    /// Recover stuck processing entries (from a previous crash).
    pub fn recover_stuck(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let cutoff = now_unix() - self.config.stuck_timeout_secs;

        let count = conn
            .execute(
                "UPDATE outbox SET status = 'pending'
                 WHERE status = 'processing'
                   AND (last_attempt IS NULL OR last_attempt <= ?1)",
                params![cutoff],
            )
            .map_err(|e| format!("Failed to recover stuck: {}", e))?;

        if count > 0 {
            warn!("Recovered {} stuck outbox entries", count);
        }
        Ok(count)
    }

    /// Get count of outbox entries by status.
    pub fn outbox_stats(&self) -> Result<OutboxStats, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;

        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count pending: {}", e))?;

        let processing: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE status = 'processing'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count processing: {}", e))?;

        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE status = 'failed'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count failed: {}", e))?;

        let wal_unapplied: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wal_log WHERE applied = 0",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count unapplied: {}", e))?;

        Ok(OutboxStats {
            pending: pending as usize,
            processing: processing as usize,
            failed: failed as usize,
            wal_unapplied: wal_unapplied as usize,
        })
    }

    /// Set the sync profile for a mount.
    pub fn set_sync_profile(&self, mount_path: &str, profile: SyncProfile) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO sync_profiles (mount_path, profile) VALUES (?1, ?2)",
            params![mount_path, profile.as_str()],
        )
        .map_err(|e| format!("Failed to set sync profile: {}", e))?;
        Ok(())
    }

    /// Get the sync profile for a mount.
    pub fn get_sync_profile(&self, mount_path: &str) -> Result<SyncProfile, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let profile: Option<String> = conn
            .query_row(
                "SELECT profile FROM sync_profiles WHERE mount_path = ?1",
                params![mount_path],
                |row| row.get(0),
            )
            .ok();

        Ok(profile
            .map(|p| SyncProfile::from_str(&p))
            .unwrap_or_default())
    }

    /// Get failed outbox entries (dead letter queue).
    pub fn get_failed(&self) -> Result<Vec<OutboxEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, op_type, path, content, mount_path, status, attempts,
                        created_at, last_attempt, error
                 FROM outbox WHERE status = 'failed' ORDER BY created_at ASC",
            )
            .map_err(|e| format!("Failed to prepare failed query: {}", e))?;

        let entries = stmt
            .query_map([], |row| {
                Ok(OutboxEntry {
                    id: row.get(0)?,
                    op_type: WalOpType::from_str(&row.get::<_, String>(1)?),
                    path: row.get(2)?,
                    content: row.get(3)?,
                    mount_path: row.get(4)?,
                    status: OutboxStatus::from_str(&row.get::<_, String>(5)?),
                    attempts: row.get::<_, u32>(6)?,
                    created_at: row.get(7)?,
                    last_attempt: row.get(8)?,
                    error: row.get(9)?,
                })
            })
            .map_err(|e| format!("Failed to query failed: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// Retry a failed outbox entry (move back to pending).
    pub fn retry_failed(&self, entry_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE outbox SET status = 'pending', attempts = 0, error = NULL WHERE id = ?1 AND status = 'failed'",
            params![entry_id],
        )
        .map_err(|e| format!("Failed to retry failed entry: {}", e))?;
        Ok(())
    }

    /// Prune applied WAL entries older than the given age (seconds).
    pub fn prune_wal(&self, max_age_secs: i64) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let cutoff = now_unix().saturating_sub(max_age_secs);
        let count = conn
            .execute(
                "DELETE FROM wal_log WHERE applied = 1 AND timestamp < ?1",
                params![cutoff],
            )
            .map_err(|e| format!("Failed to prune WAL: {}", e))?;
        Ok(count)
    }
}

/// Statistics for the outbox.
#[derive(Debug, Clone, Default)]
pub struct OutboxStats {
    pub pending: usize,
    pub processing: usize,
    pub failed: usize,
    pub wal_unapplied: usize,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wal() -> WriteAheadLog {
        WriteAheadLog::in_memory(WalConfig {
            recover_on_startup: false,
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn test_wal_log_and_apply() {
        let wal = make_wal();

        let id = wal
            .log_write(WalOpType::Write, "/test.txt", Some(b"hello"), "/")
            .unwrap();
        assert!(id > 0);

        let unapplied = wal.get_unapplied().unwrap();
        assert_eq!(unapplied.len(), 1);
        assert_eq!(unapplied[0].path, "/test.txt");
        assert_eq!(unapplied[0].content, Some(b"hello".to_vec()));

        wal.mark_applied(id).unwrap();
        let unapplied = wal.get_unapplied().unwrap();
        assert_eq!(unapplied.len(), 0);
    }

    #[test]
    fn test_outbox_enqueue_and_fetch() {
        let wal = make_wal();

        let id = wal
            .enqueue_outbox(WalOpType::Write, "/test.txt", Some(b"data"), "/mnt")
            .unwrap();
        assert!(id > 0);

        let ready = wal.fetch_ready_outbox(10).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].path, "/test.txt");
        assert_eq!(ready[0].mount_path, "/mnt");
        assert_eq!(ready[0].status, OutboxStatus::Pending);
    }

    #[test]
    fn test_outbox_upsert_semantics() {
        let wal = make_wal();

        let id1 = wal
            .enqueue_outbox(WalOpType::Write, "/test.txt", Some(b"v1"), "/")
            .unwrap();
        let id2 = wal
            .enqueue_outbox(WalOpType::Write, "/test.txt", Some(b"v2"), "/")
            .unwrap();

        assert_eq!(id1, id2);

        let ready = wal.fetch_ready_outbox(10).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].content, Some(b"v2".to_vec()));
    }

    #[test]
    fn test_outbox_complete() {
        let wal = make_wal();

        let id = wal
            .enqueue_outbox(WalOpType::Write, "/test.txt", Some(b"data"), "/")
            .unwrap();

        wal.mark_processing(id).unwrap();
        wal.complete_outbox(id).unwrap();

        let ready = wal.fetch_ready_outbox(10).unwrap();
        assert_eq!(ready.len(), 0);
    }

    #[test]
    fn test_wal_op_type_roundtrip() {
        assert_eq!(WalOpType::from_str(WalOpType::Write.as_str()), WalOpType::Write);
        assert_eq!(WalOpType::from_str(WalOpType::Delete.as_str()), WalOpType::Delete);
        assert_eq!(WalOpType::from_str(WalOpType::Append.as_str()), WalOpType::Append);
    }

    #[test]
    fn test_outbox_status_roundtrip() {
        assert_eq!(OutboxStatus::from_str(OutboxStatus::Pending.as_str()), OutboxStatus::Pending);
        assert_eq!(OutboxStatus::from_str(OutboxStatus::Processing.as_str()), OutboxStatus::Processing);
        assert_eq!(OutboxStatus::from_str(OutboxStatus::Failed.as_str()), OutboxStatus::Failed);
    }

    #[test]
    fn test_sync_profile_roundtrip() {
        assert_eq!(SyncProfile::from_str(SyncProfile::LocalOnly.as_str()), SyncProfile::LocalOnly);
        assert_eq!(SyncProfile::from_str(SyncProfile::LocalFirst.as_str()), SyncProfile::LocalFirst);
        assert_eq!(SyncProfile::from_str(SyncProfile::RemoteFirst.as_str()), SyncProfile::RemoteFirst);
        assert_eq!(SyncProfile::from_str(SyncProfile::RemoteOnly.as_str()), SyncProfile::RemoteOnly);
    }
}

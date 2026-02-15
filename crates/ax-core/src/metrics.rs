use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::RwLock;

/// Metrics for VFS operations.
#[derive(Debug, Default)]
pub struct VfsMetrics {
    // Read operations
    pub reads: AtomicU64,
    pub read_bytes: AtomicU64,
    pub read_errors: AtomicU64,

    // Write operations
    pub writes: AtomicU64,
    pub write_bytes: AtomicU64,
    pub write_errors: AtomicU64,

    // Delete operations
    pub deletes: AtomicU64,
    pub delete_errors: AtomicU64,

    // List operations
    pub lists: AtomicU64,
    pub list_errors: AtomicU64,

    // Latency tracking (using RwLock for histogram-like data)
    latencies: RwLock<LatencyTracker>,
}

/// Tracks operation latencies.
#[derive(Debug, Default)]
struct LatencyTracker {
    read_latencies: Vec<Duration>,
    write_latencies: Vec<Duration>,
    max_samples: usize,
}

impl LatencyTracker {
    fn new(max_samples: usize) -> Self {
        LatencyTracker {
            read_latencies: Vec::with_capacity(max_samples),
            write_latencies: Vec::with_capacity(max_samples),
            max_samples,
        }
    }

    fn record_read(&mut self, duration: Duration) {
        if self.read_latencies.len() >= self.max_samples {
            self.read_latencies.remove(0);
        }
        self.read_latencies.push(duration);
    }

    fn record_write(&mut self, duration: Duration) {
        if self.write_latencies.len() >= self.max_samples {
            self.write_latencies.remove(0);
        }
        self.write_latencies.push(duration);
    }

    fn read_avg(&self) -> Option<Duration> {
        if self.read_latencies.is_empty() {
            None
        } else {
            let total: Duration = self.read_latencies.iter().sum();
            Some(total / self.read_latencies.len() as u32)
        }
    }

    fn write_avg(&self) -> Option<Duration> {
        if self.write_latencies.is_empty() {
            None
        } else {
            let total: Duration = self.write_latencies.iter().sum();
            Some(total / self.write_latencies.len() as u32)
        }
    }

    fn read_p99(&self) -> Option<Duration> {
        percentile(&self.read_latencies, 99)
    }

    fn write_p99(&self) -> Option<Duration> {
        percentile(&self.write_latencies, 99)
    }
}

fn percentile(samples: &[Duration], p: usize) -> Option<Duration> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<_> = samples.to_vec();
    sorted.sort();
    let idx = (sorted.len() * p / 100).min(sorted.len() - 1);
    Some(sorted[idx])
}

impl VfsMetrics {
    /// Create a new metrics instance.
    pub fn new() -> Self {
        VfsMetrics {
            latencies: RwLock::new(LatencyTracker::new(1000)),
            ..Default::default()
        }
    }

    /// Record a successful read operation.
    pub fn record_read(&self, bytes: u64) {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.read_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record a read error.
    pub fn record_read_error(&self) {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.read_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record read latency.
    pub async fn record_read_latency(&self, duration: Duration) {
        self.latencies.write().await.record_read(duration);
    }

    /// Record a successful write operation.
    pub fn record_write(&self, bytes: u64) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.write_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record a write error.
    pub fn record_write_error(&self) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.write_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record write latency.
    pub async fn record_write_latency(&self, duration: Duration) {
        self.latencies.write().await.record_write(duration);
    }

    /// Record a successful delete operation.
    pub fn record_delete(&self) {
        self.deletes.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a delete error.
    pub fn record_delete_error(&self) {
        self.deletes.fetch_add(1, Ordering::Relaxed);
        self.delete_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a successful list operation.
    pub fn record_list(&self) {
        self.lists.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a list error.
    pub fn record_list_error(&self) {
        self.lists.fetch_add(1, Ordering::Relaxed);
        self.list_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot of the current metrics.
    pub async fn snapshot(&self) -> MetricsSnapshot {
        let latencies = self.latencies.read().await;

        MetricsSnapshot {
            reads: self.reads.load(Ordering::Relaxed),
            read_bytes: self.read_bytes.load(Ordering::Relaxed),
            read_errors: self.read_errors.load(Ordering::Relaxed),
            read_latency_avg_ms: latencies.read_avg().map(|d| d.as_millis() as f64),
            read_latency_p99_ms: latencies.read_p99().map(|d| d.as_millis() as f64),

            writes: self.writes.load(Ordering::Relaxed),
            write_bytes: self.write_bytes.load(Ordering::Relaxed),
            write_errors: self.write_errors.load(Ordering::Relaxed),
            write_latency_avg_ms: latencies.write_avg().map(|d| d.as_millis() as f64),
            write_latency_p99_ms: latencies.write_p99().map(|d| d.as_millis() as f64),

            deletes: self.deletes.load(Ordering::Relaxed),
            delete_errors: self.delete_errors.load(Ordering::Relaxed),

            lists: self.lists.load(Ordering::Relaxed),
            list_errors: self.list_errors.load(Ordering::Relaxed),
        }
    }

    /// Reset all metrics to zero.
    pub async fn reset(&self) {
        self.reads.store(0, Ordering::Relaxed);
        self.read_bytes.store(0, Ordering::Relaxed);
        self.read_errors.store(0, Ordering::Relaxed);
        self.writes.store(0, Ordering::Relaxed);
        self.write_bytes.store(0, Ordering::Relaxed);
        self.write_errors.store(0, Ordering::Relaxed);
        self.deletes.store(0, Ordering::Relaxed);
        self.delete_errors.store(0, Ordering::Relaxed);
        self.lists.store(0, Ordering::Relaxed);
        self.list_errors.store(0, Ordering::Relaxed);

        let mut latencies = self.latencies.write().await;
        latencies.read_latencies.clear();
        latencies.write_latencies.clear();
    }
}

/// A point-in-time snapshot of metrics.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub reads: u64,
    pub read_bytes: u64,
    pub read_errors: u64,
    pub read_latency_avg_ms: Option<f64>,
    pub read_latency_p99_ms: Option<f64>,

    pub writes: u64,
    pub write_bytes: u64,
    pub write_errors: u64,
    pub write_latency_avg_ms: Option<f64>,
    pub write_latency_p99_ms: Option<f64>,

    pub deletes: u64,
    pub delete_errors: u64,

    pub lists: u64,
    pub list_errors: u64,
}

impl MetricsSnapshot {
    /// Calculate read error rate as a percentage.
    pub fn read_error_rate(&self) -> f64 {
        if self.reads == 0 {
            0.0
        } else {
            (self.read_errors as f64 / self.reads as f64) * 100.0
        }
    }

    /// Calculate write error rate as a percentage.
    pub fn write_error_rate(&self) -> f64 {
        if self.writes == 0 {
            0.0
        } else {
            (self.write_errors as f64 / self.writes as f64) * 100.0
        }
    }

    /// Get total operations.
    pub fn total_operations(&self) -> u64 {
        self.reads + self.writes + self.deletes + self.lists
    }

    /// Get total errors.
    pub fn total_errors(&self) -> u64 {
        self.read_errors + self.write_errors + self.delete_errors + self.list_errors
    }
}

/// Shared metrics instance.
pub type SharedMetrics = Arc<VfsMetrics>;

/// Create a new shared metrics instance.
pub fn create_metrics() -> SharedMetrics {
    Arc::new(VfsMetrics::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_recording() {
        let metrics = VfsMetrics::new();

        metrics.record_read(100);
        metrics.record_read(200);
        metrics.record_write(50);
        metrics.record_read_error();

        let snapshot = metrics.snapshot().await;
        assert_eq!(snapshot.reads, 3); // 2 successful + 1 error
        assert_eq!(snapshot.read_bytes, 300);
        assert_eq!(snapshot.read_errors, 1);
        assert_eq!(snapshot.writes, 1);
        assert_eq!(snapshot.write_bytes, 50);
    }

    #[tokio::test]
    async fn test_metrics_latency() {
        let metrics = VfsMetrics::new();

        metrics
            .record_read_latency(Duration::from_millis(10))
            .await;
        metrics
            .record_read_latency(Duration::from_millis(20))
            .await;
        metrics
            .record_read_latency(Duration::from_millis(30))
            .await;

        let snapshot = metrics.snapshot().await;
        assert!(snapshot.read_latency_avg_ms.is_some());
        let avg = snapshot.read_latency_avg_ms.unwrap();
        assert!((avg - 20.0).abs() < 1.0);
    }

    #[tokio::test]
    async fn test_metrics_reset() {
        let metrics = VfsMetrics::new();

        metrics.record_read(100);
        metrics.record_write(50);

        metrics.reset().await;

        let snapshot = metrics.snapshot().await;
        assert_eq!(snapshot.reads, 0);
        assert_eq!(snapshot.writes, 0);
    }

    #[tokio::test]
    async fn test_error_rates() {
        let metrics = VfsMetrics::new();

        metrics.record_read(100);
        metrics.record_read(100);
        metrics.record_read(100);
        metrics.record_read(100);
        metrics.record_read_error(); // 1 error out of 5 total

        let snapshot = metrics.snapshot().await;
        assert_eq!(snapshot.read_error_rate(), 20.0);
    }
}

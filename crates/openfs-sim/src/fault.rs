use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use openfs_core::{Backend, BackendError, Entry};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use tokio::sync::Mutex;

/// Prefix used in error messages to distinguish injected faults from real errors.
pub const FAULT_PREFIX: &str = "[fault-injected]";

/// Configuration for fault injection.
#[derive(Debug, Clone)]
pub struct FaultConfig {
    /// Probability of injecting an error per operation (0.0-1.0).
    pub error_rate: f64,
    /// Probability of corrupting read data via bit flip (0.0-1.0).
    pub corruption_rate: f64,
}

impl Default for FaultConfig {
    fn default() -> Self {
        FaultConfig {
            error_rate: 0.0,
            corruption_rate: 0.0,
        }
    }
}

/// Statistics about injected faults.
#[derive(Debug, Clone)]
pub struct FaultStats {
    pub fault_count: usize,
    pub corruption_count: usize,
}

/// A backend wrapper that randomly injects errors and corrupts reads.
pub struct FaultyBackend {
    inner: Arc<dyn Backend>,
    rng: Mutex<ChaCha8Rng>,
    config: FaultConfig,
    fault_count: AtomicUsize,
    corruption_count: AtomicUsize,
}

impl FaultyBackend {
    pub fn new(inner: Arc<dyn Backend>, rng: ChaCha8Rng, config: FaultConfig) -> Self {
        FaultyBackend {
            inner,
            rng: Mutex::new(rng),
            config,
            fault_count: AtomicUsize::new(0),
            corruption_count: AtomicUsize::new(0),
        }
    }

    pub fn stats(&self) -> FaultStats {
        FaultStats {
            fault_count: self.fault_count.load(Ordering::Relaxed),
            corruption_count: self.corruption_count.load(Ordering::Relaxed),
        }
    }

    /// Roll the RNG and return true if we should inject an error.
    async fn should_inject_error(&self) -> bool {
        if self.config.error_rate <= 0.0 {
            return false;
        }
        let roll: f64 = self.rng.lock().await.gen();
        roll < self.config.error_rate
    }

    /// Roll the RNG and return true if we should corrupt read data.
    async fn should_corrupt_read(&self) -> bool {
        if self.config.corruption_rate <= 0.0 {
            return false;
        }
        let roll: f64 = self.rng.lock().await.gen();
        roll < self.config.corruption_rate
    }

    /// Generate a random injected error (ConnectionFailed or Timeout).
    async fn injected_error(&self, op: &str, path: &str) -> BackendError {
        self.fault_count.fetch_add(1, Ordering::Relaxed);
        let use_timeout: bool = self.rng.lock().await.gen();
        if use_timeout {
            BackendError::Timeout {
                operation: format!("{} {}", FAULT_PREFIX, op),
                path: path.to_string(),
            }
        } else {
            BackendError::ConnectionFailed {
                backend: format!("{} faulty", FAULT_PREFIX),
                source: Box::new(std::io::Error::other(format!(
                    "{} connection failed during {}",
                    FAULT_PREFIX, op
                ))),
            }
        }
    }

    /// Corrupt data by flipping a random bit.
    async fn corrupt_data(&self, mut data: Vec<u8>) -> Vec<u8> {
        if data.is_empty() {
            return data;
        }
        self.corruption_count.fetch_add(1, Ordering::Relaxed);
        let mut rng = self.rng.lock().await;
        let byte_idx = rng.gen_range(0..data.len());
        let bit_idx = rng.gen_range(0..8u8);
        data[byte_idx] ^= 1 << bit_idx;
        data
    }
}

#[async_trait]
impl Backend for FaultyBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("read", path).await);
        }
        let data = self.inner.read(path).await?;
        if self.should_corrupt_read().await {
            return Ok(self.corrupt_data(data).await);
        }
        Ok(data)
    }

    async fn read_with_cas_token(
        &self,
        path: &str,
    ) -> Result<(Vec<u8>, Option<String>), BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("read_with_cas_token", path).await);
        }
        let (data, token) = self.inner.read_with_cas_token(path).await?;
        if self.should_corrupt_read().await {
            return Ok((self.corrupt_data(data).await, token));
        }
        Ok((data, token))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("write", path).await);
        }
        self.inner.write(path, content).await
    }

    async fn compare_and_swap(
        &self,
        path: &str,
        expected: Option<&str>,
        content: &[u8],
    ) -> Result<Option<String>, BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("compare_and_swap", path).await);
        }
        self.inner.compare_and_swap(path, expected, content).await
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("append", path).await);
        }
        self.inner.append(path, content).await
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("delete", path).await);
        }
        self.inner.delete(path).await
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("list", path).await);
        }
        self.inner.list(path).await
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("exists", path).await);
        }
        self.inner.exists(path).await
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("stat", path).await);
        }
        self.inner.stat(path).await
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), BackendError> {
        if self.should_inject_error().await {
            return Err(self.injected_error("rename", from).await);
        }
        self.inner.rename(from, to).await
    }
}

/// Check if an error was injected by the fault layer.
pub fn is_injected_fault(msg: &str) -> bool {
    msg.contains(FAULT_PREFIX)
}

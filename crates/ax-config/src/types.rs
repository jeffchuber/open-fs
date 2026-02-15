use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

/// A wrapper type for sensitive values (API keys, passwords, connection strings)
/// that redacts the value in `Debug` and `Display` output to prevent accidental
/// logging of credentials.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Create a new secret from a string.
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    /// Get the secret value. Use sparingly and never log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Consume the wrapper and return the inner string.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret(s)
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Secret(s.to_string())
    }
}

/// Mount mode determines how data flows between local and remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MountMode {
    /// Local-only, no syncing
    Local,
    /// Local with indexing for search
    #[default]
    LocalIndexed,
    /// Write to remote immediately
    WriteThrough,
    /// Buffer writes locally, sync periodically
    WriteBack,
    /// Remote-only, no local cache
    Remote,
    /// Remote with local cache
    RemoteCached,
    /// One-way sync from remote
    PullMirror,
}

/// Search mode for queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SearchMode {
    /// Vector similarity search
    #[default]
    Dense,
    /// Keyword/BM25 search
    Sparse,
    /// Combination of dense and sparse
    Hybrid,
    /// Metadata-only filtering
    Metadata,
}

/// Write synchronization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WriteMode {
    /// Wait for write to complete
    #[default]
    Sync,
    /// Return immediately, write in background
    Async,
}

/// Conflict resolution strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConflictStrategy {
    /// Most recent write wins
    #[default]
    LastWriteWins,
    /// Lock files during edit
    Lock,
    /// Create forked versions
    Fork,
    /// Attempt to reconcile changes
    Reconcile,
}

/// Cache invalidation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InvalidationStrategy {
    /// Time-to-live based
    #[default]
    Ttl,
    /// Periodic polling
    Poll,
    /// Pub/sub notifications
    Pubsub,
    /// Manual invalidation only
    Manual,
}

/// Retry backoff strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Linearly increasing delay
    Linear,
    /// Exponentially increasing delay
    #[default]
    Exponential,
}

/// Chunking strategy for text splitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChunkStrategy {
    /// Fixed-size chunks
    #[default]
    Fixed,
    /// Recursive splitting
    Recursive,
    /// Semantic boundary detection
    Semantic,
    /// AST-aware splitting
    Ast,
    /// Row-based for tabular data
    Row,
}

/// Chunking granularity for code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChunkGranularity {
    /// Entire file
    File,
    /// Class-level
    Class,
    /// Function-level
    #[default]
    Function,
    /// Block-level
    Block,
}

/// Embedding provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EmbeddingProvider {
    /// Local Ollama
    #[default]
    Ollama,
    /// OpenAI API
    OpenAi,
    /// Local sentence transformers
    SentenceTransformers,
    /// Voyage AI
    VoyageAi,
}

/// Human-readable duration (e.g., "200ms", "5m", "1h").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanDuration(pub std::time::Duration);

impl Default for HumanDuration {
    fn default() -> Self {
        HumanDuration(std::time::Duration::from_secs(0))
    }
}

impl HumanDuration {
    pub fn as_duration(&self) -> std::time::Duration {
        self.0
    }
}

impl FromStr for HumanDuration {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();

        // Try to parse number and unit
        let (num_str, unit) = if s.ends_with("ms") {
            (&s[..s.len() - 2], "ms")
        } else if s.ends_with('s') {
            (&s[..s.len() - 1], "s")
        } else if s.ends_with('m') {
            (&s[..s.len() - 1], "m")
        } else if s.ends_with('h') {
            (&s[..s.len() - 1], "h")
        } else if s.ends_with('d') {
            (&s[..s.len() - 1], "d")
        } else {
            return Err(format!("Invalid duration format: {}", s));
        };

        let num: u64 = num_str
            .parse()
            .map_err(|_| format!("Invalid number in duration: {}", s))?;

        let duration = match unit {
            "ms" => std::time::Duration::from_millis(num),
            "s" => std::time::Duration::from_secs(num),
            "m" => std::time::Duration::from_secs(num * 60),
            "h" => std::time::Duration::from_secs(num * 3600),
            "d" => std::time::Duration::from_secs(num * 86400),
            _ => return Err(format!("Unknown duration unit: {}", unit)),
        };

        Ok(HumanDuration(duration))
    }
}

impl fmt::Display for HumanDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0.as_secs();
        let millis = self.0.as_millis();

        if millis < 1000 {
            write!(f, "{}ms", millis)
        } else if secs < 60 {
            write!(f, "{}s", secs)
        } else if secs < 3600 {
            write!(f, "{}m", secs / 60)
        } else if secs < 86400 {
            write!(f, "{}h", secs / 3600)
        } else {
            write!(f, "{}d", secs / 86400)
        }
    }
}

impl Serialize for HumanDuration {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for HumanDuration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        HumanDuration::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Human-readable bytes (e.g., "512mb", "2gb").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HumanBytes(pub u64);

impl HumanBytes {
    pub fn as_bytes(&self) -> u64 {
        self.0
    }
}

impl FromStr for HumanBytes {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();

        // Try to parse number and unit
        let (num_str, multiplier) = if s.ends_with("tb") {
            (&s[..s.len() - 2], 1024u64 * 1024 * 1024 * 1024)
        } else if s.ends_with("gb") {
            (&s[..s.len() - 2], 1024u64 * 1024 * 1024)
        } else if s.ends_with("mb") {
            (&s[..s.len() - 2], 1024u64 * 1024)
        } else if s.ends_with("kb") {
            (&s[..s.len() - 2], 1024u64)
        } else if s.ends_with('b') {
            (&s[..s.len() - 1], 1u64)
        } else {
            // Assume bytes if no unit
            (s.as_str(), 1u64)
        };

        let num: u64 = num_str
            .parse()
            .map_err(|_| format!("Invalid number in bytes: {}", s))?;

        Ok(HumanBytes(num * multiplier))
    }
}

impl fmt::Display for HumanBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.0;

        if bytes >= 1024 * 1024 * 1024 * 1024 {
            write!(f, "{}tb", bytes / (1024 * 1024 * 1024 * 1024))
        } else if bytes >= 1024 * 1024 * 1024 {
            write!(f, "{}gb", bytes / (1024 * 1024 * 1024))
        } else if bytes >= 1024 * 1024 {
            write!(f, "{}mb", bytes / (1024 * 1024))
        } else if bytes >= 1024 {
            write!(f, "{}kb", bytes / 1024)
        } else {
            write!(f, "{}b", bytes)
        }
    }
}

impl Serialize for HumanBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for HumanBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        HumanBytes::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Local filesystem backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsBackendConfig {
    pub root: String,
}

/// In-memory backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MemoryBackendConfig {}

/// S3 backend configuration (stub).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3BackendConfig {
    pub bucket: String,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub access_key_id: Option<Secret>,
    #[serde(default)]
    pub secret_access_key: Option<Secret>,
}

/// Postgres backend configuration (stub).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostgresBackendConfig {
    #[serde(alias = "connection_string")]
    pub connection_url: Secret,
    #[serde(default, alias = "table")]
    pub table_name: Option<String>,
    #[serde(default)]
    pub max_connections: Option<u32>,
}

/// Chroma backend configuration (stub).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChromaBackendConfig {
    pub url: String,
    #[serde(default)]
    pub collection: Option<String>,
}

/// API backend configuration (stub).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiBackendConfig {
    pub base_url: String,
    #[serde(default)]
    pub auth_header: Option<Secret>,
}

/// Tagged enum for backend configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BackendConfig {
    Fs(FsBackendConfig),
    #[serde(rename = "memory", alias = "mem")]
    Memory(MemoryBackendConfig),
    S3(S3BackendConfig),
    Postgres(PostgresBackendConfig),
    Chroma(ChromaBackendConfig),
    Api(ApiBackendConfig),
}

/// Chunking configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkConfig {
    #[serde(default)]
    pub strategy: ChunkStrategy,
    #[serde(default = "default_chunk_size")]
    pub size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
    #[serde(default)]
    pub granularity: ChunkGranularity,
}

fn default_chunk_size() -> usize {
    512
}

fn default_chunk_overlap() -> usize {
    64
}

impl Default for ChunkConfig {
    fn default() -> Self {
        ChunkConfig {
            strategy: ChunkStrategy::default(),
            size: default_chunk_size(),
            overlap: default_chunk_overlap(),
            granularity: ChunkGranularity::default(),
        }
    }
}

/// Embedding configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingConfig {
    #[serde(default)]
    pub provider: EmbeddingProvider,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_embedding_dimensions")]
    pub dimensions: usize,
}

fn default_embedding_dimensions() -> usize {
    384
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        EmbeddingConfig {
            provider: EmbeddingProvider::default(),
            model: None,
            dimensions: default_embedding_dimensions(),
        }
    }
}

/// Indexing configuration for a mount.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct IndexConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub search_modes: Vec<SearchMode>,
    #[serde(default)]
    pub chunk: Option<ChunkConfig>,
    #[serde(default)]
    pub embedding: Option<EmbeddingConfig>,
}

/// Sync configuration for a mount.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SyncConfig {
    #[serde(default)]
    pub interval: Option<HumanDuration>,
    #[serde(default)]
    pub write_mode: WriteMode,
    #[serde(default)]
    pub conflict: ConflictStrategy,
}

/// Watch configuration for file change notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchConfig {
    /// Use native OS file watching (inotify/FSEvents). Defaults to true.
    #[serde(default = "default_true")]
    pub native: bool,
    /// Polling interval (used when native is false or as fallback).
    #[serde(default)]
    pub poll_interval: Option<HumanDuration>,
    /// Debounce interval before emitting change events. Defaults to 500ms.
    #[serde(default = "default_debounce")]
    pub debounce: HumanDuration,
    /// Automatically re-index changed files.
    #[serde(default)]
    pub auto_index: bool,
    /// Webhook URL to POST change notifications to.
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// File patterns to include in watching.
    #[serde(default)]
    pub include: Vec<String>,
    /// File patterns to exclude from watching.
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_debounce() -> HumanDuration {
    HumanDuration(std::time::Duration::from_millis(500))
}

impl Default for WatchConfig {
    fn default() -> Self {
        WatchConfig {
            native: true,
            poll_interval: None,
            debounce: default_debounce(),
            auto_index: false,
            webhook_url: None,
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

/// Mount configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountConfig {
    pub path: String,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub mode: Option<MountMode>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub index: Option<IndexConfig>,
    #[serde(default)]
    pub sync: Option<SyncConfig>,
    #[serde(default)]
    pub watch: Option<WatchConfig>,
}

/// Top-level VFS configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfsConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub backends: IndexMap<String, BackendConfig>,
    #[serde(default)]
    pub mounts: Vec<MountConfig>,
    #[serde(default)]
    pub defaults: Option<DefaultsConfig>,
}

/// Global defaults configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DefaultsConfig {
    #[serde(default)]
    pub chunk: Option<ChunkConfig>,
    #[serde(default)]
    pub embedding: Option<EmbeddingConfig>,
    #[serde(default)]
    pub sync: Option<SyncConfig>,
    #[serde(default)]
    pub watch: Option<WatchConfig>,
}

impl Default for VfsConfig {
    fn default() -> Self {
        VfsConfig {
            name: None,
            version: None,
            backends: IndexMap::new(),
            mounts: Vec::new(),
            defaults: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_duration_parsing() {
        assert_eq!(
            HumanDuration::from_str("200ms").unwrap().as_duration(),
            std::time::Duration::from_millis(200)
        );
        assert_eq!(
            HumanDuration::from_str("5s").unwrap().as_duration(),
            std::time::Duration::from_secs(5)
        );
        assert_eq!(
            HumanDuration::from_str("5m").unwrap().as_duration(),
            std::time::Duration::from_secs(300)
        );
        assert_eq!(
            HumanDuration::from_str("1h").unwrap().as_duration(),
            std::time::Duration::from_secs(3600)
        );
    }

    #[test]
    fn test_secret_redacts_debug() {
        let secret = Secret::new("my-api-key-12345");
        let debug_output = format!("{:?}", secret);
        assert_eq!(debug_output, "Secret(***)");
        assert!(!debug_output.contains("my-api-key"));
    }

    #[test]
    fn test_secret_redacts_display() {
        let secret = Secret::new("super-secret-password");
        let display_output = format!("{}", secret);
        assert_eq!(display_output, "***");
        assert!(!display_output.contains("super-secret"));
    }

    #[test]
    fn test_secret_expose() {
        let secret = Secret::new("my-value");
        assert_eq!(secret.expose(), "my-value");
    }

    #[test]
    fn test_secret_serde_roundtrip() {
        let secret = Secret::new("test-key");
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(json, "\"test-key\"");
        let deserialized: Secret = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.expose(), "test-key");
    }

    #[test]
    fn test_postgres_config_redacts_connection_url() {
        let config = PostgresBackendConfig {
            connection_url: Secret::new("postgres://user:password@host/db"),
            table_name: None,
            max_connections: None,
        };
        let debug = format!("{:?}", config);
        assert!(!debug.contains("password"));
        assert!(debug.contains("Secret(***)"));
    }

    #[test]
    fn test_human_bytes_parsing() {
        assert_eq!(HumanBytes::from_str("512b").unwrap().as_bytes(), 512);
        assert_eq!(HumanBytes::from_str("1kb").unwrap().as_bytes(), 1024);
        assert_eq!(
            HumanBytes::from_str("512mb").unwrap().as_bytes(),
            512 * 1024 * 1024
        );
        assert_eq!(
            HumanBytes::from_str("2gb").unwrap().as_bytes(),
            2 * 1024 * 1024 * 1024
        );
    }
}

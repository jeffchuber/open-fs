use std::collections::HashSet;

use async_trait::async_trait;
use aws_sdk_s3::primitives::DateTime as AwsDateTime;
use chrono::{DateTime, Utc};

use ax_config::Secret;
use crate::error::BackendError;
use crate::traits::{Backend, Entry};

/// S3-compatible storage backend configuration.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// S3 bucket name.
    pub bucket: String,
    /// Optional key prefix (acts like a root directory).
    pub prefix: Option<String>,
    /// AWS region.
    pub region: String,
    /// Optional endpoint URL (for S3-compatible services like MinIO).
    pub endpoint: Option<String>,
    /// Access key ID (optional, uses default credentials if not provided).
    pub access_key_id: Option<Secret>,
    /// Secret access key.
    pub secret_access_key: Option<Secret>,
}

impl Default for S3Config {
    fn default() -> Self {
        S3Config {
            bucket: String::new(),
            prefix: None,
            region: "us-east-1".to_string(),
            endpoint: None,
            access_key_id: None,
            secret_access_key: None,
        }
    }
}

/// S3-compatible storage backend.
///
/// This backend stores files in an S3 bucket, with support for:
/// - AWS S3
/// - MinIO
/// - DigitalOcean Spaces
/// - Backblaze B2
/// - Any S3-compatible service
pub struct S3Backend {
    client: aws_sdk_s3::Client,
    bucket: String,
    prefix: String,
}

impl S3Backend {
    /// Create a new S3 backend with the given configuration.
    pub async fn new(config: S3Config) -> Result<Self, BackendError> {
        let mut aws_config_builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new(config.region.clone()));

        // Use custom credentials if provided
        if let (Some(access_key), Some(secret_key)) = (&config.access_key_id, &config.secret_access_key) {
            let credentials = aws_sdk_s3::config::Credentials::new(
                access_key.expose(),
                secret_key.expose(),
                None,
                None,
                "ax-s3-backend",
            );
            aws_config_builder = aws_config_builder.credentials_provider(credentials);
        }

        let aws_config = aws_config_builder.load().await;

        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(&aws_config);

        // Use custom endpoint if provided (for MinIO, etc.)
        if let Some(endpoint) = &config.endpoint {
            s3_config_builder = s3_config_builder
                .endpoint_url(endpoint)
                .force_path_style(true);
        }

        let client = aws_sdk_s3::Client::from_conf(s3_config_builder.build());

        let prefix = config.prefix.unwrap_or_default();

        Ok(S3Backend {
            client,
            bucket: config.bucket,
            prefix,
        })
    }

    /// Convert a VFS path to an S3 key.
    fn path_to_key(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if self.prefix.is_empty() {
            path.to_string()
        } else {
            format!("{}/{}", self.prefix.trim_end_matches('/'), path)
        }
    }

    /// Convert an S3 key to a VFS path.
    fn key_to_path(&self, key: &str) -> String {
        let path = if self.prefix.is_empty() {
            key.to_string()
        } else {
            key.strip_prefix(&format!("{}/", self.prefix.trim_end_matches('/')))
                .unwrap_or(key)
                .to_string()
        };
        format!("/{}", path)
    }

    /// Extract the filename from a path.
    fn filename(path: &str) -> String {
        path.rsplit('/').next().unwrap_or(path).to_string()
    }
}

#[async_trait]
impl Backend for S3Backend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let key = self.path_to_key(path);

        let response = self.client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| {
                if e.to_string().contains("NoSuchKey") {
                    BackendError::NotFound(path.to_string())
                } else {
                    BackendError::Other(format!("S3 get failed: {}", e))
                }
            })?;

        let body = response.body.collect().await
            .map_err(|e| BackendError::Other(format!("S3 read body failed: {}", e)))?;

        Ok(body.into_bytes().to_vec())
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let key = self.path_to_key(path);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(content.to_vec().into())
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("S3 put failed: {}", e)))?;

        Ok(())
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        // S3 doesn't support append, so we read + write
        let existing = match self.read(path).await {
            Ok(data) => data,
            Err(BackendError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };

        let mut new_content = existing;
        new_content.extend_from_slice(content);

        self.write(path, &new_content).await
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        let key = self.path_to_key(path);

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("S3 delete failed: {}", e)))?;

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let prefix = self.path_to_key(path);
        let prefix = if prefix.is_empty() || prefix == "/" {
            if self.prefix.is_empty() {
                String::new()
            } else {
                format!("{}/", self.prefix.trim_end_matches('/'))
            }
        } else {
            format!("{}/", prefix.trim_end_matches('/'))
        };

        let mut entries = Vec::new();
        let mut seen_dirs: HashSet<String> = HashSet::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let mut request = self.client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&prefix)
                .delimiter("/");

            if let Some(token) = &continuation_token {
                request = request.continuation_token(token);
            }

            let response = request
                .send()
                .await
                .map_err(|e| BackendError::Other(format!("S3 list failed: {}", e)))?;

            // Add common prefixes as directories
            for cp in response.common_prefixes() {
                if let Some(p) = cp.prefix() {
                    let dir_name = p.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                    if !dir_name.is_empty() && seen_dirs.insert(dir_name.to_string()) {
                        entries.push(Entry::dir(
                            self.key_to_path(p.trim_end_matches('/')),
                            dir_name.to_string(),
                            None,
                        ));
                    }
                }
            }

            // Add objects as files
            for obj in response.contents() {
                if let Some(key) = obj.key() {
                    // Skip the directory marker itself
                    if key == prefix || key.ends_with('/') {
                        continue;
                    }

                    let name = Self::filename(key);
                    let size = obj.size().map(|s| s as u64);
                    let modified = obj.last_modified()
                        .and_then(|t: &AwsDateTime| DateTime::from_timestamp(t.secs(), t.subsec_nanos()))
                        .map(|dt: DateTime<Utc>| dt.with_timezone(&Utc));

                    entries.push(Entry::file(
                        self.key_to_path(key),
                        name,
                        size.unwrap_or(0),
                        modified,
                    ));
                }
            }

            // Check for more pages
            if response.is_truncated() == Some(true) {
                continuation_token = response.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        // Sort: directories first, then alphabetically
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let key = self.path_to_key(path);

        match self.client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                if e.to_string().contains("NotFound") || e.to_string().contains("NoSuchKey") {
                    Ok(false)
                } else {
                    Err(BackendError::Other(format!("S3 head failed: {}", e)))
                }
            }
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let key = self.path_to_key(path);

        let response = self.client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| {
                if e.to_string().contains("NotFound") || e.to_string().contains("NoSuchKey") {
                    BackendError::NotFound(path.to_string())
                } else {
                    BackendError::Other(format!("S3 head failed: {}", e))
                }
            })?;

        let size = response.content_length().map(|s| s as u64).unwrap_or(0);
        let modified = response.last_modified()
            .and_then(|t: &AwsDateTime| DateTime::from_timestamp(t.secs(), t.subsec_nanos()))
            .map(|dt: DateTime<Utc>| dt.with_timezone(&Utc));

        Ok(Entry::file(
            path.to_string(),
            Self::filename(path),
            size,
            modified,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper functions for testing path conversions without needing a real client
    fn path_to_key_with_prefix(prefix: &str, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if prefix.is_empty() {
            path.to_string()
        } else {
            format!("{}/{}", prefix.trim_end_matches('/'), path)
        }
    }

    fn key_to_path_with_prefix(prefix: &str, key: &str) -> String {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            key.strip_prefix(&format!("{}/", prefix.trim_end_matches('/')))
                .unwrap_or(key)
                .to_string()
        };
        format!("/{}", path)
    }

    #[test]
    fn test_path_to_key() {
        let prefix = "data";
        assert_eq!(path_to_key_with_prefix(prefix, "/file.txt"), "data/file.txt");
        assert_eq!(path_to_key_with_prefix(prefix, "/dir/file.txt"), "data/dir/file.txt");
        assert_eq!(path_to_key_with_prefix(prefix, "file.txt"), "data/file.txt");
    }

    #[test]
    fn test_path_to_key_no_prefix() {
        let prefix = "";
        assert_eq!(path_to_key_with_prefix(prefix, "/file.txt"), "file.txt");
        assert_eq!(path_to_key_with_prefix(prefix, "/dir/file.txt"), "dir/file.txt");
    }

    #[test]
    fn test_key_to_path() {
        let prefix = "data";
        assert_eq!(key_to_path_with_prefix(prefix, "data/file.txt"), "/file.txt");
        assert_eq!(key_to_path_with_prefix(prefix, "data/dir/file.txt"), "/dir/file.txt");
    }

    #[test]
    fn test_key_to_path_no_prefix() {
        let prefix = "";
        assert_eq!(key_to_path_with_prefix(prefix, "file.txt"), "/file.txt");
        assert_eq!(key_to_path_with_prefix(prefix, "dir/file.txt"), "/dir/file.txt");
    }

    #[test]
    fn test_filename() {
        assert_eq!(S3Backend::filename("dir/file.txt"), "file.txt");
        assert_eq!(S3Backend::filename("file.txt"), "file.txt");
        assert_eq!(S3Backend::filename("/a/b/c.rs"), "c.rs");
    }
}

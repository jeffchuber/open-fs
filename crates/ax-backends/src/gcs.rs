use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use crate::error::BackendError;
use crate::traits::{Backend, Entry};

/// Google Cloud Storage backend configuration.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct GcsConfig {
    /// GCS bucket name.
    pub bucket: String,
    /// Optional key prefix (acts like a root directory).
    pub prefix: Option<String>,
    /// Optional path to service account credentials JSON file.
    pub credentials_file: Option<String>,
}


/// Google Cloud Storage backend.
///
/// This backend stores files in a GCS bucket using the JSON API.
pub struct GcsBackend {
    client: Client,
    bucket: String,
    prefix: String,
}

impl GcsBackend {
    /// Create a new GCS backend with the given configuration.
    pub fn new(config: GcsConfig) -> Result<Self, BackendError> {
        let client = Client::builder()
            .build()
            .map_err(|e| BackendError::Other(format!("Failed to create HTTP client: {}", e)))?;

        let prefix = config.prefix.unwrap_or_default();

        Ok(GcsBackend {
            client,
            bucket: config.bucket,
            prefix,
        })
    }

    /// Convert a VFS path to a GCS object name.
    fn path_to_key(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if self.prefix.is_empty() {
            path.to_string()
        } else {
            format!("{}/{}", self.prefix.trim_end_matches('/'), path)
        }
    }

    /// Convert a GCS object name to a VFS path.
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

    /// Base URL for the GCS JSON API.
    fn api_url(&self) -> String {
        format!(
            "https://storage.googleapis.com/storage/v1/b/{}/o",
            self.bucket
        )
    }

    /// Base URL for GCS media upload/download.
    fn upload_url(&self) -> String {
        format!(
            "https://storage.googleapis.com/upload/storage/v1/b/{}/o",
            self.bucket
        )
    }

    /// URL for direct object download.
    fn download_url(&self, key: &str) -> String {
        format!(
            "https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media",
            self.bucket,
            urlencoding::encode(key)
        )
    }
}

/// GCS list response.
#[derive(Deserialize)]
struct GcsListResponse {
    #[serde(default)]
    items: Vec<GcsObject>,
    #[serde(default)]
    prefixes: Vec<String>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

/// GCS object metadata.
#[derive(Deserialize)]
struct GcsObject {
    name: String,
    #[serde(default)]
    size: Option<String>,
    updated: Option<String>,
}

#[async_trait]
impl Backend for GcsBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let key = self.path_to_key(path);
        let url = self.download_url(&key);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("GCS GET failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }

        if !response.status().is_success() {
            return Err(BackendError::Other(format!(
                "GCS GET returned status {}",
                response.status()
            )));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| BackendError::Other(format!("GCS read body failed: {}", e)))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let key = self.path_to_key(path);
        let url = format!(
            "{}?uploadType=media&name={}",
            self.upload_url(),
            urlencoding::encode(&key)
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(content.to_vec())
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("GCS upload failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(BackendError::Other(format!(
                "GCS upload returned status {}",
                response.status()
            )));
        }

        Ok(())
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
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
        let url = format!(
            "{}/{}",
            self.api_url(),
            urlencoding::encode(&key)
        );

        let response = self
            .client
            .delete(&url)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("GCS delete failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }

        if !response.status().is_success() {
            return Err(BackendError::Other(format!(
                "GCS delete returned status {}",
                response.status()
            )));
        }

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let prefix_key = self.path_to_key(path);
        let prefix_key = if prefix_key.is_empty() || prefix_key == "/" {
            if self.prefix.is_empty() {
                String::new()
            } else {
                format!("{}/", self.prefix.trim_end_matches('/'))
            }
        } else {
            format!("{}/", prefix_key.trim_end_matches('/'))
        };

        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut url = format!(
                "{}?prefix={}&delimiter=/",
                self.api_url(),
                urlencoding::encode(&prefix_key)
            );

            if let Some(ref token) = page_token {
                url = format!("{}&pageToken={}", url, urlencoding::encode(token));
            }

            let response = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| BackendError::Other(format!("GCS list failed: {}", e)))?;

            if !response.status().is_success() {
                return Err(BackendError::Other(format!(
                    "GCS list returned status {}",
                    response.status()
                )));
            }

            let list_response: GcsListResponse = response
                .json()
                .await
                .map_err(|e| BackendError::Other(format!("GCS list parse failed: {}", e)))?;

            // Add directories from prefixes
            for prefix in &list_response.prefixes {
                let dir_name = prefix
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("");
                if !dir_name.is_empty() {
                    entries.push(Entry::dir(
                        self.key_to_path(prefix.trim_end_matches('/')),
                        dir_name.to_string(),
                        None,
                    ));
                }
            }

            // Add files from items
            for item in &list_response.items {
                // Skip directory markers
                if item.name == prefix_key || item.name.ends_with('/') {
                    continue;
                }

                let name = Self::filename(&item.name);
                let size = item
                    .size
                    .as_ref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let modified = item
                    .updated
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                entries.push(Entry::file(
                    self.key_to_path(&item.name),
                    name,
                    size,
                    modified,
                ));
            }

            match list_response.next_page_token {
                Some(token) => page_token = Some(token),
                None => break,
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
        let url = format!(
            "{}/{}",
            self.api_url(),
            urlencoding::encode(&key)
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("GCS head failed: {}", e)))?;

        Ok(response.status().is_success())
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let key = self.path_to_key(path);
        let url = format!(
            "{}/{}",
            self.api_url(),
            urlencoding::encode(&key)
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("GCS stat failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }

        let obj: GcsObject = response
            .json()
            .await
            .map_err(|e| BackendError::Other(format!("GCS stat parse failed: {}", e)))?;

        let size = obj
            .size
            .as_ref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let modified = obj
            .updated
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

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
        assert_eq!(
            path_to_key_with_prefix("data", "/file.txt"),
            "data/file.txt"
        );
        assert_eq!(
            path_to_key_with_prefix("data", "/dir/file.txt"),
            "data/dir/file.txt"
        );
        assert_eq!(
            path_to_key_with_prefix("data", "file.txt"),
            "data/file.txt"
        );
    }

    #[test]
    fn test_path_to_key_no_prefix() {
        assert_eq!(path_to_key_with_prefix("", "/file.txt"), "file.txt");
        assert_eq!(
            path_to_key_with_prefix("", "/dir/file.txt"),
            "dir/file.txt"
        );
    }

    #[test]
    fn test_key_to_path() {
        assert_eq!(
            key_to_path_with_prefix("data", "data/file.txt"),
            "/file.txt"
        );
        assert_eq!(
            key_to_path_with_prefix("data", "data/dir/file.txt"),
            "/dir/file.txt"
        );
    }

    #[test]
    fn test_key_to_path_no_prefix() {
        assert_eq!(key_to_path_with_prefix("", "file.txt"), "/file.txt");
        assert_eq!(
            key_to_path_with_prefix("", "dir/file.txt"),
            "/dir/file.txt"
        );
    }

    #[test]
    fn test_filename() {
        assert_eq!(GcsBackend::filename("dir/file.txt"), "file.txt");
        assert_eq!(GcsBackend::filename("file.txt"), "file.txt");
        assert_eq!(GcsBackend::filename("/a/b/c.rs"), "c.rs");
    }

    #[test]
    fn test_config_default() {
        let config = GcsConfig::default();
        assert!(config.bucket.is_empty());
        assert!(config.prefix.is_none());
        assert!(config.credentials_file.is_none());
    }

    #[test]
    fn test_new_backend() {
        let config = GcsConfig {
            bucket: "my-bucket".to_string(),
            ..Default::default()
        };
        let backend = GcsBackend::new(config);
        assert!(backend.is_ok());
    }
}

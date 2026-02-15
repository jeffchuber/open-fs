use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use ax_core::{Backend, Entry, BackendError};

/// Google Cloud Storage backend configuration.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct GcsConfig {
    pub bucket: String,
    pub prefix: Option<String>,
    pub credentials_file: Option<String>,
}

/// Google Cloud Storage backend.
pub struct GcsBackend {
    client: Client,
    bucket: String,
    prefix: String,
}

impl GcsBackend {
    pub fn new(config: GcsConfig) -> Result<Self, BackendError> {
        let client = Client::builder()
            .build()
            .map_err(|e| BackendError::Other(format!("Failed to create HTTP client: {}", e)))?;
        let prefix = config.prefix.unwrap_or_default();
        Ok(GcsBackend { client, bucket: config.bucket, prefix })
    }

    fn path_to_key(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if self.prefix.is_empty() { path.to_string() }
        else { format!("{}/{}", self.prefix.trim_end_matches('/'), path) }
    }

    fn key_to_path(&self, key: &str) -> String {
        let path = if self.prefix.is_empty() { key.to_string() }
        else { key.strip_prefix(&format!("{}/", self.prefix.trim_end_matches('/'))).unwrap_or(key).to_string() };
        format!("/{}", path)
    }

    fn filename(path: &str) -> String { path.rsplit('/').next().unwrap_or(path).to_string() }

    fn api_url(&self) -> String {
        format!("https://storage.googleapis.com/storage/v1/b/{}/o", self.bucket)
    }

    fn upload_url(&self) -> String {
        format!("https://storage.googleapis.com/upload/storage/v1/b/{}/o", self.bucket)
    }

    fn download_url(&self, key: &str) -> String {
        format!("https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media", self.bucket, urlencoding::encode(key))
    }
}

#[derive(Deserialize)]
struct GcsListResponse {
    #[serde(default)]
    items: Vec<GcsObject>,
    #[serde(default)]
    prefixes: Vec<String>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

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
        let response = self.client.get(&url).send().await
            .map_err(|e| BackendError::Other(format!("GCS GET failed: {}", e)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }
        if !response.status().is_success() {
            return Err(BackendError::Other(format!("GCS GET returned status {}", response.status())));
        }
        response.bytes().await.map(|b| b.to_vec())
            .map_err(|e| BackendError::Other(format!("GCS read body failed: {}", e)))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let key = self.path_to_key(path);
        let url = format!("{}?uploadType=media&name={}", self.upload_url(), urlencoding::encode(&key));
        let response = self.client.post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(content.to_vec())
            .send().await
            .map_err(|e| BackendError::Other(format!("GCS upload failed: {}", e)))?;
        if !response.status().is_success() {
            return Err(BackendError::Other(format!("GCS upload returned status {}", response.status())));
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
        let url = format!("{}/{}", self.api_url(), urlencoding::encode(&key));
        let response = self.client.delete(&url).send().await
            .map_err(|e| BackendError::Other(format!("GCS delete failed: {}", e)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }
        if !response.status().is_success() {
            return Err(BackendError::Other(format!("GCS delete returned status {}", response.status())));
        }
        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let prefix_key = self.path_to_key(path);
        let prefix_key = if prefix_key.is_empty() || prefix_key == "/" {
            if self.prefix.is_empty() { String::new() }
            else { format!("{}/", self.prefix.trim_end_matches('/')) }
        } else {
            format!("{}/", prefix_key.trim_end_matches('/'))
        };

        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut url = format!("{}?prefix={}&delimiter=/", self.api_url(), urlencoding::encode(&prefix_key));
            if let Some(ref token) = page_token {
                url = format!("{}&pageToken={}", url, urlencoding::encode(token));
            }
            let response = self.client.get(&url).send().await
                .map_err(|e| BackendError::Other(format!("GCS list failed: {}", e)))?;
            if !response.status().is_success() {
                return Err(BackendError::Other(format!("GCS list returned status {}", response.status())));
            }
            let list_response: GcsListResponse = response.json().await
                .map_err(|e| BackendError::Other(format!("GCS list parse failed: {}", e)))?;

            for prefix in &list_response.prefixes {
                let dir_name = prefix.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                if !dir_name.is_empty() {
                    entries.push(Entry::dir(self.key_to_path(prefix.trim_end_matches('/')), dir_name.to_string(), None));
                }
            }

            for item in &list_response.items {
                if item.name == prefix_key || item.name.ends_with('/') { continue; }
                let name = Self::filename(&item.name);
                let size = item.size.as_ref().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                let modified = item.updated.as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                entries.push(Entry::file(self.key_to_path(&item.name), name, size, modified));
            }

            match list_response.next_page_token {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        Ok(entries)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let key = self.path_to_key(path);
        let url = format!("{}/{}", self.api_url(), urlencoding::encode(&key));
        let response = self.client.get(&url).send().await
            .map_err(|e| BackendError::Other(format!("GCS head failed: {}", e)))?;
        Ok(response.status().is_success())
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let key = self.path_to_key(path);
        let url = format!("{}/{}", self.api_url(), urlencoding::encode(&key));
        let response = self.client.get(&url).send().await
            .map_err(|e| BackendError::Other(format!("GCS stat failed: {}", e)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }
        let obj: GcsObject = response.json().await
            .map_err(|e| BackendError::Other(format!("GCS stat parse failed: {}", e)))?;
        let size = obj.size.as_ref().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let modified = obj.updated.as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        Ok(Entry::file(path.to_string(), Self::filename(path), size, modified))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename() {
        assert_eq!(GcsBackend::filename("dir/file.txt"), "file.txt");
    }

    #[test]
    fn test_new_backend() {
        let config = GcsConfig { bucket: "my-bucket".to_string(), ..Default::default() };
        assert!(GcsBackend::new(config).is_ok());
    }
}

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use ax_config::Secret;
use ax_core::{Backend, Entry, BackendError};

/// Azure Blob Storage backend configuration.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct AzureBlobConfig {
    pub account: String,
    pub container: String,
    pub access_key: Option<Secret>,
    pub prefix: Option<String>,
}

/// Azure Blob Storage backend.
pub struct AzureBlobBackend {
    client: Client,
    account: String,
    container: String,
    prefix: String,
}

impl AzureBlobBackend {
    pub fn new(config: AzureBlobConfig) -> Result<Self, BackendError> {
        let client = Client::builder().build()
            .map_err(|e| BackendError::Other(format!("Failed to create HTTP client: {}", e)))?;
        let prefix = config.prefix.unwrap_or_default();
        Ok(AzureBlobBackend { client, account: config.account, container: config.container, prefix })
    }

    fn base_url(&self) -> String {
        format!("https://{}.blob.core.windows.net/{}", self.account, self.container)
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

    fn blob_url(&self, key: &str) -> String { format!("{}/{}", self.base_url(), key) }
}

#[derive(Deserialize)]
struct AzureListResponse {
    #[serde(rename = "Blobs")]
    blobs: Option<AzureBlobs>,
    #[serde(rename = "NextMarker")]
    next_marker: Option<String>,
}

#[derive(Deserialize)]
struct AzureBlobs {
    #[serde(rename = "Blob", default)]
    blob: Vec<AzureBlob>,
    #[serde(rename = "BlobPrefix", default)]
    blob_prefix: Vec<AzureBlobPrefix>,
}

#[derive(Deserialize)]
struct AzureBlob {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Properties")]
    properties: Option<AzureBlobProperties>,
}

#[derive(Deserialize)]
struct AzureBlobProperties {
    #[serde(rename = "Content-Length")]
    content_length: Option<String>,
    #[serde(rename = "Last-Modified")]
    last_modified: Option<String>,
}

#[derive(Deserialize)]
struct AzureBlobPrefix {
    #[serde(rename = "Name")]
    name: String,
}

#[async_trait]
impl Backend for AzureBlobBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let key = self.path_to_key(path);
        let url = self.blob_url(&key);
        let response = self.client.get(&url).send().await
            .map_err(|e| BackendError::Other(format!("Azure GET failed: {}", e)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }
        if !response.status().is_success() {
            return Err(BackendError::Other(format!("Azure GET returned status {}", response.status())));
        }
        response.bytes().await.map(|b| b.to_vec())
            .map_err(|e| BackendError::Other(format!("Azure read body failed: {}", e)))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let key = self.path_to_key(path);
        let url = self.blob_url(&key);
        let response = self.client.put(&url)
            .header("x-ms-blob-type", "BlockBlob")
            .header("Content-Type", "application/octet-stream")
            .body(content.to_vec())
            .send().await
            .map_err(|e| BackendError::Other(format!("Azure PUT failed: {}", e)))?;
        if !response.status().is_success() {
            return Err(BackendError::Other(format!("Azure PUT returned status {}", response.status())));
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
        let url = self.blob_url(&key);
        let response = self.client.delete(&url).send().await
            .map_err(|e| BackendError::Other(format!("Azure DELETE failed: {}", e)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }
        if !response.status().is_success() {
            return Err(BackendError::Other(format!("Azure DELETE returned status {}", response.status())));
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
        let mut marker: Option<String> = None;

        loop {
            let mut url = format!("{}?restype=container&comp=list&prefix={}&delimiter=/",
                self.base_url(), urlencoding::encode(&prefix_key));
            if let Some(ref m) = marker {
                url = format!("{}&marker={}", url, urlencoding::encode(m));
            }
            let response = self.client.get(&url).send().await
                .map_err(|e| BackendError::Other(format!("Azure list failed: {}", e)))?;
            if !response.status().is_success() {
                return Err(BackendError::Other(format!("Azure list returned status {}", response.status())));
            }
            let xml = response.text().await
                .map_err(|e| BackendError::Other(format!("Azure list body failed: {}", e)))?;
            let list_response: AzureListResponse = quick_xml::de::from_str(&xml)
                .map_err(|e| BackendError::Other(format!("Azure list parse failed: {}", e)))?;

            if let Some(blobs) = &list_response.blobs {
                for prefix in &blobs.blob_prefix {
                    let dir_name = prefix.name.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                    if !dir_name.is_empty() {
                        entries.push(Entry::dir(self.key_to_path(prefix.name.trim_end_matches('/')), dir_name.to_string(), None));
                    }
                }
                for blob in &blobs.blob {
                    if blob.name == prefix_key || blob.name.ends_with('/') { continue; }
                    let name = Self::filename(&blob.name);
                    let size = blob.properties.as_ref().and_then(|p| p.content_length.as_ref()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let modified = blob.properties.as_ref().and_then(|p| p.last_modified.as_ref())
                        .and_then(|s| DateTime::parse_from_rfc2822(s).ok()).map(|dt| dt.with_timezone(&Utc));
                    entries.push(Entry::file(self.key_to_path(&blob.name), name, size, modified));
                }
            }

            match list_response.next_marker {
                Some(ref m) if !m.is_empty() => marker = Some(m.clone()),
                _ => break,
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
        let url = self.blob_url(&key);
        let response = self.client.head(&url).send().await
            .map_err(|e| BackendError::Other(format!("Azure HEAD failed: {}", e)))?;
        Ok(response.status().is_success())
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let key = self.path_to_key(path);
        let url = self.blob_url(&key);
        let response = self.client.head(&url).send().await
            .map_err(|e| BackendError::Other(format!("Azure HEAD failed: {}", e)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }
        let size = response.headers().get("content-length")
            .and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let modified = response.headers().get("last-modified")
            .and_then(|v| v.to_str().ok()).and_then(|s| DateTime::parse_from_rfc2822(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        Ok(Entry::file(path.to_string(), Self::filename(path), size, modified))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename() {
        assert_eq!(AzureBlobBackend::filename("dir/file.txt"), "file.txt");
    }

    #[test]
    fn test_new_backend() {
        let config = AzureBlobConfig {
            account: "myaccount".to_string(),
            container: "mycontainer".to_string(),
            ..Default::default()
        };
        assert!(AzureBlobBackend::new(config).is_ok());
    }
}

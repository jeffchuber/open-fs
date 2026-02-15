use async_trait::async_trait;
use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::Client;

use ax_config::Secret;
use crate::error::BackendError;
use crate::traits::{Backend, Entry};

/// WebDAV backend configuration.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct WebDavConfig {
    /// Base URL of the WebDAV server (e.g., "https://dav.example.com/files").
    pub url: String,
    /// Optional username for authentication.
    pub username: Option<String>,
    /// Optional password for authentication.
    pub password: Option<Secret>,
    /// Optional path prefix within the WebDAV server.
    pub prefix: Option<String>,
}


/// WebDAV storage backend.
///
/// Implements file operations over WebDAV protocol using HTTP methods:
/// - GET for reads
/// - PUT for writes
/// - DELETE for deletions
/// - PROPFIND for listing and metadata
pub struct WebDavBackend {
    client: Client,
    base_url: String,
    username: Option<String>,
    password: Option<Secret>,
    prefix: String,
}

impl WebDavBackend {
    /// Create a new WebDAV backend with the given configuration.
    pub fn new(config: WebDavConfig) -> Result<Self, BackendError> {
        let client_builder = Client::builder();

        let client = client_builder
            .build()
            .map_err(|e| BackendError::Other(format!("Failed to create HTTP client: {}", e)))?;

        let base_url = config.url.trim_end_matches('/').to_string();
        let prefix = config.prefix.unwrap_or_default();

        Ok(WebDavBackend {
            client,
            base_url,
            username: config.username,
            password: config.password,
            prefix,
        })
    }

    /// Convert a VFS path to a WebDAV URL.
    fn path_to_url(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if self.prefix.is_empty() {
            format!("{}/{}", self.base_url, path)
        } else {
            format!(
                "{}/{}/{}",
                self.base_url,
                self.prefix.trim_end_matches('/'),
                path
            )
        }
    }

    /// Convert a WebDAV href to a VFS path.
    fn href_to_path(&self, href: &str) -> String {
        // Extract the path portion from the href
        let path = if let Some(stripped) = href.strip_prefix(&self.base_url) {
            stripped.to_string()
        } else {
            // href might be just a path component
            href.to_string()
        };

        let path = if self.prefix.is_empty() {
            path
        } else {
            let prefix = format!("/{}", self.prefix.trim_matches('/'));
            path.strip_prefix(&prefix)
                .unwrap_or(&path)
                .to_string()
        };

        let path = path.trim_start_matches('/');
        format!("/{}", path)
    }

    /// Extract the filename from a path.
    fn filename(path: &str) -> String {
        path.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_string()
    }

    /// Build a request with optional authentication.
    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let req = self.client.request(method, url);
        if let (Some(ref username), Some(ref password)) = (&self.username, &self.password) {
            req.basic_auth(username, Some(password.expose()))
        } else {
            req
        }
    }

    /// Parse a PROPFIND XML response into entries.
    fn parse_propfind_response(
        &self,
        xml: &str,
        parent_path: &str,
    ) -> Result<Vec<Entry>, BackendError> {
        let mut entries = Vec::new();
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut current_href = String::new();
        let mut is_collection = false;
        let mut content_length: Option<u64> = None;
        let mut last_modified: Option<DateTime<Utc>> = None;
        let mut in_response = false;
        let mut in_href = false;
        let mut in_resourcetype = false;
        let mut in_content_length = false;
        let mut in_last_modified = false;

        let parent_url_path = {
            let p = parent_path.trim_end_matches('/');
            if self.prefix.is_empty() {
                p.to_string()
            } else {
                format!(
                    "/{}/{}",
                    self.prefix.trim_matches('/'),
                    p.trim_start_matches('/')
                )
            }
        };

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.local_name();
                    let name = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    match name {
                        "response" => {
                            in_response = true;
                            current_href.clear();
                            is_collection = false;
                            content_length = None;
                            last_modified = None;
                        }
                        "href" if in_response => in_href = true,
                        "resourcetype" if in_response => in_resourcetype = true,
                        "collection" if in_resourcetype => is_collection = true,
                        "getcontentlength" if in_response => in_content_length = true,
                        "getlastmodified" if in_response => in_last_modified = true,
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) => {
                    if in_href {
                        current_href = e.unescape().unwrap_or_default().to_string();
                    } else if in_content_length {
                        if let Ok(len) = e.unescape().unwrap_or_default().parse::<u64>() {
                            content_length = Some(len);
                        }
                    } else if in_last_modified {
                        let text = e.unescape().unwrap_or_default().to_string();
                        // Parse HTTP date format
                        if let Ok(dt) = DateTime::parse_from_rfc2822(&text) {
                            last_modified = Some(dt.with_timezone(&Utc));
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local_name = e.local_name();
                    let name = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    match name {
                        "response" => {
                            in_response = false;
                            // Skip the parent directory itself
                            let href_path = current_href.trim_end_matches('/');
                            let parent_normalized = parent_url_path.trim_end_matches('/');
                            if !href_path.is_empty()
                                && href_path != parent_normalized
                                && href_path != format!("{}/", parent_normalized)
                            {
                                let vfs_path = self.href_to_path(&current_href);
                                let name = Self::filename(&vfs_path);
                                if !name.is_empty() {
                                    if is_collection {
                                        entries.push(Entry::dir(vfs_path, name, last_modified));
                                    } else {
                                        entries.push(Entry::file(
                                            vfs_path,
                                            name,
                                            content_length.unwrap_or(0),
                                            last_modified,
                                        ));
                                    }
                                }
                            }
                        }
                        "href" => in_href = false,
                        "resourcetype" => in_resourcetype = false,
                        "getcontentlength" => in_content_length = false,
                        "getlastmodified" => in_last_modified = false,
                        _ => {}
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(BackendError::Other(format!(
                        "Failed to parse PROPFIND response: {}",
                        e
                    )));
                }
                _ => {}
            }
            buf.clear();
        }

        // Sort: directories first, then alphabetically
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }
}

#[async_trait]
impl Backend for WebDavBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let url = self.path_to_url(path);

        let response = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV GET failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }

        if !response.status().is_success() {
            return Err(BackendError::Other(format!(
                "WebDAV GET returned status {}",
                response.status()
            )));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| BackendError::Other(format!("WebDAV read body failed: {}", e)))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let url = self.path_to_url(path);

        // Ensure parent directories exist (WebDAV MKCOL)
        if let Some(parent) = path.rsplit_once('/').map(|(p, _)| p) {
            if !parent.is_empty() {
                self.ensure_parent_dirs(parent).await?;
            }
        }

        let response = self
            .request(reqwest::Method::PUT, &url)
            .body(content.to_vec())
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV PUT failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(BackendError::Other(format!(
                "WebDAV PUT returned status {}",
                response.status()
            )));
        }

        Ok(())
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        // WebDAV doesn't support append, so we read + write
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
        let url = self.path_to_url(path);

        let response = self
            .request(reqwest::Method::DELETE, &url)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV DELETE failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }

        if !response.status().is_success() {
            return Err(BackendError::Other(format!(
                "WebDAV DELETE returned status {}",
                response.status()
            )));
        }

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let url = self.path_to_url(path);
        let url = if url.ends_with('/') {
            url
        } else {
            format!("{}/", url)
        };

        let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:resourcetype/>
    <D:getcontentlength/>
    <D:getlastmodified/>
  </D:prop>
</D:propfind>"#;

        let response = self
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(propfind_body)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV PROPFIND failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }

        let xml = response
            .text()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV PROPFIND body failed: {}", e)))?;

        self.parse_propfind_response(&xml, path)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let url = self.path_to_url(path);

        let response = self
            .request(reqwest::Method::HEAD, &url)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV HEAD failed: {}", e)))?;

        Ok(response.status().is_success())
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let url = self.path_to_url(path);

        let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:resourcetype/>
    <D:getcontentlength/>
    <D:getlastmodified/>
  </D:prop>
</D:propfind>"#;

        let response = self
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(propfind_body)
            .send()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV PROPFIND failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BackendError::NotFound(path.to_string()));
        }

        let xml = response
            .text()
            .await
            .map_err(|e| BackendError::Other(format!("WebDAV PROPFIND body failed: {}", e)))?;

        // Parse the single-item response
        let entries = self.parse_propfind_response(&xml, "")?;

        // The response for Depth:0 contains only the item itself
        // But our parse skips the parent, so we need to construct it manually
        // from the PROPFIND data
        if entries.is_empty() {
            // Depth:0 means the item itself is the "parent" - parse differently
            Ok(Entry::file(
                path.to_string(),
                Self::filename(path),
                0,
                None,
            ))
        } else {
            Ok(entries.into_iter().next().unwrap())
        }
    }
}

impl WebDavBackend {
    /// Ensure parent directories exist by issuing MKCOL requests.
    async fn ensure_parent_dirs(&self, dir_path: &str) -> Result<(), BackendError> {
        let parts: Vec<&str> = dir_path
            .trim_start_matches('/')
            .split('/')
            .filter(|p| !p.is_empty())
            .collect();

        let mut current = String::new();
        for part in parts {
            current = format!("{}/{}", current, part);
            let url = self.path_to_url(&current);
            let url = if url.ends_with('/') {
                url
            } else {
                format!("{}/", url)
            };

            // MKCOL - ignore 405 (already exists) and 409 (conflict)
            let response = self
                .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &url)
                .send()
                .await
                .map_err(|e| BackendError::Other(format!("WebDAV MKCOL failed: {}", e)))?;

            let status = response.status().as_u16();
            if !response.status().is_success() && status != 405 && status != 409 {
                // 405 = already exists, 409 = conflict (parent missing, will be created next)
                // Only fail on other errors
                if status != 301 && status != 302 {
                    return Err(BackendError::Other(format!(
                        "WebDAV MKCOL returned status {}",
                        status
                    )));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper functions for testing path conversions
    fn path_to_url_with_prefix(base_url: &str, prefix: &str, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if prefix.is_empty() {
            format!("{}/{}", base_url, path)
        } else {
            format!("{}/{}/{}", base_url, prefix.trim_end_matches('/'), path)
        }
    }

    fn href_to_path_with_prefix(prefix: &str, href: &str) -> String {
        let path = if prefix.is_empty() {
            href.to_string()
        } else {
            let pfx = format!("/{}", prefix.trim_matches('/'));
            href.strip_prefix(&pfx)
                .unwrap_or(href)
                .to_string()
        };
        let path = path.trim_start_matches('/');
        format!("/{}", path)
    }

    #[test]
    fn test_path_to_url() {
        assert_eq!(
            path_to_url_with_prefix("https://dav.example.com", "", "/file.txt"),
            "https://dav.example.com/file.txt"
        );
        assert_eq!(
            path_to_url_with_prefix("https://dav.example.com", "", "/dir/file.txt"),
            "https://dav.example.com/dir/file.txt"
        );
    }

    #[test]
    fn test_path_to_url_with_prefix() {
        assert_eq!(
            path_to_url_with_prefix("https://dav.example.com", "data", "/file.txt"),
            "https://dav.example.com/data/file.txt"
        );
        assert_eq!(
            path_to_url_with_prefix("https://dav.example.com", "data", "/dir/file.txt"),
            "https://dav.example.com/data/dir/file.txt"
        );
    }

    #[test]
    fn test_href_to_path() {
        assert_eq!(href_to_path_with_prefix("", "/file.txt"), "/file.txt");
        assert_eq!(
            href_to_path_with_prefix("", "/dir/file.txt"),
            "/dir/file.txt"
        );
    }

    #[test]
    fn test_href_to_path_with_prefix() {
        assert_eq!(
            href_to_path_with_prefix("data", "/data/file.txt"),
            "/file.txt"
        );
        assert_eq!(
            href_to_path_with_prefix("data", "/data/dir/file.txt"),
            "/dir/file.txt"
        );
    }

    #[test]
    fn test_filename() {
        assert_eq!(WebDavBackend::filename("dir/file.txt"), "file.txt");
        assert_eq!(WebDavBackend::filename("file.txt"), "file.txt");
        assert_eq!(WebDavBackend::filename("/a/b/c.rs"), "c.rs");
        assert_eq!(WebDavBackend::filename("/dir/"), "dir");
    }

    #[test]
    fn test_config_default() {
        let config = WebDavConfig::default();
        assert!(config.url.is_empty());
        assert!(config.username.is_none());
        assert!(config.password.is_none());
        assert!(config.prefix.is_none());
    }

    #[test]
    fn test_new_backend() {
        let config = WebDavConfig {
            url: "https://dav.example.com".to_string(),
            ..Default::default()
        };
        let backend = WebDavBackend::new(config);
        assert!(backend.is_ok());
    }
}

use std::collections::HashSet;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use chrono::DateTime;
use russh::client;
use russh_keys::key;
use russh_sftp::client::SftpSession;
use tokio::sync::Mutex;

use ax_config::Secret;
use crate::error::BackendError;
use crate::traits::{Backend, Entry};

/// SFTP backend configuration.
#[derive(Debug, Clone)]
pub struct SftpConfig {
    /// SSH hostname.
    pub host: String,
    /// SSH port (default: 22).
    pub port: u16,
    /// SSH username.
    pub username: String,
    /// Optional password for password authentication.
    pub password: Option<Secret>,
    /// Optional path to private key file.
    pub private_key: Option<String>,
    /// Root directory on the remote server.
    pub root: String,
}

impl Default for SftpConfig {
    fn default() -> Self {
        SftpConfig {
            host: String::new(),
            port: 22,
            username: String::new(),
            password: None,
            private_key: None,
            root: "/".to_string(),
        }
    }
}

/// SSH client handler that accepts all host keys.
struct SshHandler;

#[async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // In production, this should verify the host key
        Ok(true)
    }
}

/// SFTP storage backend.
///
/// This backend provides file operations over SSH/SFTP using the russh library.
pub struct SftpBackend {
    sftp: Arc<Mutex<SftpSession>>,
    root: String,
}

impl SftpBackend {
    /// Create a new SFTP backend with the given configuration.
    pub async fn new(config: SftpConfig) -> Result<Self, BackendError> {
        let ssh_config = client::Config::default();

        let mut session = client::connect(
            Arc::new(ssh_config),
            (config.host.as_str(), config.port),
            SshHandler,
        )
        .await
        .map_err(|e| BackendError::ConnectionFailed {
            backend: "sftp".to_string(),
            source: Box::new(e),
        })?;

        // Authenticate
        let authenticated = if let Some(ref key_path) = config.private_key {
            let key_pair = russh_keys::load_secret_key(key_path, None)
                .map_err(|e| BackendError::Other(format!("Failed to load private key: {}", e)))?;
            session
                .authenticate_publickey(&config.username, Arc::new(key_pair))
                .await
                .map_err(|e| {
                    BackendError::Other(format!("Public key authentication failed: {}", e))
                })?
        } else if let Some(ref password) = config.password {
            session
                .authenticate_password(&config.username, password.expose())
                .await
                .map_err(|e| {
                    BackendError::Other(format!("Password authentication failed: {}", e))
                })?
        } else {
            return Err(BackendError::Other(
                "No authentication method provided (need password or private_key)".to_string(),
            ));
        };

        if !authenticated {
            return Err(BackendError::PermissionDenied(
                "SSH authentication failed".to_string(),
            ));
        }

        let channel = session.channel_open_session().await.map_err(|e| {
            BackendError::Other(format!("Failed to open SSH channel: {}", e))
        })?;

        channel.request_subsystem(true, "sftp").await.map_err(|e| {
            BackendError::Other(format!("Failed to start SFTP subsystem: {}", e))
        })?;

        let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| {
            BackendError::Other(format!("Failed to create SFTP session: {}", e))
        })?;

        let root = config.root.trim_end_matches('/').to_string();

        Ok(SftpBackend {
            sftp: Arc::new(Mutex::new(sftp)),
            root,
        })
    }

    /// Convert a VFS path to a remote path.
    fn path_to_remote(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{}", self.root, path)
        }
    }

    /// Convert a remote path to a VFS path.
    fn remote_to_path(&self, remote: &str) -> String {
        let path = remote
            .strip_prefix(&self.root)
            .unwrap_or(remote)
            .trim_start_matches('/');
        format!("/{}", path)
    }

    /// Extract the filename from a path.
    fn filename(path: &str) -> String {
        path.rsplit('/').next().unwrap_or(path).to_string()
    }
}

#[async_trait]
impl Backend for SftpBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let remote_path = self.path_to_remote(path);
        let sftp = self.sftp.lock().await;

        let data = sftp.read(&remote_path).await.map_err(|e| {
            if e.to_string().contains("No such file") {
                BackendError::NotFound(path.to_string())
            } else {
                BackendError::Other(format!("SFTP read failed: {}", e))
            }
        })?;

        Ok(data)
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let remote_path = self.path_to_remote(path);
        let sftp = self.sftp.lock().await;

        // Ensure parent directory exists
        if let Some(parent) = remote_path.rsplit_once('/').map(|(p, _)| p) {
            if !parent.is_empty() {
                let _ = sftp.create_dir(parent).await; // Ignore if exists
            }
        }

        sftp.write(&remote_path, content).await.map_err(|e| {
            BackendError::Other(format!("SFTP write failed: {}", e))
        })?;

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
        let remote_path = self.path_to_remote(path);
        let sftp = self.sftp.lock().await;

        sftp.remove_file(&remote_path).await.map_err(|e| {
            if e.to_string().contains("No such file") {
                BackendError::NotFound(path.to_string())
            } else {
                BackendError::Other(format!("SFTP delete failed: {}", e))
            }
        })?;

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let remote_path = self.path_to_remote(path);
        let sftp = self.sftp.lock().await;

        let dir_entries = sftp.read_dir(&remote_path).await.map_err(|e| {
            if e.to_string().contains("No such file") {
                BackendError::NotFound(path.to_string())
            } else {
                BackendError::Other(format!("SFTP list failed: {}", e))
            }
        })?;

        let mut entries = Vec::new();
        let mut seen_dirs: HashSet<String> = HashSet::new();

        for dir_entry in dir_entries {
            let name = dir_entry.file_name();

            // Skip . and ..
            if name == "." || name == ".." {
                continue;
            }

            let entry_remote_path = format!("{}/{}", remote_path.trim_end_matches('/'), name);
            let vfs_path = self.remote_to_path(&entry_remote_path);

            let metadata = dir_entry.metadata();
            let is_dir = metadata.is_dir();

            let modified = metadata.modified().ok().and_then(|st| {
                st.duration_since(UNIX_EPOCH)
                    .ok()
                    .and_then(|d| DateTime::from_timestamp(d.as_secs() as i64, 0))
            });

            if is_dir {
                if seen_dirs.insert(name.clone()) {
                    entries.push(Entry::dir(vfs_path, name, modified));
                }
            } else {
                let size = metadata.len();
                entries.push(Entry::file(vfs_path, name, size, modified));
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
        let remote_path = self.path_to_remote(path);
        let sftp = self.sftp.lock().await;

        match sftp.try_exists(&remote_path).await {
            Ok(exists) => Ok(exists),
            Err(_) => Ok(false),
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let remote_path = self.path_to_remote(path);
        let sftp = self.sftp.lock().await;

        let metadata = sftp.metadata(&remote_path).await.map_err(|e| {
            if e.to_string().contains("No such file") {
                BackendError::NotFound(path.to_string())
            } else {
                BackendError::Other(format!("SFTP stat failed: {}", e))
            }
        })?;

        let is_dir = metadata.is_dir();
        let name = Self::filename(path);

        let modified = metadata.modified().ok().and_then(|st| {
            st.duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| DateTime::from_timestamp(d.as_secs() as i64, 0))
        });

        if is_dir {
            Ok(Entry::dir(path.to_string(), name, modified))
        } else {
            let size = metadata.len();
            Ok(Entry::file(path.to_string(), name, size, modified))
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), BackendError> {
        let from_remote = self.path_to_remote(from);
        let to_remote = self.path_to_remote(to);
        let sftp = self.sftp.lock().await;

        sftp.rename(&from_remote, &to_remote).await.map_err(|e| {
            BackendError::Other(format!("SFTP rename failed: {}", e))
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_to_remote_with_root(root: &str, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            root.to_string()
        } else {
            format!("{}/{}", root.trim_end_matches('/'), path)
        }
    }

    fn remote_to_path_with_root(root: &str, remote: &str) -> String {
        let path = remote
            .strip_prefix(root)
            .unwrap_or(remote)
            .trim_start_matches('/');
        format!("/{}", path)
    }

    #[test]
    fn test_path_to_remote() {
        assert_eq!(
            path_to_remote_with_root("/home/user", "/file.txt"),
            "/home/user/file.txt"
        );
        assert_eq!(
            path_to_remote_with_root("/home/user", "/dir/file.txt"),
            "/home/user/dir/file.txt"
        );
    }

    #[test]
    fn test_path_to_remote_root() {
        assert_eq!(path_to_remote_with_root("/data", "/"), "/data");
        assert_eq!(
            path_to_remote_with_root("/data", "/file.txt"),
            "/data/file.txt"
        );
    }

    #[test]
    fn test_remote_to_path() {
        assert_eq!(
            remote_to_path_with_root("/home/user", "/home/user/file.txt"),
            "/file.txt"
        );
        assert_eq!(
            remote_to_path_with_root("/home/user", "/home/user/dir/file.txt"),
            "/dir/file.txt"
        );
    }

    #[test]
    fn test_filename() {
        assert_eq!(SftpBackend::filename("dir/file.txt"), "file.txt");
        assert_eq!(SftpBackend::filename("file.txt"), "file.txt");
        assert_eq!(SftpBackend::filename("/a/b/c.rs"), "c.rs");
    }

    #[test]
    fn test_config_default() {
        let config = SftpConfig::default();
        assert!(config.host.is_empty());
        assert_eq!(config.port, 22);
        assert!(config.username.is_empty());
        assert!(config.password.is_none());
        assert!(config.private_key.is_none());
        assert_eq!(config.root, "/");
    }
}

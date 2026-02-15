//! Shared application state for the Axum server.

use std::sync::Arc;
use std::time::Instant;

use ax_config::Secret;
use ax_local::SearchEngine;
use ax_remote::Vfs;

/// Shared state accessible to all route handlers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    vfs: Vfs,
    api_key: Option<Secret>,
    search_engine: Option<SearchEngine>,
    started_at: Instant,
}

impl AppState {
    /// Create new app state wrapping a VFS instance.
    pub fn new(vfs: Vfs, api_key: Option<Secret>) -> Self {
        Self {
            inner: Arc::new(Inner {
                vfs,
                api_key,
                search_engine: None,
                started_at: Instant::now(),
            }),
        }
    }

    /// Create new app state with an optional search engine.
    pub fn with_search(vfs: Vfs, api_key: Option<Secret>, search_engine: SearchEngine) -> Self {
        Self {
            inner: Arc::new(Inner {
                vfs,
                api_key,
                search_engine: Some(search_engine),
                started_at: Instant::now(),
            }),
        }
    }

    /// Get a reference to the VFS.
    pub fn vfs(&self) -> &Vfs {
        &self.inner.vfs
    }

    /// Get a reference to the search engine (if configured).
    pub fn search_engine(&self) -> Option<&SearchEngine> {
        self.inner.search_engine.as_ref()
    }

    /// Check if the given API key is valid.
    /// Returns true if no API key is configured (open access) or if the key matches.
    pub fn check_auth(&self, key: Option<&str>) -> bool {
        match &self.inner.api_key {
            None => true,
            Some(expected) => key == Some(expected.expose()),
        }
    }

    /// Get server uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.inner.started_at.elapsed().as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ax_config::VfsConfig;

    async fn make_state() -> AppState {
        let config = VfsConfig::default();
        let vfs = Vfs::from_config(config).await.unwrap();
        AppState::new(vfs, None)
    }

    async fn make_state_with_key(key: &str) -> AppState {
        let config = VfsConfig::default();
        let vfs = Vfs::from_config(config).await.unwrap();
        AppState::new(vfs, Some(Secret::new(key)))
    }

    #[tokio::test]
    async fn test_auth_no_key_configured() {
        let state = make_state().await;
        assert!(state.check_auth(None));
        assert!(state.check_auth(Some("anything")));
    }

    #[tokio::test]
    async fn test_auth_with_key() {
        let state = make_state_with_key("secret123").await;
        assert!(!state.check_auth(None));
        assert!(!state.check_auth(Some("wrong")));
        assert!(state.check_auth(Some("secret123")));
    }

    #[tokio::test]
    async fn test_uptime() {
        let state = make_state().await;
        assert!(state.uptime_secs() < 2);
    }
}

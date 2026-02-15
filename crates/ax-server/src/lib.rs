#![recursion_limit = "512"]
//! AX REST API server.
//!
//! Exposes AX VFS operations over HTTP using Axum.
//! Endpoints: /health, /status, /read, /write, /ls, /delete, /stat, /search, /grep,
//! /append, /exists, /rename, /copy

mod handlers;
mod routes;
mod state;

use ax_config::Secret;

pub use routes::build_router;
pub use state::AppState;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to bind to.
    pub port: u16,
    /// Optional API key for authentication.
    pub api_key: Option<Secret>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 19557,
            api_key: None,
        }
    }
}

/// Start the AX REST API server with graceful shutdown on SIGTERM/SIGINT.
pub async fn serve(
    vfs: ax_remote::Vfs,
    config: ServerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    if config.api_key.is_none() && !is_loopback_host(&config.host) {
        return Err("Refusing to start without an API key on a non-local host".into());
    }

    let state = AppState::new(vfs, config.api_key);
    let app = build_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("AX server listening on http://{}", addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("AX server shut down gracefully");
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Wait for a shutdown signal (SIGINT or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("Received SIGINT, shutting down..."); }
        _ = terminate => { tracing::info!("Received SIGTERM, shutting down..."); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 19557);
        assert!(config.api_key.is_none());
    }
}

use ax_config::Secret;
use ax_remote::Vfs;
use ax_server::ServerConfig;

pub async fn run(
    vfs: Vfs,
    host: &str,
    port: u16,
    api_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig {
        host: host.to_string(),
        port,
        api_key: api_key.map(Secret::from),
    };

    eprintln!("Starting AX REST API server on http://{}:{}", host, port);
    eprintln!("Endpoints: /health, /status, /read, /write, /delete, /stat, /ls, /grep, /search");

    ax_server::serve(vfs, config).await
}

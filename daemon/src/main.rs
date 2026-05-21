mod ai;
mod api;
mod core;
mod sites;

use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:17891";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let bind_addr =
        std::env::var("PAIRPILOT_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.into());
    let addr: SocketAddr = bind_addr.parse()?;

    let listener = TcpListener::bind(addr).await?;
    eprintln!("pairpilot-daemon listening on http://{addr}");
    if api::captured_content_logging_enabled() {
        eprintln!("pairpilot-daemon logging captured content to stdout as JSONL");
    } else {
        eprintln!("pairpilot-daemon captured content stdout logging disabled");
    }

    axum::serve(listener, api::router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

mod ai;
mod api;
mod core;
mod sites;
mod storage;

use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:17891";
const BIND_ADDR_ENV: &str = "WEBLAYER_BIND_ADDR";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .with_writer(std::io::stdout)
        .init();

    let bind_addr = std::env::var(BIND_ADDR_ENV).unwrap_or_else(|_| DEFAULT_BIND_ADDR.into());
    let addr: SocketAddr = bind_addr.parse()?;

    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "weblayer-daemon listening");
    if api::captured_content_logging_enabled() {
        info!("weblayer-daemon captured content logging enabled");
    } else {
        info!("weblayer-daemon captured content logging disabled");
    }

    axum::serve(listener, api::router()?)
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

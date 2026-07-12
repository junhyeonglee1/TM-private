use std::{error::Error, io};

use tm_core::{TmCore, TmHome};
use tm_server::{ServerConfig, build_router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_ansi(false)
        .try_init();

    let config = ServerConfig::from_env().map_err(io::Error::other)?;
    let core = TmCore::open(TmHome::new(&config.home))?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;

    tracing::info!(
        bind = %config.bind_addr,
        home = %config.home.display(),
        "tm-server is listening"
    );

    axum::serve(listener, build_router(core))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for the shutdown signal");
    }
}

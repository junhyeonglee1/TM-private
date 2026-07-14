use std::{error::Error, io};

use tm_core::{TmCore, TmHome};
use tm_server::{
    ServerConfig, ServerProfile, build_cloud_bootstrap_router, build_router_with_openai,
    openai::OpenAiClient,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_ansi(false)
        .try_init();

    let config = ServerConfig::from_env().map_err(io::Error::other)?;
    let core = TmCore::open(TmHome::new(&config.home))?;
    let database_initialized_at = core.database_initialized_at()?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;

    tracing::info!(
        profile = %config.profile,
        bind = %config.bind_addr,
        home = %config.home.display(),
        database_initialized_at,
        "tm-server is listening"
    );

    let router = match config.profile {
        ServerProfile::Local => {
            let openai = OpenAiClient::new(config.openai).map_err(io::Error::other)?;
            build_router_with_openai(core, openai)
        }
        ServerProfile::CloudBootstrap => build_cloud_bootstrap_router(core),
    };

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(terminate) => terminate,
            Err(error) => {
                tracing::error!(%error, "failed to listen for the SIGTERM shutdown signal");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::error!(%error, "failed to listen for the Ctrl+C shutdown signal");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for the shutdown signal");
    }
}

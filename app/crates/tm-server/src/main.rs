use std::{env, error::Error, io};

use tm_core::{TmCore, TmHome};
use tm_server::{
    MaintenanceMode, ServerConfig, ServerProfile, build_cloud_authenticated_router_with_openai,
    build_cloud_bootstrap_router, build_cloud_import_router, build_router_with_openai,
    openai::OpenAiClient,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    init_tracing();

    let config = ServerConfig::from_env().map_err(io::Error::other)?;
    let core = TmCore::open(TmHome::new(&config.home))?;
    let database_initialized_at = core.database_initialized_at()?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;

    tracing::info!(
        profile = %config.profile,
        bind = %config.bind_addr,
        database_initialized_at,
        "tm-server is listening"
    );

    let router = match config.profile {
        ServerProfile::Local => {
            let openai = OpenAiClient::new(config.openai).map_err(io::Error::other)?;
            build_router_with_openai(core, openai)
        }
        ServerProfile::CloudBootstrap => build_cloud_bootstrap_router(core),
        ServerProfile::CloudAuthenticated => {
            let auth = config.auth.ok_or_else(|| {
                io::Error::other("cloud-authenticated profile requires authentication config")
            })?;
            match config.maintenance_mode {
                MaintenanceMode::Disabled => {
                    let openai = OpenAiClient::new(config.openai).map_err(io::Error::other)?;
                    build_cloud_authenticated_router_with_openai(core, auth, openai)
                }
                MaintenanceMode::Import => build_cloud_import_router(core, auth),
            }
        }
    };

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("tm_server=info"));
    if env::var_os("RAILWAY_ENVIRONMENT_ID").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_span_list(false)
            .with_target(false)
            .with_ansi(false)
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_ansi(false)
            .try_init();
    }
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

//! Startup sequence and graceful shutdown.
//!
//! Order mirrors the Go baseline: load config → init logging → connect MySQL →
//! connect Redis (with PING) → run migrations → seed the default admin → create
//! upload directories → rebuild the search index → serve.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use tokio::net::TcpListener;
use tokio::signal;

use smileyan_backend::app;
use smileyan_backend::bootstrap;
use smileyan_backend::config;
use smileyan_backend::logging;
use smileyan_backend::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Arc::new(config::load()?);

    logging::init()?;
    tracing::info!("Starting Smileyan Blog System...");

    cfg.validate()?;

    let state = Arc::new(AppState::connect(cfg.clone()).await?);

    bootstrap::run_migrations(&state.db).await?;
    bootstrap::ensure_default_admin(&state.db).await?;
    bootstrap::create_upload_dirs(&cfg)?;

    // The Go baseline ignores indexing errors and keeps serving; a broken
    // index must not take the whole backend down.
    match bootstrap::rebuild_search_index(&state).await {
        Ok(count) => tracing::info!("Search index rebuilt with {count} posts"),
        Err(err) => tracing::error!(error = ?err, "Failed to rebuild search index"),
    }

    let router = app::build_router(state);

    let listener = TcpListener::bind(("0.0.0.0", cfg.server.port))
        .await
        .with_context(|| format!("Failed to bind port {}", cfg.server.port))?;

    tracing::info!("Server starting on :{}", cfg.server.port);

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("Server error")?;

    tracing::info!("Server stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, stopping server...");
}

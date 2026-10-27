//! `panel-api`: Binary entrypoint.

use panel_api::agent_client::AgentClient;
use panel_api::config::PanelConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use std::sync::Arc;
use tracing::{error, info};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("panel-api {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    #[cfg(unix)]
    {
        let identity = panel_core::production::system_command("id")
            .arg("-u")
            .output()?;
        if !identity.status.success() || String::from_utf8_lossy(&identity.stdout).trim() == "0" {
            return Err("panel-api must run as a non-root user".into());
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    info!("Starting Qinpanel API daemon...");

    let config = PanelConfig::load(None)?;

    info!(
        "Server listening on {}, Agent socket configured at {}",
        config.server.listen_addr,
        config.agent.socket_path.display()
    );

    let db_url = if config.database.url.starts_with("sqlite:") {
        config.database.url.clone()
    } else {
        format!("sqlite://{}", config.database.url.replace('\\', "/"))
    };
    let pool = panel_db::pool::init_pool(&db_url).await?;
    panel_db::pool::run_migrations(&pool).await?;
    let operations = panel_db::operations::open(&pool).await?;

    let lockout_manager = Arc::new(panel_api::auth::LockoutManager::new(
        config.auth.max_failed_attempts,
        config.auth.lockout_duration_secs,
    ));

    let agent_client = Arc::new(AgentClient::new(config.agent.socket_path.clone()));
    let mut state = AppState::with_operations(
        agent_client,
        pool,
        operations,
        config.server.environment.clone(),
        config.auth.clone(),
        lockout_manager,
    );
    state.analytics_db_path = config.analytics.path.clone();
    let (metrics_shutdown_tx, metrics_shutdown_rx) = tokio::sync::watch::channel(false);
    let metrics_state = state.clone();
    let metrics_handle = tokio::spawn(panel_api::telemetry::run_system_metrics_worker(
        metrics_state,
        metrics_shutdown_rx,
    ));

    // Startup recovery: mark any interrupted running jobs as failed
    match state.job_runner.recover_interrupted().await {
        Ok(count) if count > 0 => {
            info!(
                "Recovered {} interrupted running jobs from previous session",
                count
            );
        }
        Ok(_) => {}
        Err(e) => {
            error!("Failed to recover interrupted jobs: {}", e);
        }
    }

    // Spawn background job worker loop
    let (worker_shutdown_tx, worker_shutdown_rx) = tokio::sync::watch::channel(false);
    let worker_runner = Arc::clone(&state.job_runner);
    let worker_handle = tokio::spawn(async move {
        panel_api::jobs::run_worker(worker_runner, worker_shutdown_rx).await;
    });

    // Spawn background analytics retention worker loop (Phase 9 - P9-T9)
    let (retention_shutdown_tx, retention_shutdown_rx) = tokio::sync::watch::channel(false);
    let retention_pool = state.pool.clone();
    let retention_storage = Arc::clone(&state.analytics_storage);
    let retention_handle = tokio::spawn(async move {
        panel_api::retention::run_retention_worker(
            retention_pool,
            retention_storage,
            retention_shutdown_rx,
        )
        .await;
    });

    let (catalog_shutdown_tx, catalog_shutdown_rx) = tokio::sync::watch::channel(false);
    let catalog_handle = tokio::spawn(panel_api::runtime_catalog::run_worker(
        state.pool.clone(),
        catalog_shutdown_rx,
    ));
    let ssl_handle = panel_api::ssl::start_ssl_renewal_scheduler(state.clone());
    let backup_observer = panel_api::server_backup::start_observer(state.clone());
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(&config.server.listen_addr).await?;
    info!(
        "Panel API listening at http://{}",
        config.server.listen_addr
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    info!("Initiating shutdown: stopping background workers...");
    let _ = worker_shutdown_tx.send(true);
    let _ = retention_shutdown_tx.send(true);
    let _ = metrics_shutdown_tx.send(true);
    let _ = catalog_shutdown_tx.send(true);
    let _ = worker_handle.await;
    let _ = retention_handle.await;
    let _ = metrics_handle.await;
    let _ = catalog_handle.await;
    ssl_handle.abort();
    backup_observer.abort();
    let _ = ssl_handle.await;
    let _ = backup_observer.await;

    info!("Panel API shutdown gracefully.");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, initiating graceful shutdown...");
        }
        _ = terminate => {
            info!("Received terminate signal, initiating graceful shutdown...");
        }
    }
}

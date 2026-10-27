//! `panel-agent`: Binary entrypoint.

use panel_agent::{AgentConfig, AgentServer};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--fs-worker")
    {
        panel_agent::fs_worker::run(&arguments[1..])?;
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("panel-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_agent())
}

async fn run_agent() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    info!("Starting Qinpanel Agent daemon...");

    let socket_path = std::env::var("PANEL_AGENT_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/run/panel/agent.sock"));

    let output = panel_core::production::system_command("id")
        .args(["-u", "panel-api"])
        .output()?;
    let api_uid = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()?;
    if !output.status.success() || api_uid == 0 {
        return Err("panel-api must be an existing non-root service identity".into());
    }
    let config = AgentConfig {
        socket_path,
        allowed_uid: Some(api_uid),
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::new(config));

    tokio::select! {
        res = server.run(shutdown_rx) => {
            res?;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, terminating agent...");
            let _ = shutdown_tx.send(());
        }
    }

    Ok(())
}

//! Node Agent - lightweight cross-platform process manager for DAQ components
//!
//! Usage:
//!   cargo run --bin node_agent -- -f config/agent_local.toml
//!   cargo run --bin node_agent -- -f config/agent_remote.toml --start-all
//!
//! API:
//!   GET  /api/status                     - All process statuses
//!   POST /api/processes/{name}/start     - Start a process
//!   POST /api/processes/{name}/stop      - Stop a process
//!   POST /api/processes/{name}/restart   - Restart a process
//!   GET  /api/processes/{name}/logs      - Get process logs
//!   POST /api/start-all                  - Start all processes
//!   POST /api/stop-all                   - Stop all processes

use std::net::SocketAddr;

use clap::Parser;
use delila_rs::node_agent;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(
    name = "node_agent",
    about = "DELILA node agent - process manager for DAQ components"
)]
struct Args {
    /// Path to agent configuration file
    #[arg(short = 'f', long = "config", default_value = "agent.toml")]
    config_file: String,

    /// HTTP server port (overrides config)
    #[arg(short = 'p', long = "port")]
    port: Option<u16>,

    /// Start all processes immediately on agent startup
    #[arg(long = "start-all")]
    start_all: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let args = Args::parse();
    let config = node_agent::AgentFileConfig::load(&args.config_file)?;
    let port = args.port.unwrap_or(config.agent.port);

    info!(
        "Loaded {} process config(s) from {}",
        config.process.len(),
        args.config_file
    );
    for p in &config.process {
        info!(
            "  {} -> {} {:?} (auto_restart={})",
            p.name, p.command, p.args, p.auto_restart
        );
    }

    let (state, router) = node_agent::build(config);

    if args.start_all {
        info!("Starting all processes...");
        state.manager.start_all().await;
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Node Agent listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Shutting down, stopping all processes...");
    state.manager.stop_all().await;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    info!("Received Ctrl+C, shutting down...");
}

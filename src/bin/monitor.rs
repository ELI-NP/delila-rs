//! Monitor binary - real-time histogram display via web browser
//!
//! Usage:
//!   cargo run --bin monitor                       # Use config.toml
//!   cargo run --bin monitor -- -f config.toml     # Explicit config file
//!   cargo run --bin monitor -- -a tcp://localhost:5557 -p 8080

use clap::Parser;
use delila_rs::common::{setup_shutdown_with_message, MonitorArgs};
use delila_rs::config::Config;
use delila_rs::monitor::{AxisSource, HistogramConfig, Monitor, MonitorConfig};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "monitor",
    about = "DELILA monitor - real-time histogram display via web browser"
)]
struct Args {
    #[command(flatten)]
    monitor: MonitorArgs,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing (logging)
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=info".parse()?))
        .init();

    let args = Args::parse();

    // Load configuration
    let config = Config::load(&args.monitor.common.config_file)?;
    info!(config_file = %args.monitor.common.config_file, "Loaded configuration");

    let mut monitor_config = MonitorConfig::default();

    if let Some(ref monitor) = config.network.monitor {
        monitor_config.subscribe_address = monitor.subscribe.clone();
        monitor_config.http_port = monitor.http_port;
        if let Some(ref cmd) = monitor.command {
            monitor_config.command_address = cmd.clone();
        }
        // PSD histogram config from TOML
        monitor_config.psd_histogram_config = HistogramConfig {
            num_bins: monitor.psd_bins,
            min_value: monitor.psd_min,
            max_value: monitor.psd_max,
        };
        // The legacy TOML knobs `psd2d_x_bins` / `psd2d_y_bins` only ever
        // tuned the Energy and Psd axes; under the AxisSource model we plumb
        // them through `histogram2d_overrides` so users get the same effect
        // when they look at any 2D plot whose axis is Energy or Psd.
        monitor_config.histogram2d_overrides.insert(
            AxisSource::Energy,
            HistogramConfig {
                num_bins: monitor.psd2d_x_bins,
                min_value: 0.0,
                max_value: 65536.0,
            },
        );
        monitor_config.histogram2d_overrides.insert(
            AxisSource::Psd,
            HistogramConfig {
                num_bins: monitor.psd2d_y_bins,
                min_value: monitor.psd_min,
                max_value: monitor.psd_max,
            },
        );
    }

    // CLI overrides config file
    if let Some(addr) = args.monitor.address {
        monitor_config.subscribe_address = addr;
    }
    if let Some(port) = args.monitor.port {
        monitor_config.http_port = port;
    }

    // Setup shutdown handling
    let (_shutdown_tx, shutdown_rx) =
        setup_shutdown_with_message("Received Ctrl+C, shutting down...");

    // Create and run monitor
    let mut monitor = Monitor::new(monitor_config.clone()).await?;

    println!("========================================");
    println!("       DELILA Monitor Started");
    println!("========================================");
    println!();
    println!("  Subscribing to: {}", monitor_config.subscribe_address);
    println!(
        "  Web UI:         http://localhost:{}/",
        monitor_config.http_port
    );
    println!();
    println!("  Press Ctrl+C to stop.");
    println!("========================================");

    monitor.run(shutdown_rx).await?;

    println!("Monitor stopped.");
    Ok(())
}

//! Online Event Builder v2 binary
//!
//! ZMQ SUB で Merger からデータを受信し、リアルタイムでイベントビルドして ROOT ファイルに出力する。
//!
//! Usage:
//!   cargo run --features root --bin online_event_builder -- -f config.toml

use std::path::PathBuf;

use clap::Parser;
use delila_rs::common::setup_shutdown_with_message;
use delila_rs::config::Config;
use delila_rs::event_builder::online::{OnlineEventBuilder, OnlineEventBuilderConfig};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "online_event_builder",
    about = "DELILA Online Event Builder v2 - chunk-based real-time event building"
)]
struct Args {
    /// Configuration file path
    #[arg(short = 'f', long = "config", default_value = "config.toml")]
    config_file: String,

    /// Number of worker threads (event building)
    #[arg(short = 'w', long = "workers", default_value_t = 4)]
    workers: usize,

    /// Number of writer threads (parallel ROOT I/O)
    #[arg(long = "writers", default_value_t = 4)]
    writers: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=info".parse()?))
        .init();

    let args = Args::parse();

    let config = Config::load(&args.config_file)?;
    info!(config_file = %args.config_file, "Loaded configuration");

    let eb_config = if let Some(ref eb) = config.network.event_builder {
        OnlineEventBuilderConfig {
            subscribe_address: eb.subscribe.clone(),
            command_address: eb
                .command
                .clone()
                .unwrap_or_else(|| "tcp://*:5595".to_string()),
            output_dir: PathBuf::from(&eb.output_dir),
            coincidence_window_ns: eb.coincidence_window_ns,
            safe_horizon_ns: eb.buffer_delay_ns, // Use buffer_delay as safe horizon
            ch_settings_file: eb.ch_settings_file.clone(),
            time_calib_file: eb.time_calib_file.clone(),
            n_workers: args.workers,
            n_writers: args.writers,
            ..OnlineEventBuilderConfig::default()
        }
    } else {
        anyhow::bail!("No [network.event_builder] section in config file");
    };

    let (_shutdown_tx, shutdown_rx) =
        setup_shutdown_with_message("Received Ctrl+C, shutting down...");

    let eb = OnlineEventBuilder::new(eb_config.clone())?;

    println!("========================================");
    println!("  DELILA Online Event Builder v2");
    println!("========================================");
    println!();
    println!("  Subscribing to:     {}", eb_config.subscribe_address);
    println!("  Output dir:         {}", eb_config.output_dir.display());
    println!("  Coincidence window: {} ns", eb_config.coincidence_window_ns);
    println!("  Safe horizon:       {} ms", eb_config.safe_horizon_ns / 1_000_000.0);
    println!("  Workers:            {}", eb_config.n_workers);
    println!("  Writers:            {}", eb_config.n_writers);
    println!();
    println!("  Press Ctrl+C to stop.");
    println!("========================================");

    let stats = eb.run(shutdown_rx).await?;

    println!();
    println!("========================================");
    println!("  Online Event Builder Finished");
    println!("========================================");
    println!("  Received hits:    {}", stats.received_hits);
    println!("  Received batches: {}", stats.received_batches);
    println!("  Dropped batches:  {}", stats.dropped_batches);
    println!("  Chunks processed: {}", stats.chunks_processed);
    println!("  Events built:     {}", stats.events_built);
    println!("  Files written:    {}", stats.files_written);
    println!("========================================");

    Ok(())
}

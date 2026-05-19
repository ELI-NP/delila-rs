//! Online Event Builder (unified pipeline).
//!
//! Subscribes to a Merger PUB endpoint via [`ZmqHitSource`], feeds hits into
//! the shared [`EventBuilderPipeline`] (the same engine used offline), and
//! writes built events as ROOT files.
//!
//! Replaces the prior bespoke pipeline that used to live in
//! `src/event_builder/online.rs` (deleted 2026-05-19, SPEC § 11.4 Phase 5).
//!
//! Usage:
//!
//! ```text
//! cargo run --features root --bin online_event_builder -- -f config.toml
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use delila_rs::config::Config;
use delila_rs::event_builder::chunk_builder::TriggerConfig;
use delila_rs::event_builder::{
    load_channel_config, EventBuilderPipeline, PipelineConfig, TimeCalibration, TimeOffsetsFile,
    ZmqHitSource,
};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "online_event_builder",
    about = "DELILA Online Event Builder — unified pipeline (Merger PUB → ROOT)"
)]
struct Args {
    /// Configuration file path (TOML)
    #[arg(short = 'f', long = "config", default_value = "config.toml")]
    config_file: String,

    /// Number of worker threads (event building)
    #[arg(short = 'w', long = "workers", default_value_t = 4)]
    workers: usize,

    /// Number of writer threads (parallel ROOT I/O)
    #[arg(long = "writers", default_value_t = 4)]
    writers: usize,

    /// Run ID used in output filenames
    #[arg(long = "run-id", default_value_t = 0)]
    run_id: u32,
}

fn load_trigger(
    ch_settings_path: &str,
    coincidence_window_ns: f64,
) -> anyhow::Result<TriggerConfig> {
    let cfg = load_channel_config(Path::new(ch_settings_path))?;
    Ok(TriggerConfig::from_channel_config(
        &cfg,
        coincidence_window_ns,
    ))
}

/// Load a time-calibration file.
///
/// First tries the new tree-based `timeSettings.json` schema (SPEC § 4.3);
/// falls back to the legacy `TimeCalibration` JSON if the new format fails
/// to parse. Missing or unspecified file → zero offsets.
fn load_calibration(path: Option<&str>) -> TimeCalibration {
    let Some(p) = path else {
        info!("No time calibration file specified — using zero offsets");
        return TimeCalibration::new(0, 0);
    };

    match TimeOffsetsFile::load(Path::new(p)) {
        Ok(file) => match file.resolve() {
            Ok(resolved) => {
                for w in &resolved.warnings {
                    warn!(file = p, "{w}");
                }
                info!(
                    file = p,
                    roots = resolved.root_count(),
                    "Loaded timeSettings.json (tree schema)"
                );
                return resolved.into_time_calibration();
            }
            Err(e) => {
                warn!(file = p, error = %e, "Failed to resolve timeSettings.json tree");
            }
        },
        Err(_) => {
            // Either not the new schema or a real parse error — fall through
            // to the legacy loader before bailing.
        }
    }

    match TimeCalibration::from_json_file(Path::new(p)) {
        Ok(c) => {
            info!(file = p, "Loaded legacy time calibration");
            c
        }
        Err(e) => {
            warn!(file = p, error = %e, "Time calibration load failed — using zero offsets");
            TimeCalibration::new(0, 0)
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=info".parse()?))
        .init();

    let args = Args::parse();

    let cfg = Config::load(&args.config_file)?;
    info!(config_file = %args.config_file, "Loaded TOML config");

    let eb =
        cfg.network.event_builder.as_ref().ok_or_else(|| {
            anyhow::anyhow!("[network.event_builder] section missing from config")
        })?;

    // ── Build the source ────────────────────────────────────────────────
    let source = ZmqHitSource::connect(&eb.subscribe)
        .map_err(|e| anyhow::anyhow!("Failed to connect ZmqHitSource to {}: {e}", eb.subscribe))?;
    let shutdown = source.shutdown_handle();

    // ── Build the pipeline ──────────────────────────────────────────────
    let trigger = if let Some(ref path) = eb.ch_settings_file {
        load_trigger(path, eb.coincidence_window_ns)?
    } else {
        warn!("ch_settings_file not configured — empty trigger set (no events will be built)");
        TriggerConfig {
            triggers: Default::default(),
            priorities: Default::default(),
            ac_pairs: Default::default(),
            coincidence_window_ns: eb.coincidence_window_ns,
        }
    };

    let calibration = load_calibration(eb.time_calib_file.as_deref());

    let pipeline_cfg = PipelineConfig {
        safe_horizon_ns: eb.buffer_delay_ns, // reuse buffer_delay as safe horizon
        n_workers: args.workers,
        n_writers: args.writers,
        output_dir: PathBuf::from(&eb.output_dir),
        run_id: args.run_id,
        ..PipelineConfig::default()
    };

    let pipeline = EventBuilderPipeline::new(pipeline_cfg.clone(), trigger, calibration);

    println!("========================================");
    println!("  DELILA Online Event Builder");
    println!("  (unified pipeline; Merger SUB → ROOT)");
    println!("========================================");
    println!();
    println!("  Subscribe:         {}", eb.subscribe);
    println!("  Output dir:        {}", pipeline_cfg.output_dir.display());
    println!("  Coincidence:       {} ns", eb.coincidence_window_ns);
    println!(
        "  Safe horizon:      {:.1} ms",
        pipeline_cfg.safe_horizon_ns / 1.0e6
    );
    println!(
        "  Workers / writers: {} / {}",
        pipeline_cfg.n_workers, pipeline_cfg.n_writers
    );
    println!("  Run ID:            {}", pipeline_cfg.run_id);
    println!();
    println!("  Press Ctrl+C to stop.");
    println!("========================================");

    // ── Run the pipeline on a dedicated std::thread ─────────────────────
    //
    // The pipeline is fully synchronous (Sorter / Workers / Writers are
    // std::threads under the hood). We park it on a worker thread so the
    // tokio main can wait on Ctrl+C without blocking the runtime.
    let pipeline_thread = std::thread::Builder::new()
        .name("eb-pipeline-driver".to_string())
        .spawn(move || pipeline.run(source))?;

    // ── Wait for either Ctrl+C or the pipeline finishing on its own ──────
    let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                println!("\nCtrl+C received — requesting pipeline shutdown…");
                shutdown.request();
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if pipeline_thread.is_finished() {
                    break;
                }
            }
        }
    }

    // Block off the runtime while the pipeline drains and flushes.
    let stats = match tokio::task::spawn_blocking(move || pipeline_thread.join()).await {
        Ok(Ok(stats)) => stats,
        Ok(Err(e)) => anyhow::bail!("pipeline thread panicked: {e:?}"),
        Err(e) => anyhow::bail!("pipeline join task failed: {e}"),
    };

    println!();
    println!("========================================");
    println!("  Online Event Builder Finished");
    println!("========================================");
    println!("  Received hits:    {}", stats.received_hits);
    println!("  Received batches: {}", stats.received_batches);
    println!("  Chunks processed: {}", stats.chunks_processed);
    println!("  Events built:     {}", stats.events_built);
    println!("  Files written:    {}", stats.files_written);
    println!("========================================");

    Ok(())
}

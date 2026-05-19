//! Online Event Builder (unified pipeline).
//!
//! Subscribes to a Merger PUB endpoint via [`ZmqHitSource`], feeds hits into
//! the shared [`EventBuilderPipeline`] (the same engine used offline), and
//! writes built events as ROOT files.
//!
//! Replaces the prior bespoke pipeline that used to live in
//! `src/event_builder/online.rs` (deleted 2026-05-19, SPEC § 11.4 Phase 5).
//!
//! # Config files
//!
//! Driven entirely by the three SPEC-defined files (TOML config provides
//! the *paths*):
//!
//! - `eb_config.json`   — L1 / L2 named-ops + timing (SPEC § 4.1)
//! - `chSettings.json`  — per-channel tags + calibration (SPEC § 4.2)
//! - `timeSettings.json`— tree time-offsets (SPEC § 4.3, optional)
//!
//! Usage:
//!
//! ```text
//! cargo run --features root --bin online_event_builder -- -f config.toml
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use delila_rs::config::Config;
use delila_rs::event_builder::{
    load_channel_config, ChannelConfig, ChannelTagMap, EbRuntimeConfig, EventBuilderPipeline,
    L2Filter, PipelineConfig, TimeCalibration, TimeOffsetsFile, ZmqHitSource,
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

/// Build a `(module, channel) → tags` lookup from chSettings.
fn build_tag_map(cfg: &ChannelConfig) -> ChannelTagMap {
    let mut m: ChannelTagMap = HashMap::new();
    for module_chs in cfg {
        for ch in module_chs {
            m.insert((ch.module, ch.channel), ch.tags.clone());
        }
    }
    m
}

/// Load tree-based timeSettings.json; fall back to legacy single-ref schema
/// then to zero offsets on parse failure. Missing path → zero offsets.
fn load_calibration(path: Option<&str>) -> TimeCalibration {
    let Some(p) = path else {
        info!("No time calibration file specified — using zero offsets");
        return TimeCalibration::new(0, 0);
    };

    if let Ok(file) = TimeOffsetsFile::load(Path::new(p)) {
        match file.resolve() {
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

    // ── Load the EB runtime config (eb_config.json) ──────────────────────
    //
    // Required since Phase J — the old chSettings.is_event_trigger path is
    // gone; trigger / AC / threshold semantics live entirely in L1/L2.
    let eb_config_path = eb.eb_config_file.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "[network.event_builder].eb_config_file is required (eb_config.json — SPEC § 4.1)"
        )
    })?;
    let eb_runtime = EbRuntimeConfig::load(Path::new(eb_config_path))
        .with_context(|| format!("Failed to load eb_config.json from {eb_config_path}"))?;
    info!(file = eb_config_path, "Loaded eb_config.json");

    let trigger = eb_runtime
        .build_trigger_config()
        .context("Failed to derive TriggerConfig from eb_config.l1")?;

    // ── Load the channel descriptor (chSettings.json) — tags for L2 ──────
    let ch_tags = if let Some(ref path) = eb.ch_settings_file {
        let cfg = load_channel_config(Path::new(path))
            .with_context(|| format!("Failed to load chSettings.json from {path}"))?;
        build_tag_map(&cfg)
    } else {
        warn!("ch_settings_file not configured — L2 counter ops will match nothing (no tag map)");
        ChannelTagMap::new()
    };

    let l2_filter =
        L2Filter::new(eb_runtime.l2.clone(), ch_tags).context("Failed to construct L2 filter")?;

    let calibration = load_calibration(eb.time_calib_file.as_deref());

    let pipeline_cfg = PipelineConfig {
        safe_horizon_ns: eb.buffer_delay_ns, // reuse buffer_delay as safe horizon
        n_workers: args.workers,
        n_writers: args.writers,
        output_dir: PathBuf::from(&eb.output_dir),
        run_id: args.run_id,
        zmq_pub_endpoint: eb.zmq_pub_endpoint.clone(),
        ..PipelineConfig::default()
    };

    let pipeline = EventBuilderPipeline::new(pipeline_cfg.clone(), trigger, calibration)
        .with_l2_filter(l2_filter);

    // ── Build the source ────────────────────────────────────────────────
    let source = ZmqHitSource::connect(&eb.subscribe)
        .map_err(|e| anyhow::anyhow!("Failed to connect ZmqHitSource to {}: {e}", eb.subscribe))?;
    let shutdown = source.shutdown_handle();

    println!("========================================");
    println!("  DELILA Online Event Builder");
    println!("  (unified pipeline; Merger SUB → ROOT)");
    println!("========================================");
    println!();
    println!("  Subscribe:         {}", eb.subscribe);
    println!("  Output dir:        {}", pipeline_cfg.output_dir.display());
    println!("  eb_config:         {eb_config_path}");
    println!(
        "  Coincidence:       {} ns",
        eb_runtime.timing.coincidence_window_ns
    );
    println!(
        "  Safe horizon:      {:.1} ms",
        pipeline_cfg.safe_horizon_ns / 1.0e6
    );
    println!(
        "  Workers / writers: {} / {}",
        pipeline_cfg.n_workers, pipeline_cfg.n_writers
    );
    println!("  Run ID:            {}", pipeline_cfg.run_id);
    if let Some(ref ep) = pipeline_cfg.zmq_pub_endpoint {
        println!("  EB-Monitor PUB:    {ep}");
    } else {
        println!("  EB-Monitor PUB:    disabled");
    }
    println!();
    println!("  Press Ctrl+C to stop.");
    println!("========================================");

    // ── Run the pipeline on a dedicated std::thread ─────────────────────
    let pipeline_thread = std::thread::Builder::new()
        .name("eb-pipeline-driver".to_string())
        .spawn(move || pipeline.run(source))?;

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
    println!("  Events kept (L2): {}", stats.events_kept);
    println!("  Files written:    {}", stats.files_written);
    println!("  Batches published: {}", stats.batches_published);
    println!("========================================");

    Ok(())
}

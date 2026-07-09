//! Online Event Builder (unified pipeline).
//!
//! Subscribes to a Merger PUB endpoint via [`ZmqHitSource`], feeds hits into
//! the shared [`EventBuilderPipeline`] (the same engine used offline), and
//! writes built events as ROOT files.
//!
//! Replaces the prior bespoke pipeline that used to live in
//! `src/event_builder/online.rs` (deleted 2026-05-19, SPEC § 11.4 Phase 5).
//!
//! # Modes
//!
//! Two operating modes are auto-selected based on the TOML config:
//!
//! * **Operator-managed** — when `[network.event_builder].command` is set,
//!   the binary binds a REP socket there and obeys the standard 5-state
//!   machine. The pipeline thread is spawned on `Start` and torn down on
//!   `Stop`/`Reset`. This is what the production replay stack uses.
//! * **Standalone** — when `command` is not set, the pipeline starts
//!   immediately and runs until Ctrl+C. Backward-compatible with the
//!   pre-Phase-4 invocation in scripts.
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
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use delila_rs::common::{
    handle_command, run_command_task, CommandHandlerExt, ComponentSharedState, ComponentState,
};
use delila_rs::config::Config;
use delila_rs::event_builder::{
    load_channel_config, ChannelConfig, ChannelTagMap, EbRuntimeConfig, EventBuilderPipeline,
    L2Filter, PipelineConfig, PipelineStats, TimeCalibration, TimeOffsetsFile, ZmqHitSource,
};
use tokio::sync::{watch, Mutex};
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

    /// Run ID used in output filenames. In Operator-managed mode this is
    /// overridden by the `run_number` carried in the Start command.
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

/// All inputs required to build a fresh [`EventBuilderPipeline`] on each
/// `Start`. Cloned per-run so the pipeline thread owns its own copies.
#[derive(Clone)]
struct PipelineInputs {
    eb_runtime: Arc<EbRuntimeConfig>,
    ch_tags: ChannelTagMap,
    calibration: TimeCalibration,
    cfg_template: PipelineConfig,
    subscribe: String,
}

impl PipelineInputs {
    fn build(&self, run_id: u32) -> anyhow::Result<(EventBuilderPipeline, ZmqHitSource)> {
        let trigger = self
            .eb_runtime
            .build_trigger_config()
            .context("Failed to derive TriggerConfig from eb_config.l1")?;
        let l2 = L2Filter::new(self.eb_runtime.l2.clone(), self.ch_tags.clone())
            .context("Failed to construct L2 filter")?;
        let cfg = PipelineConfig {
            run_id,
            ..self.cfg_template.clone()
        };
        let pipeline =
            EventBuilderPipeline::new(cfg, trigger, self.calibration.clone()).with_l2_filter(l2);
        let source = ZmqHitSource::connect(&self.subscribe).map_err(|e| {
            anyhow::anyhow!("Failed to connect ZmqHitSource to {}: {e}", self.subscribe)
        })?;
        Ok((pipeline, source))
    }
}

/// CommandHandlerExt for the Operator-managed Event Builder.
///
/// Hooks are intentionally minimal — all the heavy lifting (build/spawn/join
/// the pipeline thread) is done by the main async task that watches the
/// `state_rx` channel. This keeps the command task synchronous and fast.
struct EbCommandExt;

impl CommandHandlerExt for EbCommandExt {
    fn component_name(&self) -> &'static str {
        "OnlineEB"
    }
    // All on_* hooks fall back to the default no-op.
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
    let eb_config_path = eb.eb_config_file.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "[network.event_builder].eb_config_file is required (eb_config.json — SPEC § 4.1)"
        )
    })?;
    let eb_runtime = EbRuntimeConfig::load(Path::new(eb_config_path))
        .with_context(|| format!("Failed to load eb_config.json from {eb_config_path}"))?;
    info!(file = eb_config_path, "Loaded eb_config.json");

    // ── Load the channel descriptor (chSettings.json) — tags for L2 ──────
    let ch_tags = if let Some(ref path) = eb.ch_settings_file {
        let cfg = load_channel_config(Path::new(path))
            .with_context(|| format!("Failed to load chSettings.json from {path}"))?;
        build_tag_map(&cfg)
    } else {
        warn!("ch_settings_file not configured — L2 counter ops will match nothing (no tag map)");
        ChannelTagMap::new()
    };

    let calibration = load_calibration(eb.time_calib_file.as_deref());

    let cfg_template = PipelineConfig {
        safe_horizon_ns: eb.buffer_delay_ns,
        n_workers: args.workers,
        n_writers: args.writers,
        output_dir: PathBuf::from(&eb.output_dir),
        run_id: args.run_id,
        zmq_pub_endpoint: eb.zmq_pub_endpoint.clone(),
        ..PipelineConfig::default()
    };

    let inputs = PipelineInputs {
        eb_runtime: Arc::new(eb_runtime),
        ch_tags,
        calibration,
        cfg_template,
        subscribe: eb.subscribe.clone(),
    };

    print_banner(&args, eb, &inputs);

    // Branch on Operator-managed vs standalone mode.
    if eb.command.is_some() {
        run_operator_managed(args, eb.command.clone().unwrap(), inputs).await
    } else {
        run_standalone(args, inputs).await
    }
}

fn print_banner(
    args: &Args,
    eb: &delila_rs::config::EventBuilderNetworkConfig,
    inputs: &PipelineInputs,
) {
    println!("========================================");
    println!("  DELILA Online Event Builder");
    println!("  (unified pipeline; Merger SUB → ROOT)");
    println!("========================================");
    println!();
    println!("  Subscribe:         {}", eb.subscribe);
    println!(
        "  Output dir:        {}",
        inputs.cfg_template.output_dir.display()
    );
    if let Some(p) = eb.eb_config_file.as_deref() {
        println!("  eb_config:         {p}");
    }
    println!(
        "  Coincidence:       {} ns",
        inputs.eb_runtime.timing.coincidence_window_ns
    );
    println!(
        "  Safe horizon:      {:.1} ms",
        inputs.cfg_template.safe_horizon_ns / 1.0e6
    );
    println!(
        "  Workers / writers: {} / {}",
        inputs.cfg_template.n_workers, inputs.cfg_template.n_writers
    );
    println!("  Default run id:    {}", args.run_id);
    if let Some(ref ep) = inputs.cfg_template.zmq_pub_endpoint {
        println!("  EB-Monitor PUB:    {ep}");
    } else {
        println!("  EB-Monitor PUB:    disabled");
    }
    if let Some(ref cmd) = eb.command {
        println!("  Command REP:       {cmd}  (Operator-managed)");
    } else {
        println!("  Command REP:       disabled (standalone mode)");
    }
    println!();
}

// ─────────────────────────────────────────────────────────────────────────────
// Standalone mode — auto-start, run until Ctrl+C. Legacy behavior.
// ─────────────────────────────────────────────────────────────────────────────

async fn run_standalone(args: Args, inputs: PipelineInputs) -> anyhow::Result<()> {
    let (pipeline, source) = inputs.build(args.run_id)?;
    let shutdown = source.shutdown_handle();

    println!("  Press Ctrl+C to stop.");
    println!("========================================");

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

    print_run_summary(&stats);
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Operator-managed mode — wait for Start, spawn pipeline; tear down on Stop.
// ─────────────────────────────────────────────────────────────────────────────

async fn run_operator_managed(
    _args: Args,
    command_address: String,
    inputs: PipelineInputs,
) -> anyhow::Result<()> {
    let (state_tx, mut state_rx) = watch::channel(ComponentState::Idle);
    let shared_state = Arc::new(Mutex::new(ComponentSharedState::new()));

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    // SIGINT / SIGTERM handler.
    let shutdown_tx_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "Failed to install SIGTERM handler");
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => info!("SIGINT received"),
            _ = sigterm.recv() => info!("SIGTERM received"),
        }
        let _ = shutdown_tx_signal.send(());
    });

    // Command task.
    let cmd_shutdown = shutdown_tx.subscribe();
    let cmd_shared = shared_state.clone();
    let cmd_state_tx = state_tx.clone();
    let cmd_addr_for_task = command_address.clone();
    let cmd_handle = tokio::spawn(async move {
        run_command_task(
            cmd_addr_for_task,
            cmd_shared,
            cmd_state_tx,
            cmd_shutdown,
            move |state, tx, cmd| {
                let mut ext = EbCommandExt;
                handle_command(state, tx, cmd, Some(&mut ext))
            },
            "OnlineEB",
        )
        .await;
    });

    println!("  Operator-managed. Waiting for Start command…");
    println!("========================================");

    // Pipeline lifecycle state.
    let mut pipeline_thread: Option<std::thread::JoinHandle<PipelineStats>> = None;
    let mut current_shutdown: Option<delila_rs::event_builder::ZmqHitSourceShutdown> = None;
    let mut last_state = ComponentState::Idle;

    let mut shutdown_rx = shutdown_tx.subscribe();

    loop {
        tokio::select! {
            biased;

            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received — tearing down pipeline if running");
                tear_down(&mut pipeline_thread, &mut current_shutdown).await;
                break;
            }

            changed = state_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let new_state = *state_rx.borrow();
                info!(?last_state, ?new_state, "State transition observed");

                // Transition INTO Running: spawn pipeline thread.
                if new_state == ComponentState::Running
                    && pipeline_thread.is_none()
                {
                    let run_number = shared_state
                        .lock()
                        .await
                        .run_number()
                        .unwrap_or(0);
                    match inputs.build(run_number) {
                        Ok((pipeline, source)) => {
                            current_shutdown = Some(source.shutdown_handle());
                            match std::thread::Builder::new()
                                .name("eb-pipeline-driver".to_string())
                                .spawn(move || pipeline.run(source))
                            {
                                Ok(handle) => {
                                    pipeline_thread = Some(handle);
                                    info!(
                                        run_number,
                                        "EB pipeline started by Operator"
                                    );
                                }
                                Err(e) => {
                                    warn!(error = %e, "Failed to spawn pipeline thread");
                                    current_shutdown = None;
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to build pipeline; staying idle");
                        }
                    }
                }

                // Transition OUT of Running: tear down pipeline thread.
                if last_state == ComponentState::Running
                    && new_state != ComponentState::Running
                {
                    tear_down(&mut pipeline_thread, &mut current_shutdown).await;
                }

                last_state = new_state;
            }

            // Check liveness so a panic in the pipeline thread doesn't
            // silently leave us in Running forever.
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                if let Some(ref h) = pipeline_thread {
                    if h.is_finished() {
                        warn!(
                            "Pipeline thread finished while Operator state is still Running \
                             — joining to surface stats"
                        );
                        tear_down(&mut pipeline_thread, &mut current_shutdown).await;
                    }
                }
            }
        }
    }

    let _ = cmd_handle.await;
    Ok(())
}

async fn tear_down(
    pipeline_thread: &mut Option<std::thread::JoinHandle<PipelineStats>>,
    current_shutdown: &mut Option<delila_rs::event_builder::ZmqHitSourceShutdown>,
) {
    if let Some(sh) = current_shutdown.take() {
        sh.request();
    }
    if let Some(handle) = pipeline_thread.take() {
        match tokio::task::spawn_blocking(move || handle.join()).await {
            Ok(Ok(stats)) => {
                print_run_summary(&stats);
            }
            Ok(Err(e)) => {
                warn!(error = ?e, "Pipeline thread panicked while joining");
            }
            Err(e) => {
                warn!(error = %e, "Pipeline join task failed");
            }
        }
    }
}

fn print_run_summary(stats: &PipelineStats) {
    println!();
    println!("========================================");
    println!("  Event Builder Run Finished");
    println!("========================================");
    println!("  Received hits:    {}", stats.received_hits);
    println!("  Received batches: {}", stats.received_batches);
    println!("  Chunks processed: {}", stats.chunks_processed);
    println!("  Events built:     {}", stats.events_built);
    println!("  Events kept (L2): {}", stats.events_kept);
    println!("  Files written:    {}", stats.files_written);
    println!("  Write FAILURES:   {}", stats.write_failures);
    println!("  Batches published: {}", stats.batches_published);
    println!("========================================");
}

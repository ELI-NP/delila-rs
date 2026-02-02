//! Event Builder CLI
//!
//! Time Slice 方式のイベントビルダー CLI。
//! 並列処理可能で、メモリ効率が良い。
//!
//! Usage:
//!   # Time calibration
//!   event_builder time-calib -i input.root -o time_calib.json --ref-module 0 --ref-channel 0
//!
//!   # Event building (Time Slice method)
//!   event_builder build -i input.root -o events.root -T time_calib.json --trigger 0:0

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

#[cfg(feature = "root")]
use delila_rs::event_builder::{
    read_hits_from_root, write_events_to_root, SliceBuilder, TimeCalibration, TimeCalibrator,
};

#[cfg(not(feature = "root"))]
fn main() {
    eprintln!("Error: event_builder requires the 'root' feature.");
    eprintln!("Rebuild with: cargo build --release --features root --bin event_builder");
    std::process::exit(1);
}

#[derive(Parser)]
#[command(name = "event_builder")]
#[command(about = "ELIFANT-Event compatible offline event builder")]
#[command(version)]
#[cfg(feature = "root")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[cfg(feature = "root")]
enum Commands {
    /// Run time calibration to measure channel time offsets
    TimeCalib {
        /// Input ROOT file(s)
        #[arg(short, long, required = true)]
        input: Vec<PathBuf>,

        /// Output JSON file for time calibration
        #[arg(short, long, default_value = "timeSettings.json")]
        output: PathBuf,

        /// Reference trigger module
        #[arg(long, default_value = "0")]
        ref_module: u8,

        /// Reference trigger channel
        #[arg(long, default_value = "0")]
        ref_channel: u8,

        /// Coincidence window [ns]
        #[arg(long, default_value = "1000")]
        window: f64,

        /// Minimum entries for valid calibration
        #[arg(long, default_value = "1000")]
        min_entries: u64,

        /// Input tree name
        #[arg(long, default_value = "ELIADE_Tree")]
        tree_name: String,

        /// Maximum events to process (0 = all)
        #[arg(long, default_value = "0")]
        max_events: usize,
    },

    /// Build events using Time Slice method (parallel processing)
    Build {
        /// Input ROOT file(s)
        #[arg(short, long, required = true)]
        input: Vec<PathBuf>,

        /// Output ROOT file
        #[arg(short, long, default_value = "events.root")]
        output: PathBuf,

        /// Channel settings JSON file
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Time calibration JSON file
        #[arg(short = 'T', long)]
        time_calib: Option<PathBuf>,

        /// Coincidence window [ns]
        #[arg(long, default_value = "500")]
        window: f64,

        /// Time slice duration [ns] (default: 10 ms)
        #[arg(long, default_value = "10000000")]
        slice_duration: f64,

        /// Input tree name
        #[arg(long, default_value = "ELIADE_Tree")]
        tree_name: String,

        /// Output tree name
        #[arg(long, default_value = "events")]
        output_tree: String,

        /// Maximum hits to process (0 = all)
        #[arg(long, default_value = "0")]
        max_hits: usize,

        /// Trigger channels (module:channel), can be repeated
        #[arg(long)]
        trigger: Vec<String>,
    },
}

#[cfg(feature = "root")]
fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("event_builder=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::TimeCalib {
            input,
            output,
            ref_module,
            ref_channel,
            window,
            min_entries,
            tree_name,
            max_events,
        } => {
            run_time_calibration(
                &input,
                &output,
                ref_module,
                ref_channel,
                window,
                min_entries,
                &tree_name,
                max_events,
            )?;
        }
        Commands::Build {
            input,
            output,
            config,
            time_calib,
            window,
            slice_duration,
            tree_name,
            output_tree,
            max_hits,
            trigger,
        } => {
            run_event_building(
                &input,
                &output,
                config.as_deref(),
                time_calib.as_deref(),
                window,
                slice_duration,
                &tree_name,
                &output_tree,
                max_hits,
                &trigger,
            )?;
        }
    }

    Ok(())
}

#[cfg(feature = "root")]
#[allow(clippy::too_many_arguments)]
fn run_time_calibration(
    input_files: &[PathBuf],
    output: &std::path::Path,
    ref_module: u8,
    ref_channel: u8,
    window: f64,
    min_entries: u64,
    tree_name: &str,
    max_events: usize,
) -> Result<()> {
    info!(
        "Running time calibration: ref=({}, {}), window={}ns, {} files (parallel)",
        ref_module,
        ref_channel,
        window,
        input_files.len()
    );

    let total_hits = Arc::new(AtomicUsize::new(0));
    let files_processed = Arc::new(AtomicUsize::new(0));
    let tree_name = tree_name.to_string();

    // Process files in parallel
    let calibrators: Vec<TimeCalibrator> = input_files
        .par_iter()
        .filter_map(|path| {
            // Check if we've hit max_events limit
            if max_events > 0 && total_hits.load(Ordering::Relaxed) >= max_events {
                return None;
            }

            let hits = match read_hits_from_root(path, &tree_name) {
                Ok(h) => h,
                Err(e) => {
                    warn!("Failed to read {}: {:?}", path.display(), e);
                    return None;
                }
            };

            let mut calibrator = TimeCalibrator::new(ref_module, ref_channel, window);
            // ROOT files are time-sorted, use optimized O(n) algorithm
            calibrator.process_hits_sorted(&hits);

            let n = files_processed.fetch_add(1, Ordering::Relaxed) + 1;
            let total = total_hits.fetch_add(hits.len(), Ordering::Relaxed) + hits.len();
            info!(
                "  [{}/{}] {}: {} hits (total: {})",
                n,
                input_files.len(),
                path.file_name().unwrap_or_default().to_string_lossy(),
                hits.len(),
                total
            );

            Some(calibrator)
        })
        .collect();

    // Merge all calibrators
    let mut calibrator = TimeCalibrator::new(ref_module, ref_channel, window);
    calibrator.set_min_entries(min_entries);

    for other in calibrators {
        calibrator.merge(other);
    }

    let total_hits = total_hits.load(Ordering::Relaxed);
    info!("Total hits processed: {}", total_hits);

    let calib = calibrator.calculate_calibration();

    // Report statistics
    let n_histograms = calibrator.channels().count();
    let n_offsets = calib.offsets().len();
    info!(
        "Calibration complete: {} channels with histograms, {} with valid offsets",
        n_histograms, n_offsets
    );

    // Save calibration
    calib
        .to_json_file(output)
        .with_context(|| format!("Failed to write {}", output.display()))?;

    info!("Saved calibration to: {}", output.display());

    // Print summary (top 20 channels by entries)
    let mut channel_stats: Vec<_> = calibrator
        .channels()
        .filter_map(|&(m, c)| {
            calibrator
                .get_histogram(m, c)
                .map(|h| (m, c, h.entries(), calib.get_offset(m, c)))
        })
        .collect();
    channel_stats.sort_by_key(|(_, _, entries, _)| std::cmp::Reverse(*entries));

    for (module, channel, entries, offset) in channel_stats.iter().take(20) {
        info!(
            "  Ch({}, {}): offset = {:.2} ns ({} entries)",
            module, channel, offset, entries
        );
    }

    Ok(())
}

#[cfg(feature = "root")]
#[allow(clippy::too_many_arguments)]
fn run_event_building(
    input_files: &[PathBuf],
    output: &std::path::Path,
    config: Option<&std::path::Path>,
    time_calib: Option<&std::path::Path>,
    window: f64,
    slice_duration: f64,
    tree_name: &str,
    output_tree: &str,
    max_hits: usize,
    trigger_args: &[String],
) -> Result<()> {
    info!(
        "Building events (Time Slice): window={}ns, slice_duration={}ns ({:.1}ms)",
        window,
        slice_duration,
        slice_duration / 1_000_000.0
    );

    // Create SliceBuilder with slice duration and coincidence window
    let mut builder = SliceBuilder::new(slice_duration, window);

    // Load channel config if provided (for AC pairs and triggers)
    let mut config_triggers: Vec<(u8, u8)> = Vec::new();
    if let Some(config_path) = config {
        let config = delila_rs::event_builder::load_channel_config(config_path)
            .with_context(|| format!("Failed to load config: {}", config_path.display()))?;

        // Configure AC pairs and collect triggers from settings
        let mut ac_count = 0;
        for group in &config {
            for ch in group {
                // AC pairs
                if ch.has_ac && ch.ac_module != 128 {
                    builder.add_ac_pair(ch.module, ch.channel, ch.ac_module, ch.ac_channel);
                    ac_count += 1;
                }
                // Triggers (IsEventTrigger=true)
                if ch.is_event_trigger {
                    config_triggers.push((ch.module, ch.channel));
                }
            }
        }
        info!(
            "Loaded channel config from: {} ({} AC pairs, {} triggers)",
            config_path.display(),
            ac_count,
            config_triggers.len()
        );
    }

    // Use command-line triggers if provided, otherwise use config triggers
    if !trigger_args.is_empty() {
        // Parse trigger arguments from command line
        for (priority, trig) in trigger_args.iter().enumerate() {
            let parts: Vec<&str> = trig.split(':').collect();
            if parts.len() == 2 {
                let module: u8 = parts[0].parse().context("Invalid trigger module")?;
                let channel: u8 = parts[1].parse().context("Invalid trigger channel")?;
                builder.add_trigger(module, channel, priority as u32);
                info!(
                    "Added trigger: ({}, {}) priority {}",
                    module, channel, priority
                );
            } else {
                warn!("Invalid trigger format: {} (expected module:channel)", trig);
            }
        }
    } else if !config_triggers.is_empty() {
        // Use triggers from config file
        for (priority, (module, channel)) in config_triggers.iter().enumerate() {
            builder.add_trigger(*module, *channel, priority as u32);
        }
        info!(
            "Using {} triggers from config (IsEventTrigger=true)",
            config_triggers.len()
        );
    } else {
        warn!("No triggers specified! Use --trigger or provide config with IsEventTrigger=true");
    }

    // Load time calibration if provided
    if let Some(calib_path) = time_calib {
        let calib = TimeCalibration::from_json_file(calib_path).with_context(|| {
            format!("Failed to load time calibration: {}", calib_path.display())
        })?;
        builder.set_time_calibration(calib);
        info!("Loaded time calibration from: {}", calib_path.display());
    }

    // Read all hits from all input files
    let mut all_hits = Vec::new();

    for path in input_files {
        info!("Reading: {}", path.display());

        let hits = read_hits_from_root(path, tree_name)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        info!(
            "  Read {} hits from {}",
            hits.len(),
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        all_hits.extend(hits);

        // Check max_hits limit
        if max_hits > 0 && all_hits.len() >= max_hits {
            all_hits.truncate(max_hits);
            info!("Reached max_hits limit: {}", max_hits);
            break;
        }
    }

    info!("Total hits to process: {}", all_hits.len());

    // Build events using Time Slice method (parallel processing)
    let events = builder.build_events(all_hits);

    // Get statistics
    let stats = builder.stats();
    info!(
        "Built {} events using {} trigger channel(s)",
        events.len(),
        stats.n_triggers
    );

    // Write output
    write_events_to_root(output, output_tree, &events)
        .with_context(|| format!("Failed to write {}", output.display()))?;

    info!("Wrote {} events to: {}", events.len(), output.display());

    // Print statistics
    if !events.is_empty() {
        let total_mult: usize = events.iter().map(|e| e.multiplicity()).sum();
        let avg_mult = total_mult as f64 / events.len() as f64;
        info!("Average multiplicity: {:.2}", avg_mult);
    }

    Ok(())
}

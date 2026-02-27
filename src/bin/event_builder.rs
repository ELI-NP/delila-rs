//! Event Builder CLI
//!
//! Offline event builder using chunk_builder + unified pipeline.
//! .delila ファイルを直接読み込み、ROOT イベントファイルを出力。
//!
//! Usage:
//!   # Time calibration (.delila input)
//!   event_builder time-calib -i data/*.delila -o timeSettings.json --ref-module 0 --ref-channel 0
//!
//!   # Event building (.delila input → ROOT output)
//!   event_builder build -i data/*.delila -o ./output/ -c chSettings.json -T timeSettings.json --trigger 0:0

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

#[cfg(feature = "root")]
use delila_rs::event_builder::{
    write_time_histograms_to_root, Hit, TimeCalibration, TimeCalibrator,
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
        /// Input .delila file(s)
        #[arg(short, long, required = true, num_args = 1..)]
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

        /// Maximum events to process (0 = all)
        #[arg(long, default_value = "0")]
        max_events: usize,

        /// Output ROOT file for time histograms (for visual inspection)
        #[arg(long, default_value = "timeAlignment.root")]
        hist_output: PathBuf,

        /// Reference trigger energy minimum (ADC units, 16-bit)
        #[arg(long, default_value = "0")]
        ref_energy_min: u16,

        /// Reference trigger energy maximum (ADC units, 16-bit)
        #[arg(long, default_value = "65535")]
        ref_energy_max: u16,
    },

    /// Build events from .delila files using unified pipeline
    Build {
        /// Input .delila file(s)
        #[arg(short, long, required = true, num_args = 1..)]
        input: Vec<PathBuf>,

        /// Output directory for ROOT event files
        #[arg(short, long, default_value = ".")]
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

        /// Output tree name
        #[arg(long, default_value = "EventTree")]
        output_tree: String,

        /// Run ID for output file naming
        #[arg(long, default_value = "0")]
        run_id: u32,

        /// Number of worker threads
        #[arg(long, default_value = "4")]
        workers: usize,

        /// Number of writer threads
        #[arg(long, default_value = "2")]
        writers: usize,

        /// Events per ROOT file before rotation
        #[arg(long, default_value = "100000")]
        events_per_file: usize,

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
            max_events,
            hist_output,
            ref_energy_min,
            ref_energy_max,
        } => {
            run_time_calibration(
                &input,
                &output,
                ref_module,
                ref_channel,
                window,
                min_entries,
                max_events,
                &hist_output,
                ref_energy_min,
                ref_energy_max,
            )?;
        }
        Commands::Build {
            input,
            output,
            config,
            time_calib,
            window,
            output_tree,
            run_id,
            workers,
            writers,
            events_per_file,
            trigger,
        } => {
            run_event_building(
                &input,
                &output,
                config.as_deref(),
                time_calib.as_deref(),
                window,
                &output_tree,
                run_id,
                workers,
                writers,
                events_per_file,
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
    max_events: usize,
    hist_output: &std::path::Path,
    ref_energy_min: u16,
    ref_energy_max: u16,
) -> Result<()> {
    use delila_rs::recorder::DataFileReader;
    use std::io::BufReader;

    let energy_gate = if ref_energy_min == 0 && ref_energy_max == u16::MAX {
        "all".to_string()
    } else {
        format!("{}-{}", ref_energy_min, ref_energy_max)
    };
    info!(
        "Running time calibration: ref=({}, {}), window={}ns, energy={}, {} .delila files (parallel)",
        ref_module, ref_channel, window, energy_gate, input_files.len()
    );

    let total_hits = Arc::new(AtomicUsize::new(0));
    let files_processed = Arc::new(AtomicUsize::new(0));

    // Process files in parallel
    let calibrators: Vec<TimeCalibrator> = input_files
        .par_iter()
        .filter_map(|path| {
            // Check if we've hit max_events limit
            if max_events > 0 && total_hits.load(Ordering::Relaxed) >= max_events {
                return None;
            }

            // Read .delila file → Vec<Hit>
            let file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(e) => {
                    warn!("Failed to open {}: {:?}", path.display(), e);
                    return None;
                }
            };
            let buf_reader = BufReader::new(file);
            let mut reader = match DataFileReader::new(buf_reader) {
                Ok(r) => r,
                Err(e) => {
                    warn!("Failed to parse {}: {:?}", path.display(), e);
                    return None;
                }
            };

            // Phase 2: Streaming trigger-index with stateful scanner
            // 1. Separate triggers (f64 only) and detector hits (by block)
            let mut trigger_times: Vec<f64> = Vec::new();
            let mut det_blocks: Vec<Vec<Hit>> = Vec::new();
            let mut hit_count: usize = 0;

            for block_result in reader.data_blocks() {
                match block_result {
                    Ok(batch) => {
                        let mut block_hits = Vec::new();
                        for event in &batch.events {
                            let hit = Hit::from_event_data(event);
                            hit_count += 1;
                            if hit.module == ref_module
                                && hit.channel == ref_channel
                                && hit.energy >= ref_energy_min
                                && hit.energy <= ref_energy_max
                            {
                                trigger_times.push(hit.timestamp_ns);
                            } else if hit.module != ref_module
                                || hit.channel != ref_channel
                            {
                                block_hits.push(hit);
                            }
                        }
                        if !block_hits.is_empty() {
                            det_blocks.push(block_hits);
                        }
                    }
                    Err(_) => continue,
                }
            }

            // Sort only triggers: O(t log t) where t << n
            trigger_times.sort_unstable_by(|a, b| a.total_cmp(b));

            // Process each block with stateful scanner: amortized O(n)
            let mut calibrator = TimeCalibrator::new(ref_module, ref_channel, window);
            calibrator.set_ref_energy_range(ref_energy_min, ref_energy_max);
            for block in &det_blocks {
                calibrator.process_block_with_sorted_triggers(&trigger_times, block);
            }

            let n = files_processed.fetch_add(1, Ordering::Relaxed) + 1;
            let total = total_hits.fetch_add(hit_count, Ordering::Relaxed) + hit_count;
            info!(
                "  [{}/{}] {}: {} hits, {} triggers (total: {})",
                n,
                input_files.len(),
                path.file_name().unwrap_or_default().to_string_lossy(),
                hit_count,
                trigger_times.len(),
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
    let n_valid = calib.offsets().values().filter(|v| **v != 0.0).count();
    info!(
        "Calibration complete: {} channels total, {} with valid offsets, {} set to 0",
        n_histograms,
        n_valid,
        n_histograms - n_valid
    );

    // Save calibration
    calib
        .to_json_file(output)
        .with_context(|| format!("Failed to write {}", output.display()))?;

    info!("Saved calibration to: {}", output.display());

    // Write time alignment histograms to ROOT file (for visual inspection)
    write_time_histograms_to_root(hist_output, "TimeAlignment", &calibrator)
        .with_context(|| format!("Failed to write histograms to {}", hist_output.display()))?;
    info!(
        "Saved time alignment histograms to: {} ({} channels)",
        hist_output.display(),
        n_histograms
    );

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
    output_dir: &std::path::Path,
    config: Option<&std::path::Path>,
    time_calib: Option<&std::path::Path>,
    window: f64,
    output_tree: &str,
    run_id: u32,
    n_workers: usize,
    n_writers: usize,
    events_per_file: usize,
    trigger_args: &[String],
) -> Result<()> {
    use delila_rs::event_builder::chunk_builder::TriggerConfig;
    use delila_rs::event_builder::pipeline::{EventBuilderPipeline, PipelineConfig};
    use delila_rs::event_builder::{load_channel_config, DelilaFileHitSource};
    use std::collections::{HashMap, HashSet};

    info!(
        "Building events: window={}ns, {} input files, {} workers, {} writers",
        window,
        input_files.len(),
        n_workers,
        n_writers,
    );

    // Build TriggerConfig
    let trigger_config = if let Some(config_path) = config {
        // Load from channel settings JSON
        let ch_config = load_channel_config(config_path)
            .with_context(|| format!("Failed to load config: {}", config_path.display()))?;

        let mut tc = TriggerConfig::from_channel_config(&ch_config, window);
        info!(
            "Loaded channel config from: {} ({} triggers, {} AC pairs)",
            config_path.display(),
            tc.triggers.len(),
            tc.ac_pairs.len()
        );

        // Override triggers from CLI if provided
        if !trigger_args.is_empty() {
            tc.triggers.clear();
            tc.priorities.clear();
            for (priority, trig) in trigger_args.iter().enumerate() {
                let parts: Vec<&str> = trig.split(':').collect();
                if parts.len() == 2 {
                    let module: u8 = parts[0].parse().context("Invalid trigger module")?;
                    let channel: u8 = parts[1].parse().context("Invalid trigger channel")?;
                    tc.triggers.insert((module, channel));
                    tc.priorities.insert((module, channel), priority as u32);
                    info!(
                        "CLI trigger override: ({}, {}) priority {}",
                        module, channel, priority
                    );
                } else {
                    warn!("Invalid trigger format: {} (expected module:channel)", trig);
                }
            }
        }
        tc
    } else {
        // Build from CLI --trigger args only
        let mut triggers = HashSet::new();
        let mut priorities = HashMap::new();
        for (priority, trig) in trigger_args.iter().enumerate() {
            let parts: Vec<&str> = trig.split(':').collect();
            if parts.len() == 2 {
                let module: u8 = parts[0].parse().context("Invalid trigger module")?;
                let channel: u8 = parts[1].parse().context("Invalid trigger channel")?;
                triggers.insert((module, channel));
                priorities.insert((module, channel), priority as u32);
                info!(
                    "Added trigger: ({}, {}) priority {}",
                    module, channel, priority
                );
            } else {
                warn!("Invalid trigger format: {} (expected module:channel)", trig);
            }
        }

        if triggers.is_empty() {
            warn!("No triggers specified! Use --trigger or -c config with IsEventTrigger=true");
        }

        TriggerConfig {
            triggers,
            priorities,
            ac_pairs: HashMap::new(),
            coincidence_window_ns: window,
        }
    };

    // Load time calibration
    let time_calibration = if let Some(calib_path) = time_calib {
        let calib = TimeCalibration::from_json_file(calib_path).with_context(|| {
            format!("Failed to load time calibration: {}", calib_path.display())
        })?;
        info!("Loaded time calibration from: {}", calib_path.display());
        calib
    } else {
        TimeCalibration::new(0, 0)
    };

    // Ensure output directory exists
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output dir: {}", output_dir.display()))?;

    // Create pipeline
    let pipeline_config = PipelineConfig {
        safe_horizon_ns: 50_000_000.0, // 50ms
        n_workers,
        n_writers,
        events_per_file,
        sorter_threshold: 500_000,
        sorter_timeout: std::time::Duration::from_millis(500),
        output_dir: output_dir.to_path_buf(),
        run_id,
        output_tree: output_tree.to_string(),
    };

    let pipeline = EventBuilderPipeline::new(pipeline_config, trigger_config, time_calibration);

    // Create source from .delila files
    let source = DelilaFileHitSource::new(input_files.to_vec());

    // Run pipeline (blocking)
    let stats = pipeline.run(source);

    info!(
        "Event building complete: {} hits → {} events in {} files",
        stats.received_hits, stats.events_built, stats.files_written
    );

    Ok(())
}

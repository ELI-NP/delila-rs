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

        /// Trigger channels (module:channel), can be repeated.
        /// Ignored when --eb-config is supplied.
        #[arg(long)]
        trigger: Vec<String>,

        /// Path to eb_config.json (SPEC § 4.1). When set, both L1
        /// (trigger config) and L2 (filter) are derived from it,
        /// overriding --trigger / --window / -c.
        #[arg(long)]
        eb_config: Option<PathBuf>,

        /// Treat input files as ELIFANT-style ROOT (`ELIADE_Tree`)
        /// instead of `.delila`. Useful for re-analysing data from
        /// older runs through the unified pipeline.
        #[arg(long)]
        root_input: bool,

        /// Tree name used when --root-input is set.
        #[arg(long, default_value = "ELIADE_Tree")]
        root_tree: String,
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
            eb_config,
            root_input,
            root_tree,
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
                eb_config.as_deref(),
                root_input,
                &root_tree,
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
                            } else if hit.module != ref_module || hit.channel != ref_channel {
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
    eb_config: Option<&std::path::Path>,
    root_input: bool,
    root_tree: &str,
) -> Result<()> {
    use delila_rs::event_builder::chunk_builder::TriggerConfig;
    use delila_rs::event_builder::pipeline::{EventBuilderPipeline, PipelineConfig};
    use delila_rs::event_builder::{
        load_channel_config, ChannelTagMap, DelilaFileHitSource, EbRuntimeConfig, HitSource,
        L2Filter, RootFileHitSource,
    };
    use std::collections::{HashMap, HashSet};

    info!(
        "Building events: {} input files ({}), {} workers, {} writers",
        input_files.len(),
        if root_input { "ROOT" } else { ".delila" },
        n_workers,
        n_writers,
    );

    // ── Resolve TriggerConfig + L2Filter ────────────────────────────────
    //
    // Two paths:
    //   (a) --eb-config supplied → derive both L1 (TriggerConfig) and L2
    //       (L2Filter) from eb_config.json + the tag map in chSettings.json.
    //       This is the new SPEC v0.5.1 path and the only one that supports
    //       multi-channel triggers / multiplicity / L2 cuts.
    //   (b) Legacy: --trigger CLI args build a static TriggerConfig; L2
    //       filter is disabled.
    let (trigger_config, l2_filter, effective_window) = if let Some(eb_cfg_path) = eb_config {
        let rt = EbRuntimeConfig::load(eb_cfg_path)
            .with_context(|| format!("Failed to load eb_config.json: {}", eb_cfg_path.display()))?;
        let tc = rt
            .build_trigger_config()
            .context("Failed to derive TriggerConfig from eb_config.l1")?;
        let ch_tags: ChannelTagMap = match config {
            Some(p) => {
                let cfg = load_channel_config(p)
                    .with_context(|| format!("Failed to load chSettings.json: {}", p.display()))?;
                let mut m = HashMap::new();
                for module_chs in &cfg {
                    for c in module_chs {
                        m.insert((c.module, c.channel), c.tags.clone());
                    }
                }
                m
            }
            None => {
                warn!(
                    "--eb-config supplied but --config (chSettings.json) is not — \
                       L2 counter ops will see no tags"
                );
                HashMap::new()
            }
        };
        let l2 = L2Filter::new(rt.l2.clone(), ch_tags).context("Failed to build L2 filter")?;
        info!(
            file = %eb_cfg_path.display(),
            coincidence = rt.timing.coincidence_window_ns,
            triggers = tc.triggers.len(),
            mult = tc.multiplicity_triggers.len(),
            l2_ops = rt.l2.len(),
            "Loaded eb_config.json"
        );
        (tc, Some(l2), rt.timing.coincidence_window_ns)
    } else {
        if let Some(config_path) = config {
            match load_channel_config(config_path) {
                Ok(_) => info!(
                    "Loaded chSettings.json (descriptor only) from: {}",
                    config_path.display()
                ),
                Err(e) => warn!(
                    "Failed to load chSettings.json {}: {} — continuing without it",
                    config_path.display(),
                    e
                ),
            }
        }
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
            warn!("No triggers specified — pass --trigger or --eb-config");
        }
        let tc = TriggerConfig {
            triggers,
            priorities,
            ac_pairs: HashMap::new(),
            coincidence_window_ns: window,
            multiplicity_triggers: Vec::new(),
        };
        (tc, None, window)
    };

    // Load time calibration — try the SPEC v0.5.1 tree schema first
    // (`{version, entries: [...]}`), fall back to the legacy single-ref
    // JSON if that fails to parse.
    let time_calibration = if let Some(calib_path) = time_calib {
        use delila_rs::event_builder::TimeOffsetsFile;
        if let Ok(tree_file) = TimeOffsetsFile::load(calib_path) {
            match tree_file.resolve() {
                Ok(resolved) => {
                    for w in &resolved.warnings {
                        warn!("{w}");
                    }
                    info!(
                        "Loaded timeSettings.json (tree schema, {} roots) from: {}",
                        resolved.root_count(),
                        calib_path.display()
                    );
                    resolved.into_time_calibration()
                }
                Err(e) => {
                    warn!("Failed to resolve tree-form timeSettings: {e} — falling back to legacy loader");
                    TimeCalibration::from_json_file(calib_path).with_context(|| {
                        format!("Failed to load time calibration: {}", calib_path.display())
                    })?
                }
            }
        } else {
            let calib = TimeCalibration::from_json_file(calib_path).with_context(|| {
                format!("Failed to load time calibration: {}", calib_path.display())
            })?;
            info!(
                "Loaded legacy time calibration from: {}",
                calib_path.display()
            );
            calib
        }
    } else {
        TimeCalibration::new(0, 0)
    };

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output dir: {}", output_dir.display()))?;

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
        zmq_pub_endpoint: None, // offline EB does not publish
    };

    info!(
        "Pipeline config: coincidence={} ns, events_per_file={}, output_dir={}",
        effective_window,
        events_per_file,
        output_dir.display()
    );

    let mut pipeline = EventBuilderPipeline::new(pipeline_config, trigger_config, time_calibration);
    if let Some(l2) = l2_filter {
        pipeline = pipeline.with_l2_filter(l2);
    }

    let stats = if root_input {
        // ELIFANT-style ROOT (ELIADE_Tree). Batch size of 100k is a balance
        // between memory and worker throughput — matches `events_per_file`
        // order of magnitude.
        let source: RootFileHitSource =
            RootFileHitSource::new(input_files.to_vec(), root_tree, 100_000);
        info!(
            "Reading ROOT files from `{}` tree (source: {})",
            root_tree,
            source.name()
        );
        pipeline.run(source)
    } else {
        let source = DelilaFileHitSource::new(input_files.to_vec());
        pipeline.run(source)
    };

    info!(
        hits = stats.received_hits,
        events_built = stats.events_built,
        events_kept = stats.events_kept,
        files = stats.files_written,
        "Event building complete"
    );

    Ok(())
}

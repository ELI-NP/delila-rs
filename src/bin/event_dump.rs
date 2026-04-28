//! Quick event dump tool — shows Mod/Ch/Timestamp for first N events, or runs
//! analyses useful for V1743 CFD validation:
//!
//! ```text
//!   event_dump <file>                              # summary + first 50 events
//!   event_dump <file> 200                          # first 200 events
//!   event_dump <file> --summary                    # summary only
//!   event_dump <file> --waveforms N [--ch C] [-o F.csv]
//!       # dump the first N waveforms (one sample per row) to CSV
//!       # columns: event_idx,module,channel,sample_idx,time_ns,adc
//!   event_dump <file> --delta-t-hist --ch C [--bins N] [--range-ns R]
//!       # pairwise Δt histogram for channel C; prints stats + CSV-safe histogram
//!   event_dump <file> --wf-stats N [--ch C]
//!       # scan first N waveforms (optionally filtered by channel) and print
//!       # amplitude / peak-index / baseline stats
//!   event_dump --global <files...>                 # multi-file per-module summary
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use delila_rs::recorder::DataFileReader;
use rayon::prelude::*;

struct ModStats {
    count: usize,
    min_ts: f64,
    max_ts: f64,
    #[allow(dead_code)]
    first_ts: f64,
}

impl ModStats {
    fn new(first_ts: f64) -> Self {
        Self {
            count: 0,
            min_ts: f64::MAX,
            max_ts: f64::MIN,
            first_ts,
        }
    }

    fn update(&mut self, ts: f64) {
        self.count += 1;
        if ts < self.min_ts {
            self.min_ts = ts;
        }
        if ts > self.max_ts {
            self.max_ts = ts;
        }
    }
}

fn process_single_file(
    path: &PathBuf,
    count: usize,
    summary_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut data_reader = DataFileReader::new(reader)?;

    let mut mod_stats: BTreeMap<u8, ModStats> = BTreeMap::new();
    let mut total_events = 0usize;
    let mut printed = 0usize;

    for block_result in data_reader.data_blocks() {
        let batch = block_result?;
        for event in &batch.events {
            total_events += 1;

            let stats = mod_stats
                .entry(event.module)
                .or_insert_with(|| ModStats::new(event.timestamp_ns));
            stats.update(event.timestamp_ns);

            if !summary_only && printed < count {
                let fine_ns = event.timestamp_ns - event.timestamp_ns.floor();
                let peak_idx = event.flags & 0xFFFF;
                let cfd_valid = (event.flags >> 24) & 1 != 0;
                let wf_fail = (event.flags >> 25) & 1 != 0;
                let wf = event
                    .waveform
                    .as_ref()
                    .map(|w| w.analog_probe1.len())
                    .unwrap_or(0);
                println!(
                    "Event {:>8}: Mod={} Ch={:>2} T={:>18.3} ns fine={:.3} ns E={:>5} ES={:>5} peak_i={:>4} cfd_ok={} wf_fail={} WF={}",
                    total_events,
                    event.module,
                    event.channel,
                    event.timestamp_ns,
                    fine_ns,
                    event.energy,
                    event.energy_short,
                    peak_idx,
                    cfd_valid as u8,
                    wf_fail as u8,
                    wf,
                );
                printed += 1;
            }
        }
    }

    println!(
        "\n=== Per-Module Summary ({} total events) ===",
        total_events
    );
    println!(
        "{:<6} {:>10} {:>20} {:>20} {:>15}",
        "Mod", "Events", "Min Timestamp (s)", "Max Timestamp (s)", "Range (s)"
    );
    for (mod_id, stats) in &mod_stats {
        let range_s = (stats.max_ts - stats.min_ts) / 1e9;
        println!(
            "Mod {:>2} {:>10} {:>20.3} {:>20.3} {:>15.3}",
            mod_id,
            stats.count,
            stats.min_ts / 1e9,
            stats.max_ts / 1e9,
            range_s,
        );
    }

    Ok(())
}

fn process_waveform_dump(
    path: &PathBuf,
    count: usize,
    channel_filter: Option<u8>,
    output: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut data_reader = DataFileReader::new(reader)?;

    let out = File::create(output)?;
    let mut out = BufWriter::new(out);
    writeln!(out, "event_idx,module,channel,sample_idx,time_ns,adc")?;

    let mut written = 0usize;
    let mut event_idx = 0usize;

    'outer: for block_result in data_reader.data_blocks() {
        let batch = block_result?;
        for event in &batch.events {
            event_idx += 1;
            if let Some(c) = channel_filter {
                if event.channel != c {
                    continue;
                }
            }
            if let Some(wf) = event.waveform.as_ref() {
                if wf.analog_probe1.is_empty() {
                    continue;
                }
                let ns_per = wf.ns_per_sample;
                for (i, &s) in wf.analog_probe1.iter().enumerate() {
                    writeln!(
                        out,
                        "{},{},{},{},{:.4},{}",
                        event_idx,
                        event.module,
                        event.channel,
                        i,
                        i as f64 * ns_per,
                        s,
                    )?;
                }
                written += 1;
                if written >= count {
                    break 'outer;
                }
            }
        }
    }

    eprintln!(
        "Wrote {} waveform(s){} to {}",
        written,
        channel_filter
            .map(|c| format!(" (ch={})", c))
            .unwrap_or_default(),
        output.display()
    );
    if written == 0 {
        eprintln!(
            "Warning: no waveforms found — did the run have save_waveform=true and \
             matching channel filter?"
        );
    }
    Ok(())
}

/// Δt histogram for a single channel. Computes inter-event times on the sorted
/// timestamp list, centers the histogram on the sample mean, and reports RMS
/// (standard deviation). Handy for validating CFD timing resolution.
fn process_delta_t_hist(
    path: &PathBuf,
    channel: u8,
    bins: usize,
    range_ns: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut data_reader = DataFileReader::new(reader)?;

    let mut timestamps = Vec::<f64>::new();
    for block_result in data_reader.data_blocks() {
        let batch = block_result?;
        for event in &batch.events {
            if event.channel == channel {
                timestamps.push(event.timestamp_ns);
            }
        }
    }

    if timestamps.len() < 2 {
        eprintln!(
            "Only {} events on ch{} — need ≥2 for Δt statistics",
            timestamps.len(),
            channel
        );
        return Ok(());
    }
    timestamps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut deltas: Vec<f64> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = deltas.len();
    let sum: f64 = deltas.iter().sum();
    let mean = sum / n as f64;
    let var = deltas.iter().map(|&d| (d - mean).powi(2)).sum::<f64>() / n as f64;
    let rms = var.sqrt();
    let median = deltas[n / 2];
    let p01 = deltas[(n as f64 * 0.01) as usize];
    let p99 = deltas[(n - 1).min((n as f64 * 0.99) as usize)];

    println!("=== Δt statistics (ch={}, N={}) ===", channel, n);
    println!("  mean   = {:>15.4} ns ({:.6} ms)", mean, mean / 1e6);
    println!("  median = {:>15.4} ns", median);
    println!("  rms    = {:>15.4} ns  (= stdev)", rms);
    println!("  min    = {:>15.4} ns", deltas[0]);
    println!("  1%     = {:>15.4} ns", p01);
    println!("  99%    = {:>15.4} ns", p99);
    println!("  max    = {:>15.4} ns", deltas[n - 1]);
    println!(
        "  expected pulser period for a 10 kHz source = 100000 ns ({} kHz → {:.1} ns)",
        1_000_000_000f64 / mean,
        mean
    );

    let center = mean;
    let bin_width = (2.0 * range_ns) / bins as f64;
    let lo = center - range_ns;
    let mut hist = vec![0usize; bins];
    for &d in &deltas {
        let idx_f = (d - lo) / bin_width;
        if idx_f < 0.0 {
            continue;
        }
        let idx = idx_f as usize;
        if idx < bins {
            hist[idx] += 1;
        }
    }
    let peak = *hist.iter().max().unwrap_or(&0);
    let peak_scale = 60.0 / peak.max(1) as f64;

    println!(
        "\n=== Histogram (center={:.3} ns, ±{} ns, {} bins, {:.3} ns/bin) ===",
        center, range_ns, bins, bin_width
    );
    println!("  bin_lo (ns relative to mean), count, bar");
    for (i, &c) in hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let lo_rel = -range_ns + i as f64 * bin_width;
        let bar_len = (c as f64 * peak_scale).round() as usize;
        let bar = "#".repeat(bar_len);
        println!("  {:>+10.4}  {:>10}  {}", lo_rel, c, bar);
    }

    Ok(())
}

/// Scan the first `count` events that carry a waveform (optionally filtered by
/// channel) and report distributional stats on the amplitude (|peak − baseline|),
/// baseline, peak index, and sample span. This is the fastest way to tell whether
/// the readout window is actually capturing the pulse during a parameter sweep.
fn process_waveform_stats(
    path: &PathBuf,
    count: usize,
    channel_filter: Option<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut data_reader = DataFileReader::new(reader)?;

    let mut amplitudes: Vec<f64> = Vec::with_capacity(count);
    let mut baselines: Vec<f64> = Vec::with_capacity(count);
    let mut peak_idx: Vec<usize> = Vec::with_capacity(count);
    let mut spans: Vec<i32> = Vec::with_capacity(count);
    let mut seen = 0usize;
    let baseline_n = 32usize;

    'outer: for block_result in data_reader.data_blocks() {
        let batch = block_result?;
        for event in &batch.events {
            if let Some(c) = channel_filter {
                if event.channel != c {
                    continue;
                }
            }
            let Some(wf) = event.waveform.as_ref() else {
                continue;
            };
            if wf.analog_probe1.is_empty() {
                continue;
            }
            let s = &wf.analog_probe1;
            let n_bl = baseline_n.min(s.len() / 2);
            let baseline: f64 = s[..n_bl].iter().map(|&v| v as f64).sum::<f64>() / n_bl as f64;
            let (pk_idx, &pk_val) = s
                .iter()
                .enumerate()
                .min_by_key(|&(_, v)| *v)
                .unwrap_or((0, &0));
            let max_val = *s.iter().max().unwrap_or(&0);
            let amp = (pk_val as f64 - baseline)
                .abs()
                .max((max_val as f64 - baseline).abs());

            amplitudes.push(amp);
            baselines.push(baseline);
            peak_idx.push(pk_idx);
            spans.push((max_val - pk_val) as i32);
            seen += 1;
            if seen >= count {
                break 'outer;
            }
        }
    }

    if amplitudes.is_empty() {
        eprintln!(
            "No waveforms matched{} — did the run use save_waveform=true?",
            channel_filter
                .map(|c| format!(" (ch={})", c))
                .unwrap_or_default()
        );
        return Ok(());
    }

    fn percentile(sorted: &[f64], p: f64) -> f64 {
        let i = ((sorted.len() - 1) as f64 * p).round() as usize;
        sorted[i]
    }

    let mut a = amplitudes.clone();
    a.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let mean = a.iter().sum::<f64>() / a.len() as f64;
    let std = (a.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / a.len() as f64).sqrt();

    let mut b = baselines.clone();
    b.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let bmean = b.iter().sum::<f64>() / b.len() as f64;
    let bstd = (b.iter().map(|v| (v - bmean).powi(2)).sum::<f64>() / b.len() as f64).sqrt();

    let mut pk: Vec<f64> = peak_idx.iter().map(|&v| v as f64).collect();
    pk.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pkmean = pk.iter().sum::<f64>() / pk.len() as f64;
    let pkstd = (pk.iter().map(|v| (v - pkmean).powi(2)).sum::<f64>() / pk.len() as f64).sqrt();

    let mut sp = spans.iter().map(|&v| v as f64).collect::<Vec<_>>();
    sp.sort_by(|x, y| x.partial_cmp(y).unwrap());

    println!(
        "=== Waveform stats (N={}{}) ===",
        amplitudes.len(),
        channel_filter
            .map(|c| format!(", ch={}", c))
            .unwrap_or_default()
    );
    println!(
        "amplitude (|peak − baseline|): mean={:.2} std={:.2} min={:.1} median={:.1} p95={:.1} max={:.1} ADC",
        mean,
        std,
        a[0],
        percentile(&a, 0.50),
        percentile(&a, 0.95),
        *a.last().unwrap()
    );
    println!(
        "baseline:                      mean={:.2} std={:.2} min={:.1} max={:.1}",
        bmean,
        bstd,
        b[0],
        *b.last().unwrap()
    );
    println!(
        "peak_index (sample position):  mean={:.1} std={:.1} min={:.0} median={:.0} max={:.0}",
        pkmean,
        pkstd,
        pk[0],
        percentile(&pk, 0.50),
        *pk.last().unwrap()
    );
    println!(
        "full span (max − min):         median={:.1} p95={:.1} max={:.1}",
        percentile(&sp, 0.50),
        percentile(&sp, 0.95),
        *sp.last().unwrap()
    );
    Ok(())
}

type FileModSummary = BTreeMap<u8, (f64, f64, usize)>;

fn process_one_file(
    path: &PathBuf,
) -> Result<(String, usize, FileModSummary), Box<dyn std::error::Error + Send + Sync>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut data_reader = DataFileReader::new(reader)?;

    let mut file_stats: BTreeMap<u8, ModStats> = BTreeMap::new();
    let mut file_events = 0usize;

    for block_result in data_reader.data_blocks() {
        let batch = block_result?;
        for event in &batch.events {
            file_events += 1;
            let fs = file_stats
                .entry(event.module)
                .or_insert_with(|| ModStats::new(event.timestamp_ns));
            fs.update(event.timestamp_ns);
        }
    }

    let file_summary: FileModSummary = file_stats
        .iter()
        .map(|(&mod_id, s)| (mod_id, (s.min_ts, s.max_ts, s.count)))
        .collect();

    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok((name, file_events, file_summary))
}

fn process_global(files: &[PathBuf]) -> Result<(), Box<dyn std::error::Error>> {
    let done_count = AtomicUsize::new(0);
    let total = files.len();

    let results: Vec<_> = files
        .par_iter()
        .map(|path| {
            let result = process_one_file(path);
            let done = done_count.fetch_add(1, Ordering::Relaxed) + 1;
            eprint!("\rProcessed {}/{} files...", done, total);
            result
        })
        .collect();
    eprintln!(" Done.");

    let mut per_file_results: Vec<(String, usize, FileModSummary)> = Vec::new();
    let mut global_stats: BTreeMap<u8, ModStats> = BTreeMap::new();
    let mut global_total = 0usize;

    let mut sorted_results: Vec<_> = results
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| -> Box<dyn std::error::Error> { e })?;
    sorted_results.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, file_events, file_summary) in sorted_results {
        global_total += file_events;
        for (&mod_id, &(min_ts, max_ts, count)) in &file_summary {
            let gs = global_stats
                .entry(mod_id)
                .or_insert_with(|| ModStats::new(min_ts));
            if min_ts < gs.min_ts {
                gs.min_ts = min_ts;
            }
            if max_ts > gs.max_ts {
                gs.max_ts = max_ts;
            }
            gs.count += count;
        }
        per_file_results.push((name, file_events, file_summary));
    }

    println!("\n=== Per-File Max Timestamp (seconds) ===");
    let mod_ids: Vec<u8> = global_stats.keys().copied().collect();
    print!("{:<12} {:>10}", "File", "Events");
    for &mod_id in &mod_ids {
        print!(" {:>12}", format!("Mod{} max", mod_id));
    }
    println!();

    for (name, events, file_summary) in &per_file_results {
        let seq = name.split('_').nth(1).unwrap_or("????");
        print!("{:<12} {:>10}", seq, events);
        for &mod_id in &mod_ids {
            if let Some(&(_min, max, _count)) = file_summary.get(&mod_id) {
                print!(" {:>12.1}", max / 1e9);
            } else {
                print!(" {:>12}", "-");
            }
        }
        println!();
    }

    println!(
        "\n=== GLOBAL Per-Module Summary ({} total events across {} files) ===",
        global_total,
        files.len()
    );
    println!(
        "{:<6} {:>12} {:>20} {:>20} {:>15} {:>12}",
        "Mod", "Events", "Min Timestamp (s)", "Max Timestamp (s)", "Range (s)", "Rate (evt/s)"
    );
    for (mod_id, stats) in &global_stats {
        let range_s = (stats.max_ts - stats.min_ts) / 1e9;
        let rate = if range_s > 0.0 {
            stats.count as f64 / range_s
        } else {
            0.0
        };
        println!(
            "Mod {:>2} {:>12} {:>20.3} {:>20.3} {:>15.3} {:>12.1}",
            mod_id,
            stats.count,
            stats.min_ts / 1e9,
            stats.max_ts / 1e9,
            range_s,
            rate,
        );
    }

    if let Some(mod0) = global_stats.get(&0) {
        let run_duration_s = (mod0.max_ts - mod0.min_ts) / 1e9;
        println!(
            "\n=== Coverage Analysis (vs Mod 0 = {:.1}s = {:.2}h) ===",
            run_duration_s,
            run_duration_s / 3600.0
        );
        println!("{:<6} {:>15} {:>12}", "Mod", "Coverage (s)", "Coverage (%)");
        for (mod_id, stats) in &global_stats {
            let range_s = (stats.max_ts - stats.min_ts) / 1e9;
            let pct = if run_duration_s > 0.0 {
                range_s / run_duration_s * 100.0
            } else {
                0.0
            };
            println!("Mod {:>2} {:>15.1} {:>11.1}%", mod_id, range_s, pct);
        }
    }

    Ok(())
}

#[derive(Default)]
struct Opts {
    count: Option<usize>,
    summary_only: bool,
    waveforms: Option<usize>,
    waveform_out: Option<PathBuf>,
    delta_t_hist: bool,
    wf_stats: Option<usize>,
    channel: Option<u8>,
    bins: usize,
    range_ns: f64,
}

fn print_usage(argv0: &str) {
    eprintln!(
        "Usage:\n  \
         {0} <file.delila> [count] [--summary]\n  \
         {0} <file.delila> --waveforms N [--ch C] [-o OUT.csv]\n  \
         {0} <file.delila> --delta-t-hist --ch C [--bins N] [--range-ns R]\n  \
         {0} --global <file1> <file2> ...",
        argv0
    );
}

fn parse_opts(args: &[String]) -> Result<(PathBuf, Opts), String> {
    if args.len() < 2 {
        return Err("missing file argument".into());
    }
    let path = PathBuf::from(&args[1]);
    let mut opts = Opts {
        bins: 201,
        range_ns: 1000.0,
        ..Default::default()
    };
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--summary" => opts.summary_only = true,
            "--waveforms" => {
                i += 1;
                opts.waveforms = Some(
                    args.get(i)
                        .and_then(|s| s.parse().ok())
                        .ok_or("--waveforms requires a number")?,
                );
            }
            "-o" | "--output" => {
                i += 1;
                opts.waveform_out = Some(PathBuf::from(args.get(i).ok_or("-o requires a path")?));
            }
            "--delta-t-hist" => opts.delta_t_hist = true,
            "--wf-stats" => {
                i += 1;
                opts.wf_stats = Some(
                    args.get(i)
                        .and_then(|s| s.parse().ok())
                        .ok_or("--wf-stats requires a number")?,
                );
            }
            "--ch" | "--channel" => {
                i += 1;
                opts.channel = Some(
                    args.get(i)
                        .and_then(|s| s.parse().ok())
                        .ok_or("--ch requires a u8")?,
                );
            }
            "--bins" => {
                i += 1;
                opts.bins = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .ok_or("--bins requires a usize")?;
            }
            "--range-ns" => {
                i += 1;
                opts.range_ns = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .ok_or("--range-ns requires an f64")?;
            }
            s if s.parse::<usize>().is_ok() && opts.count.is_none() => {
                opts.count = Some(s.parse().unwrap());
            }
            other => return Err(format!("unknown argument: {}", other)),
        }
        i += 1;
    }
    Ok((path, opts))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    if args[1] == "--global" {
        if args.len() < 3 {
            print_usage(&args[0]);
            std::process::exit(1);
        }
        let files: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
        process_global(&files)?;
        return Ok(());
    }

    let (path, opts) = parse_opts(&args).map_err(|e| -> Box<dyn std::error::Error> {
        eprintln!("Error: {}", e);
        print_usage(&args[0]);
        e.into()
    })?;

    if let Some(n) = opts.waveforms {
        let out = opts
            .waveform_out
            .unwrap_or_else(|| PathBuf::from("waveforms.csv"));
        process_waveform_dump(&path, n, opts.channel, &out)?;
    } else if opts.delta_t_hist {
        let ch = opts.channel.ok_or("--delta-t-hist requires --ch <C>")?;
        process_delta_t_hist(&path, ch, opts.bins, opts.range_ns)?;
    } else if let Some(n) = opts.wf_stats {
        process_waveform_stats(&path, n, opts.channel)?;
    } else {
        let count = opts.count.unwrap_or(50);
        process_single_file(&path, count, opts.summary_only)?;
    }

    Ok(())
}

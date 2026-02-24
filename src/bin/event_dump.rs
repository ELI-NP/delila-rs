//! Quick event dump tool - shows Mod, Ch, Timestamp for first N events
//! Usage: cargo run --release --bin event_dump -- <file> [count] [--summary]
//!        cargo run --release --bin event_dump -- --global <file1> <file2> ...

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
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
                println!(
                    "Event {:>8}: Mod={} Ch={:>2} Timestamp={:>20.3} ns ({:.6} s) Energy={:>5} EShort={:>5}",
                    total_events,
                    event.module,
                    event.channel,
                    event.timestamp_ns,
                    event.timestamp_ns / 1e9,
                    event.energy,
                    event.energy_short,
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

    // Process all files in parallel
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

    // Merge results (sorted by filename to preserve file order)
    let mut per_file_results: Vec<(String, usize, FileModSummary)> = Vec::new();
    let mut global_stats: BTreeMap<u8, ModStats> = BTreeMap::new();
    let mut global_total = 0usize;

    // Collect and sort by filename
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

    // Print per-file summary (compact: one line per file showing max_ts per mod)
    println!("\n=== Per-File Max Timestamp (seconds) ===");
    let mod_ids: Vec<u8> = global_stats.keys().copied().collect();
    print!("{:<12} {:>10}", "File", "Events");
    for &mod_id in &mod_ids {
        print!(" {:>12}", format!("Mod{} max", mod_id));
    }
    println!();

    for (name, events, file_summary) in &per_file_results {
        // Extract file sequence number from name
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

    // Print global summary
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

    // Expected run duration from Mod 0 (assumed to be ground truth)
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.delila> [count] [--summary]", args[0]);
        eprintln!("       {} --global <file1> <file2> ...", args[0]);
        std::process::exit(1);
    }

    if args[1] == "--global" {
        if args.len() < 3 {
            eprintln!("Usage: {} --global <file1> <file2> ...", args[0]);
            std::process::exit(1);
        }
        let files: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
        process_global(&files)?;
    } else {
        let path = PathBuf::from(&args[1]);
        let count: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
        let summary_only = args.iter().any(|a| a == "--summary");
        process_single_file(&path, count, summary_only)?;
    }

    Ok(())
}

//! Configure step benchmark
//!
//! Measures how long it takes to apply digitizer configuration parameters.
//! Usage: cargo run --bin configure_benchmark -- [config_file]
//! Default config: config/digitizers/psd1_test.json

use delila_rs::config::digitizer::DigitizerConfig;
use delila_rs::reader::CaenHandle;
use std::time::Instant;

const URL: &str = "dig1://caen.internal/usb?link_num=0";

fn main() {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/digitizers/psd1_test.json".to_string());

    println!("=== Configure Benchmark ===");
    println!("Config: {}", config_path);
    println!();

    // 1. Load config
    let t0 = Instant::now();
    let config = DigitizerConfig::load(&config_path).expect("Failed to load config");
    let load_time = t0.elapsed();
    println!("[1] Config load: {:?}", load_time);

    // 2. Generate CAEN parameters
    let t0 = Instant::now();
    let params = config.to_caen_parameters();
    let gen_time = t0.elapsed();
    println!("[2] Parameter generation: {:?} ({} params)", gen_time, params.len());

    // 3. Connect to digitizer
    let t0 = Instant::now();
    let handle = CaenHandle::open(URL).expect("Failed to connect");
    let connect_time = t0.elapsed();
    println!("[3] Connect: {:?}", connect_time);

    // 4. Check timebomb
    if let Ok(tb) = handle.get_value("/par/timebombdowncounter") {
        let secs: u32 = tb.parse().unwrap_or(0);
        if secs == 0 {
            println!("[!!!] TIMEBOMB EXPIRED!");
            return;
        }
        println!("    Timebomb: {}:{:02}", secs / 60, secs % 60);
    }

    // 5. Reset before benchmark
    println!("\n[4] Resetting digitizer...");
    let t0 = Instant::now();
    let _ = handle.send_command("/cmd/disarmacquisition");
    let _ = handle.send_command("/cmd/cleardata");
    let reset_time = t0.elapsed();
    println!("    Reset: {:?}", reset_time);

    // 6. Apply parameters with per-parameter timing
    println!("\n[5] Applying {} parameters:", params.len());
    let mut timings: Vec<(String, std::time::Duration, bool)> = Vec::new();
    let total_start = Instant::now();

    for param in &params {
        let t0 = Instant::now();
        let result = handle.set_value(&param.path, &param.value);
        let elapsed = t0.elapsed();
        let ok = result.is_ok();
        if !ok {
            println!(
                "    [ERR] {} = {} ({:?}): {}",
                param.path,
                param.value,
                elapsed,
                result.unwrap_err()
            );
        }
        timings.push((format!("{} = {}", param.path, param.value), elapsed, ok));
    }

    let total_apply = total_start.elapsed();

    // 7. Summary
    let success_count = timings.iter().filter(|(_, _, ok)| *ok).count();
    let fail_count = timings.len() - success_count;

    println!("\n=== Results ===");
    println!("Config load:     {:?}", load_time);
    println!("Param generation:{:?}", gen_time);
    println!("Connect:         {:?}", connect_time);
    println!("Reset:           {:?}", reset_time);
    println!("Apply total:     {:?} ({} params, {} ok, {} err)", total_apply, params.len(), success_count, fail_count);

    // Statistics
    let ok_timings: Vec<_> = timings.iter().filter(|(_, _, ok)| *ok).map(|(_, d, _)| *d).collect();
    if !ok_timings.is_empty() {
        let min = ok_timings.iter().min().unwrap();
        let max = ok_timings.iter().max().unwrap();
        let avg = total_apply / ok_timings.len() as u32;
        println!("\nPer-parameter: min={:?}, max={:?}, avg={:?}", min, max, avg);
    }

    // Top 10 slowest
    let mut sorted = timings.clone();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\nTop 10 slowest:");
    for (name, dur, ok) in sorted.iter().take(10) {
        let status = if *ok { "ok" } else { "ERR" };
        println!("  {:>8?} [{}] {}", dur, status, name);
    }

    // Full cycle estimate
    let full_cycle = reset_time + total_apply;
    println!("\n=== Estimate for Start auto-cycle ===");
    println!("Reset + Configure: {:?}", full_cycle);
    println!("(Arm + Start will add ~100-200ms)");
}

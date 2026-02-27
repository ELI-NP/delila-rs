//! DIG2 Trigger Loss Counter Verification Tool
//!
//! Tests ChTriggerCnt vs ChSavedEventCnt counters by intentionally
//! starving readout to cause buffer overflow on VX2730 (PSD2).
//!
//! Usage: trigger_loss_test_dig2 <URL> <CONFIG_JSON> [options]
//!
//! Example:
//!   cargo run --release --bin trigger_loss_test_dig2 -- \
//!     "dig2://172.18.4.56" \
//!     config/digitizers/psd2_56.json \
//!     --record-length-ns 8192 --enable-waveform

use delila_rs::config::digitizer::DigitizerConfig;
use delila_rs::reader::{CaenHandle, EndpointHandle};
use std::time::{Duration, Instant};

/// Per-channel counter snapshot from FPGA
#[derive(Debug, Default, Clone)]
struct ChannelCounters {
    realtime_ns: u64,
    deadtime_ns: u64,
    trigger_cnt: u64,
    saved_event_cnt: u64,
}

/// Phase statistics
#[derive(Debug, Default)]
struct PhaseStats {
    baseline: Vec<ChannelCounters>,
    final_counters: Vec<ChannelCounters>,
    total_bytes_read: u64,
    total_events_read: u64,
    read_calls: u64,
    duration_secs: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <URL> <CONFIG_JSON> [options]", args[0]);
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --phase1-secs N         Phase 1 duration (default: 10)");
        eprintln!("  --phase2-secs N         Phase 2 duration (default: 30)");
        eprintln!("  --delay-ms N            Phase 2 read delay (default: 1000)");
        eprintln!("  --record-length-ns N    Override record_length_ns");
        eprintln!("  --enable-waveform       Force waveform on (WaveTriggerSource=ChSelfTrigger)");
        eprintln!();
        eprintln!("Example:");
        eprintln!(
            "  {} \"dig2://172.18.4.56\" config/digitizers/psd2_56.json --record-length-ns 8192 --enable-waveform",
            args[0]
        );
        std::process::exit(1);
    }

    let url = &args[1];
    let config_path = &args[2];
    let phase1_secs = parse_arg(&args, "--phase1-secs", 10u64);
    let phase2_secs = parse_arg(&args, "--phase2-secs", 30u64);
    let delay_ms = parse_arg(&args, "--delay-ms", 1000u64);
    let record_length_override = parse_arg_opt(&args, "--record-length-ns");
    let enable_waveform = args.iter().any(|a| a == "--enable-waveform");

    println!("=== DIG2 Trigger Loss Counter Test ===");
    println!("URL:    {}", url);
    println!("Config: {}", config_path);
    println!(
        "Phase 1: {}s (normal) | Phase 2: {}s ({}ms delay)",
        phase1_secs, phase2_secs, delay_ms
    );

    // 1. Load config
    let mut config = DigitizerConfig::load(config_path).unwrap_or_else(|e| {
        eprintln!("Failed to load config: {}", e);
        std::process::exit(1);
    });
    println!(
        "Firmware: {:?}, Channels: {}",
        config.firmware, config.num_channels
    );

    // 2. Apply overrides
    let mut overrides_desc = Vec::new();
    if let Some(rl) = record_length_override {
        config.channel_defaults.record_length_ns = Some(rl as u32);
        for ch_config in config.channel_overrides.values_mut() {
            ch_config.record_length_ns = Some(rl as u32);
        }
        overrides_desc.push(format!("record_length_ns={}", rl));
    }
    if enable_waveform {
        config.channel_defaults.wave_trigger_source = Some("ChSelfTrigger".to_string());
        for ch_config in config.channel_overrides.values_mut() {
            ch_config.wave_trigger_source = Some("ChSelfTrigger".to_string());
        }
        overrides_desc.push("waveform=enabled".to_string());
    }
    if !overrides_desc.is_empty() {
        println!("Overrides: {}", overrides_desc.join(", "));
    }

    // 3. Detect enabled channels
    let enabled_channels = get_enabled_channels(&config);
    if enabled_channels.is_empty() {
        eprintln!("No enabled channels found in config!");
        std::process::exit(1);
    }
    println!(
        "\n--- Enabled Channels ---\n  {}",
        enabled_channels
            .iter()
            .map(|ch| format!("ch{}", ch))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // 4. Open handle
    let handle = CaenHandle::open(url).unwrap_or_else(|e| {
        eprintln!("Failed to connect: {}", e);
        std::process::exit(1);
    });

    // 5. Device info
    println!("\n--- Device Info ---");
    for path in ["/par/ModelName", "/par/SerialNum", "/par/FwType"] {
        if let Ok(value) = handle.get_value(path) {
            println!("  {}: {}", path, value);
        }
    }

    // 6. Apply config
    match handle.apply_config(&config) {
        Ok(n) => println!("\n[OK] Applied {} parameters", n),
        Err(e) => {
            eprintln!("Failed to apply config: {}", e);
            std::process::exit(1);
        }
    }

    // 7. Configure RAW endpoint (DIG2: include_n_events = true)
    let endpoint = handle.configure_endpoint(true).unwrap_or_else(|e| {
        eprintln!("Failed to configure endpoint: {}", e);
        std::process::exit(1);
    });

    // 8. Phase 1: Normal readout (baseline)
    println!("\n{}", "=".repeat(60));
    println!("=== Phase 1: Normal readout ({} seconds) ===", phase1_secs);
    arm_and_start(&handle);
    let phase1 = run_phase(&handle, &endpoint, &enabled_channels, phase1_secs, 0);
    stop_acquisition(&handle, &endpoint);
    print_phase_stats("Phase 1 (Normal)", &phase1, &enabled_channels);

    // 9. Phase 2: Delayed readout (trigger loss expected)
    println!("\n{}", "=".repeat(60));
    println!(
        "=== Phase 2: Delayed readout ({} seconds, {}ms delay) ===",
        phase2_secs, delay_ms
    );
    arm_and_start(&handle);
    let phase2 = run_phase(&handle, &endpoint, &enabled_channels, phase2_secs, delay_ms);
    stop_acquisition(&handle, &endpoint);
    print_phase_stats("Phase 2 (Delayed)", &phase2, &enabled_channels);

    // 10. Comparison summary
    print_comparison(&phase1, &phase2, &enabled_channels);
}

fn arm_and_start(handle: &CaenHandle) {
    handle
        .send_command("/cmd/cleardata")
        .expect("cleardata failed");
    handle
        .send_command("/cmd/armacquisition")
        .expect("arm failed");
    handle
        .send_command("/cmd/swstartacquisition")
        .expect("swstart failed");
    println!("  [OK] Acquisition started");
}

fn stop_acquisition(handle: &CaenHandle, endpoint: &EndpointHandle) {
    let _ = handle.send_command("/cmd/swstopacquisition");
    let _ = handle.send_command("/cmd/disarmacquisition");

    // Drain remaining buffered data
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut drained = 0usize;
    while let Ok(Some(raw)) = endpoint.read_data(200, &mut buf) {
        drained += raw.size;
    }
    if drained > 0 {
        println!("  Drained {} bytes after stop", drained);
    }
    let _ = handle.send_command("/cmd/cleardata");
    println!("  [OK] Acquisition stopped");
}

/// Read FPGA counters for enabled channels.
/// IMPORTANT: ChRealtimeMonitor must be read FIRST — this latches all other counters.
fn read_channel_counters(handle: &CaenHandle, enabled_channels: &[u8]) -> Vec<ChannelCounters> {
    let mut counters = Vec::with_capacity(enabled_channels.len());
    for &ch in enabled_channels {
        let mut c = ChannelCounters::default();
        // Step 1: Read realtime first (latches deadtime, trigger_cnt, saved_event_cnt)
        if let Ok(v) = handle.get_value(&format!("/ch/{}/par/ChRealtimeMonitor", ch)) {
            c.realtime_ns = v.parse().unwrap_or(0);
        }
        // Step 2: Read latched counters
        if let Ok(v) = handle.get_value(&format!("/ch/{}/par/ChDeadtimeMonitor", ch)) {
            c.deadtime_ns = v.parse().unwrap_or(0);
        }
        if let Ok(v) = handle.get_value(&format!("/ch/{}/par/ChTriggerCnt", ch)) {
            c.trigger_cnt = v.parse().unwrap_or(0);
        }
        if let Ok(v) = handle.get_value(&format!("/ch/{}/par/ChSavedEventCnt", ch)) {
            c.saved_event_cnt = v.parse().unwrap_or(0);
        }
        counters.push(c);
    }
    counters
}

fn run_phase(
    handle: &CaenHandle,
    endpoint: &EndpointHandle,
    enabled_channels: &[u8],
    duration_secs: u64,
    delay_ms: u64,
) -> PhaseStats {
    let mut stats = PhaseStats::default();
    let mut buffer = vec![0u8; 64 * 1024 * 1024]; // 64 MB

    // Baseline counter read
    stats.baseline = read_channel_counters(handle, enabled_channels);

    let start = Instant::now();
    let mut last_progress = Instant::now();

    while start.elapsed() < Duration::from_secs(duration_secs) {
        // Progress report every 5 seconds
        if last_progress.elapsed() >= Duration::from_secs(5) {
            let elapsed = start.elapsed().as_secs_f64();
            let snapshot = read_channel_counters(handle, enabled_channels);
            let total_lost: u64 = snapshot
                .iter()
                .zip(stats.baseline.iter())
                .map(|(f, b)| {
                    let trig = wrapping_diff_24bit(f.trigger_cnt, b.trigger_cnt);
                    let saved = wrapping_diff_24bit(f.saved_event_cnt, b.saved_event_cnt);
                    trig.saturating_sub(saved)
                })
                .sum();
            println!(
                "  [{:.0}s] {} events read, {:.1} MB, {} reads, {} lost (counter)",
                elapsed,
                stats.total_events_read,
                stats.total_bytes_read as f64 / 1_000_000.0,
                stats.read_calls,
                total_lost
            );
            last_progress = Instant::now();
        }

        // Read data (no decoding — just consume buffer)
        match endpoint.read_data(100, &mut buffer) {
            Ok(Some(raw_data)) => {
                stats.read_calls += 1;
                stats.total_bytes_read += raw_data.size as u64;
                stats.total_events_read += raw_data.n_events as u64;
            }
            Ok(None) => { /* timeout */ }
            Err(e) => {
                if e.code == -12 {
                    println!("  Stop signal received");
                    break;
                }
                eprintln!("  Read error (code {}): {}", e.code, e);
                break;
            }
        }

        // Intentional delay for Phase 2
        if delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }

    // Final counter read
    stats.final_counters = read_channel_counters(handle, enabled_channels);
    stats.duration_secs = start.elapsed().as_secs_f64();
    stats
}

fn print_phase_stats(label: &str, stats: &PhaseStats, enabled_channels: &[u8]) {
    println!("\n--- {} Results ---", label);
    println!("  Duration:     {:.1}s", stats.duration_secs);
    println!("  Read calls:   {}", stats.read_calls);
    println!(
        "  Total bytes:  {:.1} MB",
        stats.total_bytes_read as f64 / 1_000_000.0
    );
    println!(
        "  Events read:  {} (from n_events)",
        stats.total_events_read
    );

    println!(
        "\n  {:<4} {:>12} {:>12} {:>12} {:>14} {:>14}",
        "Ch", "TriggerCnt", "SavedCnt", "Lost", "Deadtime(ms)", "Realtime(ms)"
    );
    println!("  {}", "-".repeat(72));

    let mut total_trigger = 0u64;
    let mut total_saved = 0u64;
    let mut total_lost = 0u64;

    for (i, &ch) in enabled_channels.iter().enumerate() {
        let b = &stats.baseline[i];
        let f = &stats.final_counters[i];

        let trigger = wrapping_diff_24bit(f.trigger_cnt, b.trigger_cnt);
        let saved = wrapping_diff_24bit(f.saved_event_cnt, b.saved_event_cnt);
        let lost = trigger.saturating_sub(saved);
        let deadtime_ms = (f.deadtime_ns.wrapping_sub(b.deadtime_ns)) as f64 / 1_000_000.0;
        let realtime_ms = (f.realtime_ns.wrapping_sub(b.realtime_ns)) as f64 / 1_000_000.0;

        println!(
            "  {:<4} {:>12} {:>12} {:>12} {:>14.1} {:>14.1}",
            ch, trigger, saved, lost, deadtime_ms, realtime_ms
        );

        total_trigger += trigger;
        total_saved += saved;
        total_lost += lost;
    }

    println!("  {}", "-".repeat(72));
    println!(
        "  {:<4} {:>12} {:>12} {:>12}",
        "ALL", total_trigger, total_saved, total_lost
    );

    if total_trigger > 0 {
        let loss_pct = total_lost as f64 / total_trigger as f64 * 100.0;
        println!("\n  Loss rate: {:.2}%", loss_pct);
    }
}

fn print_comparison(phase1: &PhaseStats, phase2: &PhaseStats, enabled_channels: &[u8]) {
    let (p1_trig, p1_saved, p1_lost) = sum_counters(phase1, enabled_channels);
    let (p2_trig, p2_saved, p2_lost) = sum_counters(phase2, enabled_channels);

    println!("\n{}", "=".repeat(60));
    println!("=== COMPARISON SUMMARY ===");
    println!("{}", "=".repeat(60));
    println!("  {:>25} {:>15} {:>15}", "", "Phase 1", "Phase 2");
    println!(
        "  {:>25} {:>15.1} {:>15.1}",
        "Duration (s)", phase1.duration_secs, phase2.duration_secs
    );
    println!(
        "  {:>25} {:>15} {:>15}",
        "Events read", phase1.total_events_read, phase2.total_events_read
    );
    println!(
        "  {:>25} {:>15} {:>15}",
        "TriggerCnt (FPGA)", p1_trig, p2_trig
    );
    println!(
        "  {:>25} {:>15} {:>15}",
        "SavedEventCnt (FPGA)", p1_saved, p2_saved
    );
    println!(
        "  {:>25} {:>15} {:>15}",
        "Lost (Trig - Saved)", p1_lost, p2_lost
    );
    if p1_trig > 0 || p2_trig > 0 {
        let p1_pct = if p1_trig > 0 {
            format!("{:.2}%", p1_lost as f64 / p1_trig as f64 * 100.0)
        } else {
            "N/A".to_string()
        };
        let p2_pct = if p2_trig > 0 {
            format!("{:.2}%", p2_lost as f64 / p2_trig as f64 * 100.0)
        } else {
            "N/A".to_string()
        };
        println!("  {:>25} {:>15} {:>15}", "Loss rate", p1_pct, p2_pct);
    }

    // Verdict
    println!();
    if p1_lost == 0 && p2_lost > 0 {
        let loss_pct = p2_lost as f64 / p2_trig as f64 * 100.0;
        println!("  RESULT: Trigger loss counters WORKING correctly.");
        println!(
            "  Phase 1: 0 lost. Phase 2: {} lost ({:.2}%).",
            p2_lost, loss_pct
        );
    } else if p1_lost == 0 && p2_lost == 0 {
        println!("  RESULT: No trigger loss detected in either phase.");
        println!("  Try: --record-length-ns 16384 --enable-waveform --delay-ms 2000");
    } else if p1_lost > 0 {
        println!("  WARNING: Trigger losses detected even in Phase 1 (normal readout).");
        println!("  Phase 1 lost: {}, Phase 2 lost: {}", p1_lost, p2_lost);
    }
}

fn sum_counters(stats: &PhaseStats, enabled_channels: &[u8]) -> (u64, u64, u64) {
    let mut total_trig = 0u64;
    let mut total_saved = 0u64;
    let mut total_lost = 0u64;
    for (i, _ch) in enabled_channels.iter().enumerate() {
        let b = &stats.baseline[i];
        let f = &stats.final_counters[i];
        let trig = wrapping_diff_24bit(f.trigger_cnt, b.trigger_cnt);
        let saved = wrapping_diff_24bit(f.saved_event_cnt, b.saved_event_cnt);
        total_trig += trig;
        total_saved += saved;
        total_lost += trig.saturating_sub(saved);
    }
    (total_trig, total_saved, total_lost)
}

/// 24-bit wrapping-aware subtraction
fn wrapping_diff_24bit(current: u64, baseline: u64) -> u64 {
    if current >= baseline {
        current - baseline
    } else {
        (current + 0x100_0000) - baseline
    }
}

fn get_enabled_channels(config: &DigitizerConfig) -> Vec<u8> {
    let default_enabled = config
        .channel_defaults
        .enabled
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));

    let mut enabled = Vec::new();
    for ch in 0..config.num_channels {
        let ch_enabled = config
            .channel_overrides
            .get(&ch)
            .and_then(|c| c.enabled.as_deref())
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(default_enabled);
        if ch_enabled {
            enabled.push(ch);
        }
    }
    enabled
}

fn parse_arg(args: &[String], key: &str, default: u64) -> u64 {
    for i in 0..args.len().saturating_sub(1) {
        if args[i] == key {
            return args[i + 1].parse().unwrap_or(default);
        }
    }
    default
}

fn parse_arg_opt(args: &[String], key: &str) -> Option<u64> {
    for i in 0..args.len().saturating_sub(1) {
        if args[i] == key {
            return args[i + 1].parse().ok();
        }
    }
    None
}

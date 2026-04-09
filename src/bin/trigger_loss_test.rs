//! DIG1 Trigger Loss Flag Verification Tool
//!
//! Tests EXTRAS word flags (bit[15] Trigger Lost, bit[12] N Lost Trigger Counted)
//! by intentionally starving readout to cause buffer overflow.
//!
//! Usage: trigger_loss_test <URL> <CONFIG_JSON> [--phase1-secs N] [--phase2-secs N] [--delay-ms N]
//!
//! Example:
//!   cargo run --release --bin trigger_loss_test -- \
//!     "dig1://caen.internal/usb?link_num=0" \
//!     config/digitizers/psd1_test.json

use delila_rs::config::digitizer::DigitizerConfig;
use delila_rs::reader::decoder::{Psd1Config, Psd1Decoder, RawData as DecoderRawData};
use delila_rs::reader::{CaenHandle, EndpointHandle};
use std::time::{Duration, Instant};

// EXTRAS flag bit positions in EventData.flags (after decode_extras_word shift by 10)
const FLAG_TRIGGER_LOST: u32 = 0x20; // bit[5] <- EXTRAS bit[15]
const FLAG_OVER_RANGE: u32 = 0x10; // bit[4] <- EXTRAS bit[14]
const FLAG_N_TRIGGER_COUNTED: u32 = 0x08; // bit[3] <- EXTRAS bit[13]
const FLAG_N_LOST_COUNTED: u32 = 0x04; // bit[2] <- EXTRAS bit[12]

// Acquisition Status register
const ACQ_STATUS_REG: u32 = 0x8104;
const ACQ_STATUS_EVENT_FULL: u32 = 1 << 4;

// DPP Algorithm Control 2 register (per-channel: base + ch * 0x100)
const DPP_ALGO_CTRL2_BASE: u32 = 0x1084;
const DPP_ALGO_CTRL2_STEP: u32 = 0x0100;

#[derive(Debug, Default, Clone)]
struct ChannelStats {
    total_events: u64,
    trigger_lost: u64,
    over_range: u64,
    n_trigger_counted: u64,
    n_lost_counted: u64,
}

#[derive(Debug, Default)]
struct PhaseStats {
    channels: [ChannelStats; 16],
    buffer_full_count: u64,
    duration_secs: f64,
    read_calls: u64,
    total_bytes: u64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: {} <URL> <CONFIG_JSON> [--phase1-secs N] [--phase2-secs N] [--delay-ms N]",
            args[0]
        );
        eprintln!();
        eprintln!("Example:");
        eprintln!(
            "  {} \"dig1://caen.internal/usb?link_num=0\" config/digitizers/psd1_test.json",
            args[0]
        );
        std::process::exit(1);
    }

    let url = &args[1];
    let config_path = &args[2];
    let phase1_secs = parse_arg(&args, "--phase1-secs", 10u64);
    let phase2_secs = parse_arg(&args, "--phase2-secs", 30u64);
    let delay_ms = parse_arg(&args, "--delay-ms", 1000u64);

    println!("=== DIG1 Trigger Loss Flag Test ===");
    println!("URL:    {}", url);
    println!("Config: {}", config_path);
    println!(
        "Phase 1: {}s (normal) | Phase 2: {}s ({}ms delay)",
        phase1_secs, phase2_secs, delay_ms
    );

    // 1. Load config
    let config = DigitizerConfig::load(config_path).unwrap_or_else(|e| {
        eprintln!("Failed to load config: {}", e);
        std::process::exit(1);
    });
    println!(
        "Firmware: {:?}, Channels: {}",
        config.firmware, config.num_channels
    );

    // 2. Open handle
    let handle = CaenHandle::open(url).unwrap_or_else(|e| {
        eprintln!("Failed to connect: {}", e);
        std::process::exit(1);
    });

    // 3. Device info
    println!("\n--- Device Info ---");
    for path in ["/par/ModelName", "/par/SerialNum", "/par/FwType"] {
        if let Ok(value) = handle.get_value(path) {
            println!("  {}: {}", path, value);
        }
    }

    // 4. Apply config (ch_extras_opt=2 forced automatically for DIG1)
    match handle.apply_config(&config) {
        Ok(n) => println!("\n[OK] Applied {} parameters", n),
        Err(e) => {
            eprintln!("Failed to apply config: {}", e);
            std::process::exit(1);
        }
    }

    // 5. Read N values from register 0x1n84 bits[17:16]
    let n_values = read_n_values(&handle, config.num_channels);
    println!("\n--- N Lost Trigger Settings (register 0x1n84 bits[17:16]) ---");
    let first_n = n_values.first().copied().unwrap_or(1024);
    let all_same = n_values.iter().all(|&n| n == first_n);
    if all_same {
        println!(
            "  ch0-{}: N = {} (all channels)",
            config.num_channels - 1,
            first_n
        );
    } else {
        for (ch, n) in n_values.iter().enumerate() {
            println!("  ch{}: N = {}", ch, n);
        }
    }

    // 6. Configure RAW endpoint (DIG1: include_n_events = false)
    let endpoint = handle.configure_endpoint(false).unwrap_or_else(|e| {
        eprintln!("Failed to configure endpoint: {}", e);
        std::process::exit(1);
    });

    // 7. Create decoder
    let mut decoder = Psd1Decoder::new(Psd1Config {
        time_step_ns: 2.0,
        module_id: 0,
        dump_enabled: false,
    });

    // 8. Phase 1: Normal readout (baseline)
    println!("\n{}", "=".repeat(60));
    println!("=== Phase 1: Normal readout ({} seconds) ===", phase1_secs);
    arm_and_start(&handle);
    let phase1 = run_phase(&handle, &endpoint, &mut decoder, phase1_secs, 0);
    stop_acquisition(&handle, &endpoint);
    print_phase_stats("Phase 1 (Normal)", &phase1, &n_values);

    // 9. Phase 2: Delayed readout (trigger loss expected)
    println!("\n{}", "=".repeat(60));
    println!(
        "=== Phase 2: Delayed readout ({} seconds, {}ms delay) ===",
        phase2_secs, delay_ms
    );
    arm_and_start(&handle);
    let phase2 = run_phase(&handle, &endpoint, &mut decoder, phase2_secs, delay_ms);
    stop_acquisition(&handle, &endpoint);
    print_phase_stats("Phase 2 (Delayed)", &phase2, &n_values);

    // 10. Comparison summary
    print_comparison(&phase1, &phase2, &n_values);
}

fn arm_and_start(handle: &CaenHandle) {
    handle
        .send_command("/cmd/cleardata")
        .expect("cleardata failed");
    // DIG1 + START_MODE_SW: armacquisition = arm + start
    // DIG2: armacquisition + swstartacquisition
    handle
        .send_command("/cmd/armacquisition")
        .expect("arm failed");
    // Try swstartacquisition (DIG2), ignore error (DIG1 doesn't need it)
    let _ = handle.send_command("/cmd/swstartacquisition");
    println!("  [OK] Acquisition started");
}

fn stop_acquisition(handle: &CaenHandle, endpoint: &EndpointHandle) {
    // DIG1: disarm stops triggers. DIG2: swstopacquisition first.
    let _ = handle.send_command("/cmd/swstopacquisition");
    let _ = handle.send_command("/cmd/disarmacquisition");

    // Drain remaining buffered data (after disarm, no new data arrives)
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

fn run_phase(
    handle: &CaenHandle,
    endpoint: &EndpointHandle,
    decoder: &mut Psd1Decoder,
    duration_secs: u64,
    delay_ms: u64,
) -> PhaseStats {
    let mut stats = PhaseStats::default();
    let mut buffer = vec![0u8; 64 * 1024 * 1024]; // 64 MB
    let start = Instant::now();
    let mut last_status_poll = Instant::now();
    let poll_interval = Duration::from_millis(500);
    let mut last_progress = Instant::now();

    while start.elapsed() < Duration::from_secs(duration_secs) {
        // Periodic register poll for buffer full status
        if last_status_poll.elapsed() >= poll_interval {
            if let Ok(status) = handle.get_user_register(ACQ_STATUS_REG) {
                if status & ACQ_STATUS_EVENT_FULL != 0 {
                    stats.buffer_full_count += 1;
                    if stats.buffer_full_count <= 5 {
                        println!("  [!] Buffer FULL at {:.1}s", start.elapsed().as_secs_f64());
                    }
                }
            }
            last_status_poll = Instant::now();
        }

        // Progress report every 5 seconds
        if last_progress.elapsed() >= Duration::from_secs(5) {
            let total: u64 = stats.channels.iter().map(|c| c.total_events).sum();
            let elapsed = start.elapsed().as_secs_f64();
            println!(
                "  [{:.0}s] {} events ({:.0} Hz), {} reads, {} buffer-full",
                elapsed,
                total,
                total as f64 / elapsed,
                stats.read_calls,
                stats.buffer_full_count
            );
            last_progress = Instant::now();
        }

        // Read data
        match endpoint.read_data(100, &mut buffer) {
            Ok(Some(raw_data)) => {
                stats.read_calls += 1;
                stats.total_bytes += raw_data.size as u64;

                let decoder_raw = DecoderRawData {
                    data: raw_data.data,
                    size: raw_data.size,
                    n_events: raw_data.n_events,
                    host_receive_time: None,
                };
                let events = decoder.decode(&decoder_raw);

                for event in &events {
                    let ch = event.channel as usize;
                    if ch < 16 {
                        let cs = &mut stats.channels[ch];
                        cs.total_events += 1;
                        if event.flags & FLAG_TRIGGER_LOST != 0 {
                            cs.trigger_lost += 1;
                        }
                        if event.flags & FLAG_OVER_RANGE != 0 {
                            cs.over_range += 1;
                        }
                        if event.flags & FLAG_N_TRIGGER_COUNTED != 0 {
                            cs.n_trigger_counted += 1;
                        }
                        if event.flags & FLAG_N_LOST_COUNTED != 0 {
                            cs.n_lost_counted += 1;
                        }
                    }
                }
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

    stats.duration_secs = start.elapsed().as_secs_f64();
    stats
}

fn read_n_values(handle: &CaenHandle, num_channels: u8) -> Vec<u32> {
    let mut n_values = Vec::with_capacity(num_channels as usize);
    for ch in 0..num_channels as u32 {
        let addr = DPP_ALGO_CTRL2_BASE + ch * DPP_ALGO_CTRL2_STEP;
        match handle.get_user_register(addr) {
            Ok(val) => {
                let n_code = (val >> 16) & 0x3;
                let n = match n_code {
                    0b00 => 1024,
                    0b01 => 128,
                    0b10 => 8192,
                    _ => {
                        eprintln!("  Warning: ch{} has unknown N code {:#04b}", ch, n_code);
                        1024
                    }
                };
                n_values.push(n);
            }
            Err(e) => {
                eprintln!("  Warning: Cannot read 0x{:04X} for ch{}: {}", addr, ch, e);
                n_values.push(1024);
            }
        }
    }
    n_values
}

fn print_phase_stats(label: &str, stats: &PhaseStats, n_values: &[u32]) {
    println!("\n--- {} Results ---", label);
    println!("  Duration:     {:.1}s", stats.duration_secs);
    println!("  Read calls:   {}", stats.read_calls);
    println!(
        "  Total bytes:  {} ({:.1} MB)",
        stats.total_bytes,
        stats.total_bytes as f64 / 1_000_000.0
    );
    println!("  Buffer Full:  {} detections", stats.buffer_full_count);

    println!(
        "\n  {:<4} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "Ch", "Events", "TrigLost", "OverRange", "NTrigCnt", "NLostCnt", "Est.Lost"
    );
    println!("  {}", "-".repeat(72));

    let mut totals = ChannelStats::default();
    let mut total_est_lost: u64 = 0;
    let mut total_est_triggers: u64 = 0;

    for ch in 0..16usize {
        let cs = &stats.channels[ch];
        if cs.total_events == 0 {
            continue;
        }

        let n = n_values.get(ch).copied().unwrap_or(1024) as u64;
        let est_lost = cs.n_lost_counted * n;
        let est_triggers = cs.n_trigger_counted * n;

        println!(
            "  {:<4} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
            ch,
            cs.total_events,
            cs.trigger_lost,
            cs.over_range,
            cs.n_trigger_counted,
            cs.n_lost_counted,
            est_lost
        );

        totals.total_events += cs.total_events;
        totals.trigger_lost += cs.trigger_lost;
        totals.over_range += cs.over_range;
        totals.n_trigger_counted += cs.n_trigger_counted;
        totals.n_lost_counted += cs.n_lost_counted;
        total_est_lost += est_lost;
        total_est_triggers += est_triggers;
    }

    println!("  {}", "-".repeat(72));
    println!(
        "  {:<4} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "ALL",
        totals.total_events,
        totals.trigger_lost,
        totals.over_range,
        totals.n_trigger_counted,
        totals.n_lost_counted,
        total_est_lost
    );

    if stats.duration_secs > 0.0 {
        println!(
            "\n  Event rate: {:.0} Hz",
            totals.total_events as f64 / stats.duration_secs
        );
    }

    // Gemini review suggestion: detect small loss (bit[15] > 0 but bit[12] == 0)
    if totals.trigger_lost > 0 && totals.n_lost_counted == 0 {
        let n = n_values.first().copied().unwrap_or(1024);
        println!(
            "  Note: Trigger Lost flags detected but N-counter not yet fired (< {} lost triggers)",
            n
        );
    }

    // Sanity check: bit[13] count * N ≈ total_events
    if total_est_triggers > 0 {
        let ratio = totals.total_events as f64 / total_est_triggers as f64;
        println!(
            "  Trigger count check: events={}, est(NTrigCnt*N)={}, ratio={:.2}",
            totals.total_events, total_est_triggers, ratio
        );
    }
}

fn print_comparison(phase1: &PhaseStats, phase2: &PhaseStats, n_values: &[u32]) {
    let p1_events: u64 = phase1.channels.iter().map(|c| c.total_events).sum();
    let p2_events: u64 = phase2.channels.iter().map(|c| c.total_events).sum();
    let p1_lost: u64 = phase1.channels.iter().map(|c| c.trigger_lost).sum();
    let p2_lost: u64 = phase2.channels.iter().map(|c| c.trigger_lost).sum();
    let p1_n_lost: u64 = phase1.channels.iter().map(|c| c.n_lost_counted).sum();
    let p2_n_lost: u64 = phase2.channels.iter().map(|c| c.n_lost_counted).sum();
    let n = n_values.first().copied().unwrap_or(1024) as u64;

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
        "Total events", p1_events, p2_events
    );
    if phase1.duration_secs > 0.0 && phase2.duration_secs > 0.0 {
        println!(
            "  {:>25} {:>15.0} {:>15.0}",
            "Event rate (Hz)",
            p1_events as f64 / phase1.duration_secs,
            p2_events as f64 / phase2.duration_secs
        );
    }
    println!(
        "  {:>25} {:>15} {:>15}",
        "Trigger Lost (bit[15])", p1_lost, p2_lost
    );
    println!(
        "  {:>25} {:>15} {:>15}",
        "N Lost Counted (bit[12])", p1_n_lost, p2_n_lost
    );
    println!(
        "  {:>25} {:>15} {:>15}",
        "Est. lost triggers",
        p1_n_lost * n,
        p2_n_lost * n
    );
    println!(
        "  {:>25} {:>15} {:>15}",
        "Buffer Full detections", phase1.buffer_full_count, phase2.buffer_full_count
    );

    // Verdict
    println!();
    if p1_lost == 0 && p2_lost > 0 {
        println!("  RESULT: Trigger loss flags WORKING correctly.");
        println!(
            "  Phase 1 (normal): no losses. Phase 2 (delayed): {} loss flags detected.",
            p2_lost
        );
        if p2_n_lost > 0 {
            println!(
                "  N-counter also working: {} x {} = {} estimated lost triggers.",
                p2_n_lost,
                n,
                p2_n_lost * n
            );
        } else {
            println!(
                "  N-counter did not fire (total lost < N={}). bit[15] confirms loss occurred.",
                n
            );
        }
    } else if p1_lost == 0 && p2_lost == 0 {
        println!("  RESULT: No trigger loss detected in either phase.");
        println!("  Try increasing --delay-ms or increasing trigger rate.");
    } else if p1_lost > 0 {
        println!("  WARNING: Trigger losses detected even in Phase 1 (normal readout).");
        println!("  Trigger rate may exceed USB/optical readout bandwidth.");
        println!("  Phase 1 losses: {}, Phase 2 losses: {}", p1_lost, p2_lost);
    }
}

fn parse_arg(args: &[String], key: &str, default: u64) -> u64 {
    for i in 0..args.len().saturating_sub(1) {
        if args[i] == key {
            return args[i + 1].parse().unwrap_or(default);
        }
    }
    default
}

//! AMax Energy Verification Tool
//!
//! Compares Energy (OpenDPP, possibly 14-bit truncated) vs UserWord1 (raw Energy input from FW).
//! If ratio ≈ 4.0, confirms 2-bit right shift in OpenDPP Energy field.
//!
//! Usage: amax_energy_check [URL] [SECONDS]
//!   URL:     dig2://IP (default: dig2://172.18.4.56)
//!   SECONDS: acquisition duration (default: 5)

use delila_rs::reader::CaenHandle;
use std::time::Duration;

const DETAIL_EVENTS: u32 = 20;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let url = if args.len() > 1 {
        &args[1]
    } else {
        "dig2://172.18.4.56"
    };
    let duration_secs: u64 = if args.len() > 2 {
        args[2].parse().unwrap_or(5)
    } else {
        5
    };

    println!("=== AMax Energy Verification Tool ===");
    println!("URL: {}", url);
    println!("Duration: {}s", duration_secs);
    println!("Comparing: Energy (OpenDPP) vs UserWord1 (raw Energy input from FW)");

    let handle = match CaenHandle::open(url) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    // Device info
    println!("\n--- Device Info ---");
    for path in ["/par/ModelName", "/par/SerialNum", "/par/FwType"] {
        if let Ok(value) = handle.get_value(path) {
            println!("  {}: {}", path, value);
        }
    }

    // Configure channel 0
    println!("\n--- Channel Configuration ---");
    for ch in 0..64 {
        let enable = if ch == 0 { "True" } else { "False" };
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), enable);
    }
    println!("  ch0 enabled (negative polarity)");

    // Trigger: self-trigger via firmware OR gate (external signal)
    println!("\n--- Trigger Configuration ---");
    let _ = handle.set_value("/par/TestPulsePeriod", "0"); // disable test pulse
    let _ = handle.set_value("/par/AcqTriggerSource", "SwTrg");
    let _ = handle.set_value("/par/StartSource", "SWcmd");
    println!("  TestPulse disabled, AcqTriggerSource = SwTrg");
    println!("  Self-trigger via FW OR gate (external signal expected)");

    // Set MCA HLS registers
    println!("\n--- MCA HLS Registers ---");
    let core_regs: [(u32, u32, &str); 13] = [
        (0x0, 1, "POLARITY"),
        (0x1, 0, "OFFSET"),
        (0x2, 100, "THRS"),
        (0x3, 10, "TRIG_K"),
        (0x4, 12, "TRIG_M"),
        (0x5, 500, "TRAP_K"),
        (0x6, 550, "TRAP_M"),
        (0x7, 3499000, "DECONV_M"),
        (0x8, 2500, "TRAP_GAIN"),
        (0x9, 6, "BL_LEN"),
        (0xA, 1200, "BL_INIB"),
        (0xB, 510, "SAMPLE_POS"),
        (0xC, 1, "RUN_CFG"),
    ];
    for (addr, value, name) in &core_regs {
        let _ = handle.set_user_register(addr * 4, *value);
        println!("  {} = {}", name, value);
    }

    // AMax-specific registers (from amax_viewer defaults)
    println!("\n--- AMax Registers ---");
    let amax_regs: [(u32, u32, &str); 7] = [
        (0x14000, 200, "WINDOW_MAXIM"),
        (0x160000, 200, "baseline_delay"),
        (0x160001, 6, "baseline_len"),
        (0x160002, 1000, "baseline_offset"),
        (0x160003, 1000, "AMAX_window"),
        (0x160004, 4, "AMAX_delay"),
        (0x160005, 2, "AMAX_len"),
    ];
    for (addr, value, name) in &amax_regs {
        let _ = handle.set_user_register(addr * 4, *value);
        println!("  {} = {}", name, value);
    }

    // Read back key registers for diagnosis
    println!("\n--- Register Readback ---");
    let diag_regs: [(u32, &str); 5] = [
        (0xC00A, "amax_trigger_and"),
        (0xC00B, "en_trigger_and"),
        (0x160006, "debug_maxim_value"),
        (0x160007, "debug_amax_out"),
        (0x160008, "debug_baseline"),
    ];
    for (addr, name) in &diag_regs {
        match handle.get_user_register(addr * 4) {
            Ok(v) => println!("  {} (0x{:X}) = {} (0x{:08X})", name, addr, v, v),
            Err(e) => println!("  {} (0x{:X}) = ERROR: {}", name, addr, e),
        }
    }

    // Configure OpenDPP endpoint (without waveform)
    println!("\n--- Configuring OpenDPP Endpoint ---");
    let endpoint = match handle.configure_opendpp_endpoint(false) {
        Ok(ep) => {
            println!("  OpenDPP endpoint configured (no waveform)");
            ep
        }
        Err(e) => {
            eprintln!("  OpenDPP failed: {}", e);
            std::process::exit(1);
        }
    };

    // Start acquisition
    println!("\n--- Starting Acquisition ---");
    let _ = handle.send_command("/cmd/cleardata");
    let _ = handle.send_command("/cmd/armacquisition");
    let _ = handle.send_command("/cmd/swstartacquisition");
    println!("  Acquisition started");

    // Statistics
    let mut total_events: u64 = 0;
    let mut energy_zero: u64 = 0;
    let mut no_uw1: u64 = 0;
    let mut exact_match_2bit: u64 = 0;
    let mut ratio_sum: f64 = 0.0;
    let mut ratio_min: f64 = f64::MAX;
    let mut ratio_max: f64 = f64::MIN;
    let mut ratio_count: u64 = 0;

    let mut user_info_buffer = [0u64; 16];

    // Live register polling
    let poll_regs: [(u32, &str); 3] = [
        (0xC00A, "amax_trig_and"),
        (0xC00B, "en_trig_and"),
        (0x160009, "rangee"),
    ];
    let mut last_poll = std::time::Instant::now();
    let poll_interval = Duration::from_millis(500);

    println!(
        "\n--- Reading Data ({}s, first {} events detailed) ---",
        duration_secs, DETAIL_EVENTS
    );

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(duration_secs) {
        // Periodic register readback
        if last_poll.elapsed() >= poll_interval {
            let vals: Vec<String> = poll_regs
                .iter()
                .map(|(addr, name)| {
                    match handle.get_user_register(addr * 4) {
                        Ok(v) => format!("{}={}", name, v),
                        Err(_) => format!("{}=ERR", name),
                    }
                })
                .collect();
            println!("  [REG @{:.1}s] {}", start.elapsed().as_secs_f64(), vals.join("  "));
            last_poll = std::time::Instant::now();
        }

        match endpoint.read_opendpp_event(100, &mut user_info_buffer) {
            Ok(Some(event)) => {
                total_events += 1;

                let energy = event.energy as u64;
                let uw0 = event.user_info.first().copied();
                let uw1 = event.user_info.get(1).copied();

                if energy == 0 {
                    energy_zero += 1;
                }

                match uw1 {
                    Some(raw_e) => {
                        // Check exact 2-bit shift match
                        if energy << 2 == raw_e {
                            exact_match_2bit += 1;
                        }

                        // Ratio statistics (skip energy=0)
                        if energy > 0 {
                            let ratio = raw_e as f64 / energy as f64;
                            ratio_sum += ratio;
                            ratio_count += 1;
                            if ratio < ratio_min {
                                ratio_min = ratio;
                            }
                            if ratio > ratio_max {
                                ratio_max = ratio;
                            }
                        }

                        // Detail output
                        if total_events <= DETAIL_EVENTS as u64 {
                            let ratio_str = if energy > 0 {
                                format!("{:.3}", raw_e as f64 / energy as f64)
                            } else {
                                "N/A".to_string()
                            };
                            let diff = if energy > 0 {
                                format!("{}", raw_e as i64 - ((energy << 2) as i64))
                            } else {
                                "N/A".to_string()
                            };
                            println!(
                                "[Event #{:>3}] Ch={}  Energy={:<5}  UW0(amax)={:<10}  UW1(raw_E)={:<10}  ratio={}  diff={}",
                                total_events,
                                event.channel,
                                energy,
                                uw0.map_or("N/A".to_string(), |v| v.to_string()),
                                raw_e,
                                ratio_str,
                                diff,
                            );
                        }
                    }
                    None => {
                        no_uw1 += 1;
                        if total_events <= DETAIL_EVENTS as u64 {
                            println!(
                                "[Event #{:>3}] Ch={}  Energy={:<5}  UW0(amax)={:<10}  UW1=MISSING ({} user words)",
                                total_events,
                                event.channel,
                                energy,
                                uw0.map_or("N/A".to_string(), |v| v.to_string()),
                                event.user_info.len(),
                            );
                        }
                    }
                }
            }
            Ok(None) => {
                // Timeout - continue
            }
            Err(e) => {
                eprintln!("\nRead error: {}", e);
                break;
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();

    // Stop: drain remaining events before disarm
    let _ = handle.send_command("/cmd/swstopacquisition");
    while let Ok(Some(_)) = endpoint.read_opendpp_event(100, &mut user_info_buffer) {}
    let _ = handle.send_command("/cmd/disarmacquisition");
    let _ = handle.send_command("/cmd/cleardata");

    // Read registers after acquisition (may have updated values)
    println!("\n--- Register Readback (after acquisition) ---");
    for (addr, name) in &diag_regs {
        match handle.get_user_register(addr * 4) {
            Ok(v) => println!("  {} (0x{:X}) = {} (0x{:08X})", name, addr, v, v),
            Err(e) => println!("  {} (0x{:X}) = ERROR: {}", name, addr, e),
        }
    }

    // Summary
    println!("\n=== Summary ({} events, {:.1}s) ===", total_events, elapsed);
    if ratio_count > 0 {
        println!(
            "  Ratio (UW1/Energy): avg={:.3}  min={:.3}  max={:.3}",
            ratio_sum / ratio_count as f64,
            ratio_min,
            ratio_max,
        );
    } else {
        println!("  Ratio: no valid data (all energy=0 or no UW1)");
    }
    let comparable = total_events - energy_zero - no_uw1;
    if comparable > 0 {
        println!(
            "  Exact match (Energy<<2 == UW1): {}/{} ({:.2}%)",
            exact_match_2bit,
            comparable,
            exact_match_2bit as f64 / comparable as f64 * 100.0,
        );
    }
    println!("  Energy=0 events: {}", energy_zero);
    println!("  Missing UW1 events: {}", no_uw1);
    if elapsed > 0.0 {
        println!("  Rate: {:.1} Hz", total_events as f64 / elapsed);
    }
}

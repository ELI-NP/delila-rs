//! PSD1 Pre-trigger / Pre-gate timing verification
//!
//! Tests:
//! 1. Direct parameter read/write
//! 2. CFD mode timing with gate visualization

use delila_rs::reader::decoder::{Psd1Config, Psd1Decoder, RawData as DecoderRawData};
use delila_rs::reader::CaenHandle;
use std::thread;
use std::time::Duration;

const URL: &str = "dig1://caen.internal/usb?link_num=0";

fn main() {
    println!("=== PSD1 CFD Timing Verification ===\n");

    let handle = CaenHandle::open(URL).expect("Failed to connect");
    println!("[OK] Connected");

    // Check timebomb
    if let Ok(tb) = handle.get_value("/par/timebombdowncounter") {
        let secs: u32 = tb.parse().unwrap_or(0);
        if secs == 0 {
            println!("[!!!] TIMEBOMB EXPIRED!");
            return;
        }
        println!("[OK] Timebomb: {}:{:02}\n", secs / 60, secs % 60);
    }

    // Reset
    let _ = handle.send_command("/cmd/reset");
    thread::sleep(Duration::from_millis(100));

    let ch = 4;

    // =====================================================
    // TEST: CFD mode with specific timing parameters
    // =====================================================
    println!("=== CFD Mode Timing Test ===\n");

    // Global settings
    set(&handle, "/par/reclen", "1024"); // 1024 ns = 512 samples
    set(&handle, "/par/waveforms", "TRUE");
    set(&handle, "/par/extras", "TRUE");

    // Set analog probe 1 = INPUT, digital probe 1 = GATE
    set(&handle, "/vtrace/0/par/vtrace_probe", "VPROBE_INPUT");
    set(&handle, "/vtrace/2/par/vtrace_probe", "VPROBE_GATE");

    // Channel settings - CFD mode
    set(&handle, &format!("/ch/{}/par/ch_enabled", ch), "True");
    set(
        &handle,
        &format!("/ch/{}/par/ch_polarity", ch),
        "POLARITY_NEGATIVE",
    );
    set(&handle, &format!("/ch/{}/par/ch_dcoffset", ch), "50");
    set(
        &handle,
        &format!("/ch/{}/par/ch_discr_mode", ch),
        "DISCR_MODE_CFD",
    );
    set(&handle, &format!("/ch/{}/par/ch_threshold", ch), "250");
    set(
        &handle,
        &format!("/ch/{}/par/ch_self_trg_enable", ch),
        "TRUE",
    );

    // CFD parameters
    set(&handle, &format!("/ch/{}/par/ch_cfd_delay", ch), "4"); // 4 samples = 8ns
    set(
        &handle,
        &format!("/ch/{}/par/ch_cfd_fraction", ch),
        "CFD_FRACTLIST_25",
    );
    set(
        &handle,
        &format!("/ch/{}/par/ch_cfd_smoothexp", ch),
        "CFD_SMOOTH_EXP_8",
    );

    // CRITICAL: Timing parameters to test
    // Values in SAMPLES (not ns!)
    let pre_trigger_samples = 160; // 320 ns
    let gate_pre_samples = 128; // 256 ns
    let gate_long_samples = 256; // 512 ns
    let gate_short_samples = 64; // 128 ns

    println!("\n--- Setting timing parameters (in samples) ---");
    set(
        &handle,
        &format!("/ch/{}/par/ch_pretrg", ch),
        &pre_trigger_samples.to_string(),
    );
    set(
        &handle,
        &format!("/ch/{}/par/ch_gatepre", ch),
        &gate_pre_samples.to_string(),
    );
    set(
        &handle,
        &format!("/ch/{}/par/ch_gate", ch),
        &gate_long_samples.to_string(),
    );
    set(
        &handle,
        &format!("/ch/{}/par/ch_gateshort", ch),
        &gate_short_samples.to_string(),
    );

    // Disable other channels
    for other_ch in 0..8 {
        if other_ch != ch {
            let _ = handle.set_value(&format!("/ch/{}/par/ch_enabled", other_ch), "False");
        }
    }

    // Read back all timing parameters
    println!("\n=== Readback Verification ===");
    show_timing_params(&handle, ch);

    // Show expected timing diagram
    println!("\n=== Expected Timing (CFD mode) ===");
    println!(
        "  Pre-trigger: {} samples ({} ns) - data before trigger point",
        pre_trigger_samples,
        pre_trigger_samples * 2
    );
    println!(
        "  Pre-gate:    {} samples ({} ns) - gate opens before trigger",
        gate_pre_samples,
        gate_pre_samples * 2
    );
    println!(
        "  Gate Long:   {} samples ({} ns) - total gate width",
        gate_long_samples,
        gate_long_samples * 2
    );
    println!(
        "  Gate Short:  {} samples ({} ns)",
        gate_short_samples,
        gate_short_samples * 2
    );
    println!("\n  Timeline (samples from waveform start):");
    let trigger_pos: u32 = pre_trigger_samples;
    let gate_start = trigger_pos.saturating_sub(gate_pre_samples);
    let gate_end = gate_start + gate_long_samples;
    println!(
        "    0 -------- {} (gate start) -------- {} (trigger) -------- {} (gate end) -------- 512 (record end)",
        gate_start, trigger_pos, gate_end
    );

    // Acquire data
    println!("\n=== Data Acquisition ===");
    let endpoint = handle
        .configure_endpoint(false)
        .expect("Failed to configure endpoint");
    let _ = handle.send_command("/cmd/cleardata");
    handle
        .send_command("/cmd/armacquisition")
        .expect("Arm failed");

    println!("Acquiring for 1 second...");
    thread::sleep(Duration::from_millis(1000));

    let mut buffer = vec![0u8; 64 * 1024 * 1024];
    match endpoint.read_data(1000, &mut buffer) {
        Ok(Some(raw_data)) => {
            println!("Read {} bytes, {} events", raw_data.size, raw_data.n_events);

            let config = Psd1Config {
                time_step_ns: 2.0,
                module_id: 0,
                dump_enabled: false,
            };
            let mut decoder = Psd1Decoder::new(config);
            let decoder_raw = DecoderRawData {
                data: raw_data.data,
                size: raw_data.size,
                n_events: raw_data.n_events,
                host_receive_time: None,
            };
            let events = decoder.decode(&decoder_raw);

            println!("\nDecoded {} events", events.len());

            // Show a few pulses with gate analysis
            let mut shown = 0;
            for event in events.iter() {
                if event.channel != ch as u8 {
                    continue;
                }
                if let Some(ref wf) = event.waveform {
                    let samples = &wf.analog_probe1;
                    let dp1 = &wf.digital_probe1; // Gate signal

                    let min = *samples.iter().min().unwrap_or(&0);
                    let max = *samples.iter().max().unwrap_or(&0);
                    let range = max - min;

                    if range > 200 {
                        let min_pos = samples.iter().position(|&x| x == min).unwrap_or(0);

                        // Find gate edges
                        let gate_on_start = dp1.iter().position(|&x| x > 0);
                        let gate_on_end = dp1.iter().rposition(|&x| x > 0);

                        println!("\n--- Pulse {} ---", shown + 1);
                        println!(
                            "  Waveform: min={} at pos={}, max={}, range={}",
                            min, min_pos, max, range
                        );
                        println!(
                            "  Gate (DP1): start={:?}, end={:?}",
                            gate_on_start, gate_on_end
                        );
                        println!(
                            "  Charge: long={}, short={}",
                            event.energy, event.energy_short
                        );

                        // Check if pulse is inside gate
                        if let (Some(gs), Some(ge)) = (gate_on_start, gate_on_end) {
                            if min_pos >= gs && min_pos <= ge {
                                println!("  [OK] Pulse minimum IS inside gate");
                            } else {
                                println!(
                                    "  [!!!] Pulse minimum OUTSIDE gate! (gs={}, min_pos={}, ge={})",
                                    gs, min_pos, ge
                                );
                            }
                        }

                        shown += 1;
                        if shown >= 3 {
                            break;
                        }
                    }
                }
            }

            if shown == 0 {
                println!("No significant pulses detected on ch{}!", ch);
            }
        }
        Ok(None) => println!("Timeout - no data"),
        Err(e) => println!("Read error: {}", e),
    }

    let _ = handle.send_command("/cmd/disarmacquisition");
    println!("\n=== Done ===");
}

fn set(handle: &CaenHandle, path: &str, value: &str) {
    match handle.set_value(path, value) {
        Ok(()) => println!("  Set {} = {}", path, value),
        Err(e) => println!("  [ERR] {} = {}: {}", path, value, e),
    }
}

fn show_timing_params(handle: &CaenHandle, ch: usize) {
    let params = [
        ("ch_pretrg", "Pre-trigger"),
        ("ch_gatepre", "Pre-gate"),
        ("ch_gate", "Gate Long"),
        ("ch_gateshort", "Gate Short"),
        ("ch_cfd_delay", "CFD Delay"),
    ];

    for (param, desc) in params {
        let path = format!("/ch/{}/par/{}", ch, param);
        match handle.get_value(&path) {
            Ok(v) => {
                let samples: i32 = v.parse().unwrap_or(0);
                println!("  {}: {} samples ({} ns)", desc, samples, samples * 2);
            }
            Err(e) => println!("  {}: error - {}", desc, e),
        }
    }
}

//! PSD1 Waveform Debug Tool - Simple hardcoded test

use delila_rs::reader::decoder::{Psd1Config, Psd1Decoder, RawData as DecoderRawData};
use delila_rs::reader::CaenHandle;
use std::thread;
use std::time::Duration;

const URL: &str = "dig1://caen.internal/usb?link_num=0";

fn main() {
    println!("=== PSD1 Waveform Test ===\n");

    // Connect
    let handle = CaenHandle::open(URL).expect("Failed to connect");
    println!("[OK] Connected");

    // Check timebomb
    if let Ok(tb) = handle.get_value("/par/timebombdowncounter") {
        let secs: u32 = tb.parse().unwrap_or(0);
        if secs == 0 {
            println!("[!!!] TIMEBOMB EXPIRED! Power cycle the digitizer.");
            return;
        }
        println!("[OK] Timebomb: {}:{:02} remaining", secs / 60, secs % 60);
    }

    // Reset first to ensure clean state
    println!("\n--- Reset ---");
    match handle.send_command("/cmd/reset") {
        Ok(()) => println!("  Reset OK"),
        Err(e) => println!("  Reset failed: {}", e),
    }
    thread::sleep(Duration::from_millis(100)); // Wait for reset

    // Apply all settings BEFORE arm
    println!("\n--- Applying Settings ---");

    // Global settings
    set_param(&handle, "/par/reclen", "800"); // 800 ns = 400 samples
    set_param(&handle, "/par/waveforms", "TRUE"); // Enable waveform
    set_param(&handle, "/par/extras", "TRUE"); // Enable extras
    set_param(&handle, "/vtrace/0/par/vtrace_probe", "VPROBE_INPUT"); // Analog Probe 1 = Input

    // Channel 4 settings (where pulse input is)
    let ch = 4;
    set_param(&handle, &format!("/ch/{}/par/ch_enabled", ch), "True");
    set_param(
        &handle,
        &format!("/ch/{}/par/ch_polarity", ch),
        "POLARITY_NEGATIVE",
    );
    set_param(&handle, &format!("/ch/{}/par/ch_dcoffset", ch), "50"); // 50%
    set_param(&handle, &format!("/ch/{}/par/ch_pretrg", ch), "320"); // 320 ns pre-trigger = 160 samples
    set_param(&handle, &format!("/ch/{}/par/ch_threshold", ch), "250"); // 250 LSB
    set_param(
        &handle,
        &format!("/ch/{}/par/ch_discr_mode", ch),
        "DISCR_MODE_LED",
    ); // Leading Edge
    set_param(
        &handle,
        &format!("/ch/{}/par/ch_self_trg_enable", ch),
        "TRUE",
    );

    // Disable other channels to reduce noise
    for other_ch in 0..8 {
        if other_ch != ch {
            let _ = handle.set_value(&format!("/ch/{}/par/ch_enabled", other_ch), "False");
        }
    }

    // Show current settings
    println!("\n--- Current Settings ---");
    show_param(&handle, "/par/reclen");
    show_param(&handle, "/par/waveforms");
    show_param(&handle, "/vtrace/0/par/vtrace_probe");
    show_param(&handle, &format!("/ch/{}/par/ch_pretrg", ch));
    show_param(&handle, &format!("/ch/{}/par/ch_threshold", ch));
    show_param(&handle, &format!("/ch/{}/par/ch_discr_mode", ch));

    // Configure endpoint
    println!("\n--- Data Acquisition ---");
    let endpoint = handle
        .configure_endpoint(false)
        .expect("Failed to configure endpoint");

    // Clear old data
    let _ = handle.send_command("/cmd/cleardata");

    // Arm and acquire
    handle
        .send_command("/cmd/armacquisition")
        .expect("Arm failed");
    println!("Acquiring for 1 second...");
    thread::sleep(Duration::from_millis(1000));

    // Read and decode
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
            };
            let events = decoder.decode(&decoder_raw);

            println!("\nDecoded {} events", events.len());

            // Show ch4 events with pulses
            let mut shown = 0;
            for event in events.iter() {
                if event.channel != ch as u8 {
                    continue;
                }
                if let Some(ref wf) = event.waveform {
                    let samples = &wf.analog_probe1;
                    let min = *samples.iter().min().unwrap_or(&0);
                    let max = *samples.iter().max().unwrap_or(&0);
                    let range = max - min;

                    if range > 100 {
                        let min_pos = samples.iter().position(|&x| x == min).unwrap_or(0);
                        println!(
                            "\nPulse: min={} max={} range={} min_pos={}",
                            min, max, range, min_pos
                        );
                        println!(
                            "  First 30 samples: {:?}",
                            &samples[..30.min(samples.len())]
                        );
                        shown += 1;
                        if shown >= 3 {
                            break;
                        }
                    }
                }
            }

            if shown == 0 {
                println!("No pulses detected on ch{}!", ch);
            }
        }
        Ok(None) => println!("Timeout - no data"),
        Err(e) => println!("Read error: {}", e),
    }

    // Cleanup
    let _ = handle.send_command("/cmd/disarmacquisition");
    println!("\n=== Done ===");
}

fn set_param(handle: &CaenHandle, path: &str, value: &str) {
    match handle.set_value(path, value) {
        Ok(()) => println!("  Set {} = {}", path, value),
        Err(e) => println!("  [ERR] {} = {}: {}", path, value, e),
    }
}

fn show_param(handle: &CaenHandle, path: &str) {
    match handle.get_value(path) {
        Ok(v) => println!("  {} = {}", path, v),
        Err(e) => println!("  {} = [error: {}]", path, e),
    }
}

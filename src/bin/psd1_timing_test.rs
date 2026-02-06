//! PSD1 Pre-trigger / Pre-gate timing test

use delila_rs::reader::CaenHandle;
use std::thread;
use std::time::Duration;

const URL: &str = "dig1://caen.internal/usb?link_num=0";

fn main() {
    println!("=== PSD1 Timing Parameter Test ===\n");

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

    // Show default values first
    println!("=== Default Values (after reset) ===");
    show_timing_params(&handle, ch);

    // Test pre-trigger settings
    println!("\n=== Testing Pre-trigger ===");
    // PSD1: ch_pretrg is in samples (1 sample = 2ns for DT5730B)
    // Range: 40-2016 samples
    let test_pretrg_samples = [40, 80, 160, 320];
    for samples in test_pretrg_samples {
        println!("\n--- Setting pre-trigger to {} samples ({} ns) ---", samples, samples * 2);
        match handle.set_value(&format!("/ch/{}/par/ch_pretrg", ch), &samples.to_string()) {
            Ok(()) => {
                // Read back
                match handle.get_value(&format!("/ch/{}/par/ch_pretrg", ch)) {
                    Ok(v) => println!("  Read back: {} samples", v),
                    Err(e) => println!("  Read error: {}", e),
                }
            }
            Err(e) => println!("  Set error: {}", e),
        }
    }

    // Test pre-gate settings
    println!("\n=== Testing Pre-gate (ch_gatepre) ===");
    // PSD1: ch_gatepre is in samples, range 0-510
    let test_gatepre_samples = [0, 32, 64, 128, 256];
    for samples in test_gatepre_samples {
        println!("\n--- Setting pre-gate to {} samples ({} ns) ---", samples, samples * 2);
        match handle.set_value(&format!("/ch/{}/par/ch_gatepre", ch), &samples.to_string()) {
            Ok(()) => {
                match handle.get_value(&format!("/ch/{}/par/ch_gatepre", ch)) {
                    Ok(v) => println!("  Read back: {} samples", v),
                    Err(e) => println!("  Read error: {}", e),
                }
            }
            Err(e) => println!("  Set error: {}", e),
        }
    }

    // Test gate long
    println!("\n=== Testing Gate Long (ch_gate) ===");
    let test_gate_samples = [64, 128, 256, 512];
    for samples in test_gate_samples {
        println!("\n--- Setting gate to {} samples ({} ns) ---", samples, samples * 2);
        match handle.set_value(&format!("/ch/{}/par/ch_gate", ch), &samples.to_string()) {
            Ok(()) => {
                match handle.get_value(&format!("/ch/{}/par/ch_gate", ch)) {
                    Ok(v) => println!("  Read back: {} samples", v),
                    Err(e) => println!("  Read error: {}", e),
                }
            }
            Err(e) => println!("  Set error: {}", e),
        }
    }

    // Final timing summary
    println!("\n=== Final Timing Summary ===");
    show_timing_params(&handle, ch);

    println!("\n=== Done ===");
}

fn show_timing_params(handle: &CaenHandle, ch: usize) {
    let params = [
        ("ch_pretrg", "Pre-trigger"),
        ("ch_gatepre", "Pre-gate"),
        ("ch_gate", "Gate Long"),
        ("ch_gateshort", "Gate Short"),
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

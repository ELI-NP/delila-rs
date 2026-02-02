//! AMax OpenDPP Endpoint Test
//!
//! Uses OpenDPP endpoint instead of Raw to get decoded data.

use delila_rs::reader::CaenHandle;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let url = if args.len() > 1 {
        &args[1]
    } else {
        "dig2://172.18.4.56"
    };

    println!("=== AMax OpenDPP Endpoint Test ===");
    println!("URL: {}", url);

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
    handle.set_value("/ch/0/par/chenable", "True").ok();
    println!("  ch0 enabled");
    for ch in 1..32 {
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), "False");
    }

    // Set MCA HLS registers
    println!("\n--- MCA HLS Registers ---");
    let core_regs: [(u32, u32, &str); 13] = [
        (0x0, 0, "POLARITY"),
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

    // Configure OpenDPP endpoint (without waveform for smaller data)
    println!("\n--- Configuring OpenDPP Endpoint ---");
    let endpoint = match handle.configure_opendpp_endpoint(false) {
        Ok(ep) => {
            println!("  OpenDPP endpoint configured (no waveform)");
            ep
        }
        Err(e) => {
            println!("  OpenDPP failed: {}", e);
            std::process::exit(1);
        }
    };

    // Start acquisition
    println!("\n--- Starting Acquisition ---");
    let _ = handle.send_command("/cmd/cleardata");
    let _ = handle.send_command("/cmd/armacquisition");
    let _ = handle.send_command("/cmd/swstartacquisition");
    println!("  Acquisition started");

    // Read data using OpenDPP event-by-event
    println!("\n--- Reading Data (3 seconds) ---");
    let mut user_info_buffer = [0u64; 16]; // Buffer for user info words
    let mut total_events = 0u32;

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        match endpoint.read_opendpp_event(100, &mut user_info_buffer) {
            Ok(Some(event)) => {
                total_events += 1;
                if total_events <= 10 {
                    println!("\n  [Event #{}]", total_events);
                    println!("    Channel:   {}", event.channel);
                    println!(
                        "    Timestamp: {} ({:.3} us)",
                        event.timestamp,
                        event.timestamp as f64 * 8.0 / 1000.0
                    );
                    println!("    FineTS:    {}", event.fine_timestamp);
                    println!("    Energy:    {}", event.energy);
                    println!("    PSD:       {}", event.psd);
                    println!(
                        "    Flags A/B: 0x{:02X} / 0x{:03X}",
                        event.flags_a, event.flags_b
                    );
                    println!("    UserInfo:  {} words", event.user_info.len());
                    for (i, &ui) in event.user_info.iter().enumerate() {
                        println!("      [{}]: 0x{:016X} ({})", i, ui, ui);
                    }
                    println!("    EventSize: {} bytes", event.event_size);
                }
            }
            Ok(None) => {
                // Timeout - continue
            }
            Err(e) => {
                println!("\n  Read error: {}", e);
                break;
            }
        }
    }

    // Stop
    println!("\n\n--- Stopping Acquisition ---");
    let _ = handle.send_command("/cmd/swstopacquisition");
    let _ = handle.send_command("/cmd/disarmacquisition");

    println!("\n=== Results ===");
    println!("Total events: {}", total_events);
    println!("Rate: {:.1} Hz", total_events as f64 / 3.0);
}

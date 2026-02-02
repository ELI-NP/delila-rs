//! AMax Raw Data Dump
//!
//! Dumps raw data for analysis to understand the actual format.

use delila_rs::reader::CaenHandle;
use std::env;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    let url = if args.len() > 1 {
        &args[1]
    } else {
        "dig2://172.18.4.56"
    };

    println!("=== AMax Raw Data Dump ===");
    println!("URL: {}", url);

    let handle = match CaenHandle::open(url) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    // Minimal configuration
    handle.set_value("/ch/0/par/chenable", "True").ok();
    for ch in 1..32 {
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), "False");
    }

    // Set MCA HLS registers
    let core_regs: [(u32, u32); 13] = [
        (0x0, 0),      // POLARITY
        (0x1, 0),      // OFFSET
        (0x2, 100),    // THRS
        (0x3, 10),     // TRIG_K
        (0x4, 12),     // TRIG_M
        (0x5, 500),    // TRAP_K
        (0x6, 550),    // TRAP_M
        (0x7, 3499000),// DECONV_M
        (0x8, 2500),   // TRAP_GAIN
        (0x9, 6),      // BL_LEN
        (0xA, 1200),   // BL_INIB
        (0xB, 510),    // SAMPLE_POS
        (0xC, 1),      // RUN_CFG
    ];
    for (addr, value) in &core_regs {
        let _ = handle.set_user_register(addr * 4, *value);
    }

    // Configure endpoint
    let endpoint = match handle.configure_endpoint(true) {
        Ok(ep) => ep,
        Err(e) => {
            eprintln!("Endpoint error: {}", e);
            std::process::exit(1);
        }
    };

    // Start acquisition
    let _ = handle.send_command("/cmd/cleardata");
    let _ = handle.send_command("/cmd/armacquisition");
    let _ = handle.send_command("/cmd/swstartacquisition");

    println!("\nWaiting for data...\n");

    let mut buffer = Vec::with_capacity(4 * 1024 * 1024);
    let mut read_count = 0;

    for _ in 0..20 {
        match endpoint.read_data(100, &mut buffer) {
            Ok(Some(raw)) => {
                if raw.size > 32 {
                    read_count += 1;
                    println!("=== Read #{} ===", read_count);
                    println!("Size: {} bytes, N_EVENTS: {}", raw.size, raw.n_events);
                    println!();

                    // Dump raw bytes (first 256 bytes or full data if smaller)
                    let dump_size = raw.size.min(256);
                    println!("Raw bytes (first {} bytes):", dump_size);
                    for (i, chunk) in raw.data[..dump_size].chunks(16).enumerate() {
                        print!("{:04X}: ", i * 16);
                        for b in chunk {
                            print!("{:02X} ", b);
                        }
                        println!();
                    }
                    println!();

                    // Dump as 64-bit words (big-endian)
                    let num_words = raw.size / 8;
                    println!("As 64-bit BE words (first 32 words):");
                    for i in 0..num_words.min(32) {
                        let offset = i * 8;
                        let word = u64::from_be_bytes([
                            raw.data[offset], raw.data[offset+1],
                            raw.data[offset+2], raw.data[offset+3],
                            raw.data[offset+4], raw.data[offset+5],
                            raw.data[offset+6], raw.data[offset+7],
                        ]);

                        // Parse fields
                        let last = (word >> 63) & 0x1;
                        let bit62 = (word >> 62) & 0x1;
                        let ch = (word >> 56) & 0x7F;
                        let special = (word >> 55) & 0x1;
                        let info = (word >> 51) & 0xF;
                        let ts = word & 0x0000_FFFF_FFFF_FFFF;

                        println!(
                            "  W{:02}: 0x{:016X}  last={} b62={} ch={:2} se={} info={:X} ts/val={}",
                            i, word, last, bit62, ch, special, info, ts
                        );

                        // If this looks like Word 1 (data word), parse differently
                        if i > 0 && last == 0 {
                            let wf = (word >> 62) & 0x1;
                            let flags_b = (word >> 50) & 0xFFF;
                            let flags_a = (word >> 42) & 0xFF;
                            let psd = (word >> 26) & 0xFFFF;
                            let fine_ts = (word >> 16) & 0x3FF;
                            let energy = word & 0xFFFF;
                            println!(
                                "       [Data parse: wf={} flags_b={:03X} flags_a={:02X} psd={} fine_ts={} energy={}]",
                                wf, flags_b, flags_a, psd, fine_ts, energy
                            );
                        }
                    }
                    println!();

                    if read_count >= 3 {
                        break;
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                println!("Read error: {}", e);
                break;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Stop
    let _ = handle.send_command("/cmd/swstopacquisition");
    let _ = handle.send_command("/cmd/disarmacquisition");
    println!("Done.");
}

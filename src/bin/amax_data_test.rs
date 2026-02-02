//! AMax Data Acquisition Test
//!
//! Captures raw data from AMax firmware using internal test pulse.
//! Dumps the data in hexadecimal format for analysis.
//!
//! Usage: cargo run --bin amax_data_test [-- <url>]
//! Default URL: dig2://172.18.4.56

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

    println!("=== AMax Data Acquisition Test ===");
    println!("URL: {}", url);
    println!();

    // Connect to digitizer
    let handle = match CaenHandle::open(url) {
        Ok(h) => {
            println!("[OK] Connected to digitizer");
            h
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    // Read device info
    println!();
    println!("--- Device Info ---");
    read_device_info(&handle);

    // Configure for internal test pulse
    println!();
    println!("--- Configuring for Test Pulse ---");
    if let Err(e) = configure_test_pulse(&handle) {
        eprintln!("[ERROR] Configuration failed: {}", e);
        std::process::exit(1);
    }
    println!("[OK] Test pulse configured");

    // Configure endpoint for RAW data
    println!();
    println!("--- Configuring Endpoint ---");
    let endpoint = match handle.configure_endpoint(true) {
        Ok(ep) => {
            println!("[OK] Endpoint configured (RAW mode)");
            ep
        }
        Err(e) => {
            eprintln!("[ERROR] Endpoint configuration failed: {}", e);
            std::process::exit(1);
        }
    };

    // Clear data
    println!();
    println!("--- Starting Acquisition ---");
    if let Err(e) = handle.send_command("/cmd/cleardata") {
        eprintln!("[WARNING] ClearData failed: {}", e);
    }

    // Arm and start acquisition
    if let Err(e) = handle.send_command("/cmd/armacquisition") {
        eprintln!("[WARNING] ArmAcquisition failed: {}", e);
    }
    if let Err(e) = handle.send_command("/cmd/swstartacquisition") {
        eprintln!("[ERROR] SwStartAcquisition failed: {}", e);
        std::process::exit(1);
    }
    println!("[OK] Acquisition started");

    // Send SW triggers to capture real signal
    println!("Sending SW triggers (20 @ 10ms interval)...");
    for i in 0..20 {
        let _ = handle.send_command("/cmd/sendswtrigger");
        if i % 5 == 0 {
            print!(".");
        }
        thread::sleep(Duration::from_millis(10));
    }
    println!(" done");

    // Wait a bit for data
    println!("Waiting 200ms...");
    thread::sleep(Duration::from_millis(200));

    // Read data
    println!();
    println!("--- Reading Data ---");
    let mut buffer = Vec::with_capacity(1024 * 1024); // 1MB buffer

    let mut total_events = 0;
    let mut total_bytes = 0;
    let read_attempts = 5;

    let mut dumped_real_event = false;
    for attempt in 0..read_attempts {
        match endpoint.read_data(100, &mut buffer) {
            Ok(Some(raw)) => {
                println!(
                    "Read #{}: {} bytes, {} events",
                    attempt + 1,
                    raw.size,
                    raw.n_events
                );
                total_events += raw.n_events;
                total_bytes += raw.size;

                // Dump: first actual event data (not the start signal which is 32 bytes)
                if !dumped_real_event && raw.size > 32 {
                    dump_raw_data(&raw.data, raw.n_events);
                    dumped_real_event = true;
                } else if attempt == 0 && raw.size == 32 {
                    // Start signal
                    println!("  (Start signal detected - 32 bytes)");
                }
            }
            Ok(None) => {
                println!("Read #{}: Timeout (no data)", attempt + 1);
            }
            Err(e) => {
                println!("Read #{}: Error - {}", attempt + 1, e);
                break;
            }
        }
    }

    // Stop acquisition
    println!();
    println!("--- Stopping Acquisition ---");
    if let Err(e) = handle.send_command("/cmd/swstopacquisition") {
        eprintln!("[WARNING] SwStopAcquisition failed: {}", e);
    }
    if let Err(e) = handle.send_command("/cmd/disarmacquisition") {
        eprintln!("[WARNING] DisarmAcquisition failed: {}", e);
    }
    println!("[OK] Acquisition stopped");

    println!();
    println!("=== Summary ===");
    println!("Total bytes: {}", total_bytes);
    println!("Total events: {}", total_events);
    println!("=== Done ===");
}

fn read_device_info(handle: &CaenHandle) {
    let params = [
        "/par/ModelName",
        "/par/SerialNum",
        "/par/FwType",
        "/par/NumCh",
    ];

    for path in &params {
        match handle.get_value(path) {
            Ok(value) => {
                let name = path.split('/').next_back().unwrap_or(path);
                println!("  {:<20}: {}", name, value);
            }
            Err(_) => {
                // Try lowercase for compatibility
                let lower_path = path.to_lowercase();
                if let Ok(value) = handle.get_value(&lower_path) {
                    let name = path.split('/').next_back().unwrap_or(path);
                    println!("  {:<20}: {}", name, value);
                }
            }
        }
    }
}

fn configure_test_pulse(handle: &CaenHandle) -> Result<(), Box<dyn std::error::Error>> {
    // Enable channel 0 only (AMax only supports ch0)
    handle.set_value("/ch/0/par/chenable", "True")?;
    println!("  Enabled ch0");

    // Disable all other channels (VX2730 has 32 channels, but AMax only uses ch0)
    for ch in 1..32 {
        let path = format!("/ch/{}/par/chenable", ch);
        let _ = handle.set_value(&path, "False");
    }

    // Configure data source - ADC_DATA for real signal input
    match handle.set_value("/ch/0/par/WaveDataSource", "ADC_DATA") {
        Ok(()) => println!("  Set WaveDataSource = ADC_DATA"),
        Err(e) => println!("  WaveDataSource failed: {}", e),
    }

    // Use SwTrg (software trigger) to verify data path
    match handle.set_value("/par/AcqTriggerSource", "SwTrg") {
        Ok(()) => println!("  Set AcqTriggerSource = SwTrg"),
        Err(e) => println!("  AcqTriggerSource=SwTrg failed: {}", e),
    }

    // Disable data reduction to get full events
    match handle.set_value("/par/EnDataReduction", "False") {
        Ok(()) => println!("  Set EnDataReduction = False"),
        Err(e) => println!("  EnDataReduction failed: {}", e),
    }

    // Set StartSource to SWcmd
    match handle.set_value("/par/StartSource", "SWcmd") {
        Ok(()) => println!("  Set StartSource = SWcmd"),
        Err(e) => println!("  StartSource failed: {}", e),
    }

    // AMax specific: Set some basic parameters (byte_address = logical_address * 4)
    println!("  Setting AMax registers...");

    // Try NEGATIVE polarity (same as PHA1) with LOW threshold
    let amax_regs = [
        (0x0, 0, "POLARITY"),       // 0 = NEGATIVE (like PHA1 signal)
        (0x1, 0, "OFFSET"),         // 0
        (0x2, 20, "THRS"),          // VERY LOW threshold to ensure trigger
        (0x3, 10, "TRIG_K"),        // fast trigger rise
        (0x4, 12, "TRIG_M"),        // fast trigger decay
        (0x5, 500, "TRAP_K"),       // trapezoid rise
        (0x6, 550, "TRAP_M"),       // trapezoid decay
        (0x7, 3499000, "DECONV_M"), // deconvolution
        (0x8, 2500, "TRAP_GAIN"),   // digital gain
        (0x9, 6, "BL_LEN"),         // baseline length (2^6 = 64 samples)
        (0xA, 1200, "BL_INIB"),     // baseline inhibit
        (0xB, 510, "SAMPLE_POS"),   // sample position
        (0xC, 1, "RUN_CFG"),        // run config
    ];

    for (addr, value, name) in &amax_regs {
        let byte_addr = addr * 4;
        match handle.set_user_register(byte_addr, *value) {
            Ok(()) => println!("    {} = {} (0x{:X})", name, value, byte_addr),
            Err(e) => println!("    {} (0x{:X}): ERROR {}", name, byte_addr, e),
        }
    }
    println!("  AMax core registers set");

    // AMax high-address registers
    let amax_high_regs = [
        (0x14000, 200, "WINDOW_MAXIM"),
        (0x160000, 200, "baseline_delay"),
        (0x160001, 6, "baseline_len"),
        (0x160002, 1000, "baseline_offset"),
        (0x160003, 1000, "AMAX_window"),
        (0x160004, 4, "AMAX_delay"),
        (0x160005, 2, "AMAX_len"),
    ];

    for (addr, value, name) in &amax_high_regs {
        let byte_addr = addr * 4;
        match handle.set_user_register(byte_addr, *value) {
            Ok(()) => (),
            Err(e) => println!("    {} (0x{:X}): {}", name, byte_addr, e),
        }
    }
    println!("  AMax high-address registers set");

    Ok(())
}

fn dump_raw_data(data: &[u8], n_events: u32) {
    println!();
    println!("=== Raw Data Dump ===");
    println!("Total bytes: {}, Events: {}", data.len(), n_events);
    println!();

    // Dump as 64-bit words (big-endian, 8 bytes per word)
    let word_size = 8;
    let num_words = data.len() / word_size;

    println!("Parsing events...");
    println!();

    let mut word_idx = 0;
    let mut event_count = 0;

    while word_idx < num_words && event_count < 5 {
        // Limit to first 5 events
        let word0 = read_u64_be(&data[word_idx * word_size..(word_idx + 1) * word_size]);

        let bit63 = (word0 >> 63) & 0x1;
        let channel = ((word0 >> 56) & 0x7F) as u8;
        let special_event = (word0 >> 55) & 0x1;
        let info = ((word0 >> 51) & 0xF) as u8;
        let timestamp = word0 & 0x0000_FFFF_FFFF_FFFF;

        println!("--- Event {} (word {}) ---", event_count, word_idx);
        println!("  Word 0: 0x{:016X}", word0);
        println!(
            "    bit63={}, channel={}, special={}, info=0x{:X}",
            bit63, channel, special_event, info
        );
        println!(
            "    timestamp={} ({}ns, ~{:.3}s)",
            timestamp,
            timestamp * 8,
            (timestamp * 8) as f64 / 1e9
        );

        word_idx += 1;
        if word_idx >= num_words {
            break;
        }

        let word1 = read_u64_be(&data[word_idx * word_size..(word_idx + 1) * word_size]);
        let last_word = (word1 >> 63) & 0x1;
        let waveform_present = (word1 >> 62) & 0x1;
        let flags_b = ((word1 >> 50) & 0xFFF) as u16;
        let flags_a = ((word1 >> 42) & 0xFF) as u8;
        let psd = ((word1 >> 26) & 0xFFFF) as u16;
        let fine_time = ((word1 >> 16) & 0x3FF) as u16;
        let energy = (word1 & 0xFFFF) as u16;

        println!("  Word 1: 0x{:016X}", word1);
        println!("    last={}, waveform={}", last_word, waveform_present);
        println!("    flags_b=0x{:03X}, flags_a=0x{:02X}", flags_b, flags_a);
        println!(
            "    PSD={}, fine_time={}, energy={}",
            psd, fine_time, energy
        );

        // Calculate full timestamp
        let fine_time_ns = (fine_time as f64 / 1024.0) * 8.0;
        let full_timestamp_ns = (timestamp as f64 * 8.0) + fine_time_ns;
        println!(
            "    Full timestamp: {:.3}ns (~{:.6}s)",
            full_timestamp_ns,
            full_timestamp_ns / 1e9
        );

        word_idx += 1;

        // Check for USER WORDS (AMax value, baseline, etc.)
        if last_word == 0 && waveform_present == 0 {
            // There are user words
            let mut user_word_count = 0;
            while word_idx < num_words {
                let user_word =
                    read_u64_be(&data[word_idx * word_size..(word_idx + 1) * word_size]);
                let is_last = (user_word >> 63) & 0x1;
                let user_data = user_word & 0x7FFF_FFFF_FFFF_FFFF;

                println!(
                    "  User Word {}: 0x{:016X} (data=0x{:015X}, last={})",
                    user_word_count, user_word, user_data, is_last
                );

                word_idx += 1;
                user_word_count += 1;

                if is_last != 0 {
                    break;
                }
            }
            println!("    ({} user words)", user_word_count);
        }

        // Handle waveform if present
        if waveform_present != 0 && word_idx < num_words {
            let wave_header = read_u64_be(&data[word_idx * word_size..(word_idx + 1) * word_size]);
            let truncated = (wave_header >> 63) & 0x1;
            let wave_word_count = (wave_header & 0xFFF) as usize;
            println!(
                "  Wave Header: 0x{:016X} (truncated={}, {} words = {} samples)",
                wave_header,
                truncated,
                wave_word_count,
                wave_word_count * 4
            );
            word_idx += 1;

            // Skip waveform data
            word_idx += wave_word_count;
        }

        event_count += 1;
        println!();
    }

    if event_count < n_events as usize {
        println!("... ({} more events)", n_events as usize - event_count);
    }

    // Also dump raw hex for debugging
    println!();
    println!("Raw hex dump (first 64 words):");
    for i in 0..num_words.min(64) {
        let offset = i * word_size;
        let word = read_u64_be(&data[offset..offset + word_size]);
        if i % 4 == 0 {
            print!("\n{:4}: ", i);
        }
        print!("{:016X} ", word);
    }
    println!();
}

fn read_u64_be(data: &[u8]) -> u64 {
    u64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ])
}

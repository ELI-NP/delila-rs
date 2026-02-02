//! AMax Test Pulse Verification
//!
//! Uses internal test pulse with GlobalTrigger to verify data acquisition.

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

    println!("=== AMax Test Pulse Verification ===");
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
    for path in ["/par/ModelName", "/par/SerialNum", "/par/FwType"] {
        if let Ok(value) = handle.get_value(path) {
            let name = path.split('/').next_back().unwrap_or(path);
            println!("  {:<15}: {}", name, value);
        }
    }

    // Configure for test pulse
    println!();
    println!("--- Configuring for Test Pulse ---");

    // Enable channel 0 only
    handle.set_value("/ch/0/par/chenable", "True").ok();
    println!("  Enabled ch0");

    for ch in 1..32 {
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), "False");
    }

    // Set WaveDataSource to ADC_DATA
    handle.set_value("/ch/0/par/WaveDataSource", "ADC_DATA").ok();
    println!("  WaveDataSource = ADC_DATA");

    // Enable test pulse
    // Try different test pulse parameters
    println!();
    println!("--- Setting Test Pulse ---");

    // Check available test pulse parameters
    let test_params = [
        "/par/TestPulsePeriod",
        "/par/TestPulseWidth",
        "/par/TestPulseLowLevel",
        "/par/TestPulseHighLevel",
        "/par/TestPulseSource",
    ];

    for param in &test_params {
        match handle.get_value(param) {
            Ok(v) => println!("  {} = {}", param, v),
            Err(_) => (),
        }
    }

    // Set test pulse period (e.g., 1000000 ns = 1ms = 1kHz)
    match handle.set_value("/par/TestPulsePeriod", "1000000") {
        Ok(()) => println!("  TestPulsePeriod = 1000000 (1kHz)"),
        Err(e) => println!("  TestPulsePeriod error: {}", e),
    }

    // Set GlobalTriggerSource to include test pulse
    println!();
    println!("--- Setting GlobalTriggerSource ---");

    // Try various GlobalTriggerSource options
    let gts_options = [
        "TestPulse",
        "TstTrg",
        "SwTrg",
        "TestPulse | SwTrg",
    ];

    for opt in &gts_options {
        match handle.set_value("/par/GlobalTriggerSource", opt) {
            Ok(()) => println!("  GlobalTriggerSource = {} - ACCEPTED", opt),
            Err(_) => (),
        }
    }

    // Check current value
    if let Ok(v) = handle.get_value("/par/GlobalTriggerSource") {
        println!("  Current GlobalTriggerSource: {}", v);
    }

    // Set AcqTriggerSource
    handle.set_value("/par/AcqTriggerSource", "GlobalTriggerSource").ok();
    if let Ok(v) = handle.get_value("/par/AcqTriggerSource") {
        println!("  AcqTriggerSource: {}", v);
    }

    // Set MCA HLS registers
    println!();
    println!("--- Setting MCA HLS Registers ---");

    let core_regs: [(u32, u32, &str); 13] = [
        (0x0, 0, "POLARITY"),        // 0 = NEGATIVE
        (0x1, 0, "OFFSET"),
        (0x2, 100, "THRS"),          // Threshold
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
        let byte_addr = addr * 4;
        match handle.set_user_register(byte_addr, *value) {
            Ok(()) => println!("    {} = {}", name, value),
            Err(e) => println!("    {} = {} ERROR: {}", name, value, e),
        }
    }

    // Configure endpoint
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

    // Start acquisition
    println!();
    println!("--- Starting Acquisition ---");
    let _ = handle.send_command("/cmd/cleardata");
    let _ = handle.send_command("/cmd/armacquisition");
    if let Err(e) = handle.send_command("/cmd/swstartacquisition") {
        eprintln!("[ERROR] SwStartAcquisition failed: {}", e);
        std::process::exit(1);
    }
    println!("[OK] Acquisition started");

    // Read data
    let mut buffer = Vec::with_capacity(4 * 1024 * 1024);
    let mut total_events = 0;
    let mut total_bytes = 0;

    println!();
    println!("--- Reading Data (3 seconds) ---");

    for i in 0..30 {
        match endpoint.read_data(100, &mut buffer) {
            Ok(Some(raw)) => {
                if raw.size > 32 {
                    println!(
                        "[{:.1}s] {} bytes, {} events",
                        i as f32 * 0.1,
                        raw.size,
                        raw.n_events
                    );
                    total_events += raw.n_events;
                    total_bytes += raw.size;

                    // Decode and show first few events
                    if total_events <= 10 {
                        dump_events(&raw.data, raw.n_events.min(3) as usize);
                    }
                }
            }
            Ok(None) => {
                if i % 10 == 0 {
                    print!(".");
                    use std::io::Write;
                    std::io::stdout().flush().ok();
                }
            }
            Err(e) => {
                println!("\n[ERROR] Read error: {}", e);
                break;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    println!();

    // Stop
    println!();
    println!("--- Stopping Acquisition ---");
    let _ = handle.send_command("/cmd/swstopacquisition");
    let _ = handle.send_command("/cmd/disarmacquisition");
    println!("[OK] Stopped");

    println!();
    println!("=== Results ===");
    println!("Total bytes: {}", total_bytes);
    println!("Total events: {}", total_events);
    println!("Rate: {:.1} Hz", total_events as f64 / 3.0);
}

fn dump_events(data: &[u8], max_events: usize) {
    let offset = 0;
    let event_count = 0;

    while offset + 16 <= data.len() && event_count < max_events {
        let word0 = u64::from_be_bytes([
            data[offset], data[offset+1], data[offset+2], data[offset+3],
            data[offset+4], data[offset+5], data[offset+6], data[offset+7],
        ]);
        let word1 = u64::from_be_bytes([
            data[offset+8], data[offset+9], data[offset+10], data[offset+11],
            data[offset+12], data[offset+13], data[offset+14], data[offset+15],
        ]);

        let channel = ((word0 >> 56) & 0x7F) as u8;
        let timestamp = word0 & 0x0000_FFFF_FFFF_FFFF;
        let energy = (word1 & 0xFFFF) as u16;
        let fine_time = ((word1 >> 16) & 0x3FF) as u16;
        let has_waveform = ((word1 >> 62) & 0x1) != 0;

        println!(
            "    Event {}: ch={}, ts={}, energy={}, fine_time={}, waveform={}",
            event_count, channel, timestamp, energy, fine_time, has_waveform
        );

        // Skip to next event (estimate based on event size)
        // For now, just show first event per read
        break;
    }
}

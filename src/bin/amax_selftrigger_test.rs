//! AMax Self-Trigger Investigation
//!
//! Tests various trigger configurations to enable MCA HLS internal trigger.
//!
//! Usage: cargo run --bin amax_selftrigger_test [-- <url>]

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

    println!("=== AMax Self-Trigger Investigation ===");
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

    // Check available AcqTriggerSource options
    println!();
    println!("--- Checking AcqTriggerSource Options ---");
    check_trigger_options(&handle);

    // Configure for self-trigger test
    println!();
    println!("--- Configuring for Self-Trigger ---");
    if let Err(e) = configure_selftrigger(&handle) {
        eprintln!("[ERROR] Configuration failed: {}", e);
        std::process::exit(1);
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

    // Clear and start
    println!();
    println!("--- Starting Acquisition (Self-Trigger Mode) ---");
    let _ = handle.send_command("/cmd/cleardata");
    let _ = handle.send_command("/cmd/armacquisition");
    if let Err(e) = handle.send_command("/cmd/swstartacquisition") {
        eprintln!("[ERROR] SwStartAcquisition failed: {}", e);
        std::process::exit(1);
    }
    println!("[OK] Acquisition started - waiting for self-triggers...");

    // Wait for self-triggers (no SW trigger!)
    let mut buffer = Vec::with_capacity(4 * 1024 * 1024); // 4MB buffer
    let mut total_events = 0;
    let mut total_bytes = 0;

    println!();
    println!("--- Waiting for Data (5 seconds, no SW trigger) ---");

    for i in 0..50 {
        match endpoint.read_data(100, &mut buffer) {
            Ok(Some(raw)) => {
                if raw.size > 32 {
                    // Real event data (not just start signal)
                    println!(
                        "[{:.1}s] Got {} bytes, {} events",
                        i as f32 * 0.1,
                        raw.size,
                        raw.n_events
                    );
                    total_events += raw.n_events;
                    total_bytes += raw.size;

                    // Dump first event
                    if total_events <= 5 {
                        dump_first_event(&raw.data);
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

    if total_events > 0 {
        println!("[SUCCESS] Self-trigger is working!");
    } else {
        println!("[FAIL] No self-triggered events received");
        println!();
        println!("Possible causes:");
        println!("  1. THRS too high - try lowering threshold");
        println!("  2. trigger_and registers not properly configured");
        println!("  3. Internal trigger not connected to acquisition path");
        println!("  4. RUN_CFG needs different value");
    }
}

fn read_device_info(handle: &CaenHandle) {
    let params = ["/par/ModelName", "/par/SerialNum", "/par/FwType"];
    for path in &params {
        if let Ok(value) = handle.get_value(path) {
            let name = path.split('/').next_back().unwrap_or(path);
            println!("  {:<15}: {}", name, value);
        }
    }
}

fn check_trigger_options(handle: &CaenHandle) {
    // Try to read current value
    match handle.get_value("/par/AcqTriggerSource") {
        Ok(v) => println!("  Current AcqTriggerSource: {}", v),
        Err(e) => println!("  Cannot read AcqTriggerSource: {}", e),
    }

    // Try various trigger source options
    let options = [
        "SwTrg",
        "ChSelfTrigger",
        "Ch0SelfTrigger",
        "GlobalTriggerSource",
        "TRGIN",
        "ITLA",
        "ITLB",
        "Internal",
        "SelfTrigger",
        "Auto",
    ];

    println!("  Testing trigger source options:");
    for opt in &options {
        match handle.set_value("/par/AcqTriggerSource", opt) {
            Ok(()) => println!("    {} - ACCEPTED", opt),
            Err(_) => println!("    {} - rejected", opt),
        }
    }
}

fn configure_selftrigger(handle: &CaenHandle) -> Result<(), Box<dyn std::error::Error>> {
    // Enable channel 0
    handle.set_value("/ch/0/par/chenable", "True")?;
    println!("  Enabled ch0");

    // Disable other channels
    for ch in 1..32 {
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), "False");
    }

    // Set data source
    let _ = handle.set_value("/ch/0/par/WaveDataSource", "ADC_DATA");
    println!("  Set WaveDataSource = ADC_DATA");

    // Disable ITL (not needed with firmware OR gate modification)
    let _ = handle.set_value("/ch/0/par/ITLConnect", "Disabled");

    // AcqTriggerSource doesn't matter - internal trigger goes directly to CAEN LIST via OR gate
    // But set to SwTrg to avoid external trigger requirements
    let _ = handle.set_value("/par/AcqTriggerSource", "SwTrg");
    println!("  AcqTriggerSource = SwTrg (internal trigger via firmware OR gate)");

    // Disable data reduction
    let _ = handle.set_value("/par/EnDataReduction", "False");
    let _ = handle.set_value("/par/StartSource", "SWcmd");

    // Disable test pulse
    match handle.set_value("/par/TestPulsePeriod", "0") {
        Ok(()) => println!("    TestPulsePeriod = 0 (disabled)"),
        Err(_) => (),
    }
    match handle.set_value("/ch/0/par/WaveDataSource", "ADC_DATA") {
        Ok(()) => println!("    WaveDataSource = ADC_DATA"),
        Err(_) => (),
    }

    // Set MCA HLS core registers with LOW threshold for easy triggering
    println!("  Setting MCA HLS registers (low threshold)...");

    let core_regs: [(u32, u32, &str); 13] = [
        (0x0, 0, "POLARITY"),        // 0 = NEGATIVE
        (0x1, 0, "OFFSET"),
        (0x2, 50, "THRS"),           // Very low threshold
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

    // Set high-address registers
    let high_regs: [(u32, u32, &str); 7] = [
        (0x14000, 200, "WINDOW_MAXIM"),
        (0x160000, 200, "baseline_delay"),
        (0x160001, 6, "baseline_len"),
        (0x160002, 1000, "baseline_offset"),
        (0x160003, 1000, "AMAX_window"),
        (0x160004, 4, "AMAX_delay"),
        (0x160005, 2, "AMAX_len"),
    ];

    for (addr, value, _name) in &high_regs {
        let byte_addr = addr * 4;
        let _ = handle.set_user_register(byte_addr, *value);
    }
    println!("    High-address registers set");

    // KEY: Set trigger_and registers (from amax_test.json)
    println!("  Setting trigger_and registers...");

    // amax_trigger_and at 0xC00A
    let trigger_and_addr = 0xC00A * 4;
    match handle.set_user_register(trigger_and_addr, 1) {
        Ok(()) => println!("    amax_trigger_and (0x{:X}) = 1", trigger_and_addr),
        Err(e) => println!("    amax_trigger_and ERROR: {}", e),
    }

    // en_trigger_and at 0xC00B with value 4409 (0x1139)
    let en_trigger_and_addr = 0xC00B * 4;
    match handle.set_user_register(en_trigger_and_addr, 4409) {
        Ok(()) => println!("    en_trigger_and (0x{:X}) = 4409 (0x1139)", en_trigger_and_addr),
        Err(e) => println!("    en_trigger_and ERROR: {}", e),
    }

    // Try different RUN_CFG values
    println!("  Testing RUN_CFG values...");
    for cfg in [1, 3, 5, 7] {
        let _ = handle.set_user_register(0xC * 4, cfg);
    }
    // Set back to 1
    let _ = handle.set_user_register(0xC * 4, 1);
    println!("    RUN_CFG = 1");

    Ok(())
}

fn dump_first_event(data: &[u8]) {
    if data.len() < 16 {
        return;
    }

    let word0 = u64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    let word1 = u64::from_be_bytes([
        data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
    ]);

    let channel = ((word0 >> 56) & 0x7F) as u8;
    let timestamp = word0 & 0x0000_FFFF_FFFF_FFFF;
    let energy = (word1 & 0xFFFF) as u16;
    let fine_time = ((word1 >> 16) & 0x3FF) as u16;

    println!(
        "    Event: ch={}, ts={}, energy={}, fine_time={}",
        channel, timestamp, energy, fine_time
    );
}

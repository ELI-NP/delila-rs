//! AMax FW incremental test
//!
//! Step-by-step: Open → ChEnable → TestPulse → Endpoint → Arm → Start → Read 10s → Stop → Close
//! Usage: cargo run --release --bin amax_fw_test [url]

use delila_rs::reader::CaenHandle;
use std::env;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = env::args().collect();
    let url = if args.len() > 1 {
        &args[1]
    } else {
        "dig2://172.18.4.56"
    };

    // --- Open ---
    println!("[1] Open: {}", url);
    let handle = match CaenHandle::open(url) {
        Ok(h) => {
            println!("  OK");
            h
        }
        Err(e) => {
            eprintln!("  FAILED: {}", e);
            std::process::exit(1);
        }
    };

    for path in [
        "/par/ModelName",
        "/par/SerialNum",
        "/par/FwType",
        "/par/LicenseStatus",
        "/par/LicenseRemainingTime",
    ] {
        if let Ok(v) = handle.get_value(path) {
            println!("  {} = {}", path, v);
        }
    }

    // --- Enable channels ---
    println!("[2] Enable all channels");
    for ch in 0..32 {
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), "True");
    }
    for ch in 0..2 {
        if let Ok(v) = handle.get_value(&format!("/ch/{}/par/chenable", ch)) {
            println!("  ch{}/chenable = {}", ch, v);
        }
    }

    // --- Configure test pulse ---
    println!("[3] Configure TestPulse (1000 Hz)");
    let tp_params = [
        ("/par/TestPulsePeriod", "1000000"),
        ("/par/TestPulseWidth", "10"),
        ("/par/TestPulseLowLevel", "1000"),
        ("/par/TestPulseHighLevel", "3000"),
    ];
    for (path, value) in &tp_params {
        match handle.set_value(path, value) {
            Ok(()) => println!("  {} = {} OK", path, value),
            Err(e) => println!("  {} = {} FAILED: {}", path, value, e),
        }
    }
    match handle.set_value("/par/AcqTriggerSource", "TestPulse") {
        Ok(()) => println!("  AcqTriggerSource = TestPulse OK"),
        Err(e) => println!("  AcqTriggerSource FAILED: {}", e),
    }

    // --- Configure RAW endpoint (like amax_testpulse_test.rs) ---
    println!("[4] Configure RAW endpoint");
    let endpoint = match handle.configure_endpoint(true) {
        Ok(ep) => {
            println!("  OK");
            ep
        }
        Err(e) => {
            eprintln!("  FAILED: {}", e);
            std::process::exit(1);
        }
    };

    // --- Arm ---
    println!("[5] Arm");
    if let Err(e) = handle.send_command("/cmd/cleardata") {
        eprintln!("  cleardata failed: {}", e);
    }
    match handle.send_command("/cmd/armacquisition") {
        Ok(()) => println!("  OK"),
        Err(e) => {
            eprintln!("  FAILED: {}", e);
            std::process::exit(1);
        }
    }

    // --- Start ---
    println!("[6] Start");
    match handle.send_command("/cmd/swstartacquisition") {
        Ok(()) => println!("  OK"),
        Err(e) => {
            eprintln!("  FAILED: {}", e);
            std::process::exit(1);
        }
    }

    // --- Continuous read loop for 10s ---
    println!("[7] Continuous read loop (10s)...");
    let start = Instant::now();
    let mut total_events: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut total_reads: u64 = 0;
    let mut last_report = Instant::now();
    let mut events_since_report: u64 = 0;
    let mut buffer = Vec::with_capacity(4 * 1024 * 1024);
    let mut no_data_since: Option<Instant> = None;

    while start.elapsed() < Duration::from_secs(10) {
        match endpoint.read_data(10, &mut buffer) {
            Ok(Some(raw)) => {
                total_reads += 1;
                total_events += raw.n_events as u64;
                total_bytes += raw.size as u64;
                events_since_report += raw.n_events as u64;
                no_data_since = None;

                if total_reads <= 3 {
                    println!(
                        "  read {}: {} bytes, {} events",
                        total_reads, raw.size, raw.n_events
                    );
                }
            }
            Ok(None) => {
                if no_data_since.is_none() && total_events > 0 {
                    no_data_since = Some(Instant::now());
                }
                if let Some(t) = no_data_since {
                    if t.elapsed() >= Duration::from_secs(3) {
                        println!(
                            "  *** NO DATA for 3s after {} events (at {:.1}s) ***",
                            total_events,
                            start.elapsed().as_secs_f64()
                        );
                        if let Ok(v) = handle.get_value("/par/AcquisitionStatus") {
                            println!("  AcquisitionStatus = {}", v);
                        }
                        no_data_since = None;
                    }
                }
            }
            Err(e) => {
                println!(
                    "  READ ERROR: code={} {}: {}",
                    e.code, e.name, e.description
                );
                if e.code == -12 {
                    println!("  STOP signal — breaking");
                    break;
                }
            }
        }

        if last_report.elapsed() >= Duration::from_secs(1) {
            let rate = events_since_report as f64 / last_report.elapsed().as_secs_f64();
            println!(
                "  {:2.0}s: {} events, {} bytes, {:.0} events/s",
                start.elapsed().as_secs_f64(),
                total_events,
                total_bytes,
                rate
            );
            events_since_report = 0;
            last_report = Instant::now();
        }
    }

    println!(
        "  Total: {} events, {} bytes in {:.1}s",
        total_events,
        total_bytes,
        start.elapsed().as_secs_f64()
    );

    // --- Stop ---
    println!("[8] Stop");
    if let Err(e) = handle.send_command("/cmd/swstopacquisition") {
        eprintln!("  swstopacquisition failed: {}", e);
    }
    if let Err(e) = handle.send_command("/cmd/disarmacquisition") {
        eprintln!("  disarmacquisition failed: {}", e);
    }
    println!("  OK");

    // --- Close ---
    println!("[9] Close");
    drop(handle);
    println!("  OK");

    println!();
    println!("All steps completed.");
}

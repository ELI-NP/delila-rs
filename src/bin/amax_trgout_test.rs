//! AMax TRGOUT Configuration Test
//!
//! Tests if we can configure TRGOUT to output the internal trigger,
//! which could then be looped back to TRGIN for self-triggering.

use delila_rs::reader::CaenHandle;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let url = if args.len() > 1 {
        &args[1]
    } else {
        "dig2://172.18.4.56"
    };

    println!("=== AMax TRGOUT Configuration Test ===");
    println!("URL: {}", url);
    println!();

    let handle = match CaenHandle::open(url) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    // Check available GPIO/TRGOUT parameters
    println!("--- Checking GPIO/TRGOUT Parameters ---");
    println!();

    // Standard CAEN parameters for trigger output
    let trgout_params = [
        "/par/TrgOutMode",
        "/par/TrgOutMux",
        "/par/GPIOMode",
        "/par/FPIOtype",
        "/par/FPTrgOutMode",
        "/par/SyncOutMode",
        "/par/BusyInSource",
        "/par/VetoSource",
        "/par/RunSyncMode",
        "/par/BoardVetoSource",
        "/par/VolatileClockOutDelay",
    ];

    println!("Reading current values:");
    for param in &trgout_params {
        if let Ok(value) = handle.get_value(param) {
            println!("  {} = {}", param, value);
        }
    }

    println!();
    println!("--- Testing TrgOutMode Options ---");

    // Try various TrgOutMode values
    let trgout_options = [
        "Disabled",
        "TrgIn",
        "SwTrg",
        "LVDS",
        "ITLA",
        "ITLB",
        "ChSelfTrigger",
        "AcqTrigger",
        "Run",
        "RefClk",
        "TestPulse",
        "Busy",
        "Fixed0",
        "Fixed1",
        "SyncIn",
        "SIN",
        "GPIO",
        "AcceptTrg",
        "TrgClk",
        "UserTrigger", // Maybe this outputs the DPP trigger?
        "InternalTrigger",
        "SelfTrigger",
    ];

    for opt in &trgout_options {
        if handle.set_value("/par/TrgOutMode", opt).is_ok() {
            println!("  TrgOutMode = {} - ACCEPTED", opt);
        }
    }

    println!();
    println!("--- Testing GPIOMode Options ---");

    let gpio_options = [
        "Disabled", "TrgIn", "TrgOut", "Run", "RefClk", "SIN", "LVDS", "Busy", "UserGPO",
    ];

    for opt in &gpio_options {
        if handle.set_value("/par/GPIOMode", opt).is_ok() {
            println!("  GPIOMode = {} - ACCEPTED", opt);
        }
    }

    // Check if there are user registers for trigger output
    println!();
    println!("--- Checking for Trigger Output Registers ---");

    // The discrim_out signal in the VHDL might be accessible
    // Check some potential addresses
    let debug_addrs: [(u32, &str); 6] = [
        (0x160006, "debug_maxim_value"),
        (0x160007, "debug_amax_out"),
        (0x160008, "debug_baseline"),
        (0xC00A, "amax_trigger_and (read)"),
        (0xC00B, "en_trigger_and (read)"),
        (0x180003, "time_start"),
    ];

    for (addr, name) in &debug_addrs {
        let byte_addr = addr * 4;
        if let Ok(value) = handle.get_user_register(byte_addr) {
            println!("  {} (0x{:X}): {}", name, byte_addr, value);
        }
    }

    println!();
    println!("--- Recommendation ---");
    println!();
    println!("If TrgOutMode accepts any option that outputs the internal trigger,");
    println!("you can physically connect TRGOUT to TRGIN with a LEMO cable,");
    println!("and set AcqTriggerSource = TRGIN for self-triggering.");
    println!();
    println!("Alternative: Connect another digitizer's TRGOUT to this unit's TRGIN.");
}

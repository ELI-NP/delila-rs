//! AMax Firmware Check
//!
//! Reads registers to determine which firmware version is loaded.

use delila_rs::reader::CaenHandle;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let url = if args.len() > 1 {
        &args[1]
    } else {
        "dig2://172.18.4.56"
    };

    println!("=== AMax Firmware Check ===");
    println!("URL: {}", url);
    println!();

    let handle = match CaenHandle::open(url) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    // Read FwType
    if let Ok(fw_type) = handle.get_value("/par/FwType") {
        println!("FwType: {}", fw_type);
    }

    println!();
    println!("=== Register Read Test ===");
    println!();

    // Test registers from BOTH firmware types
    println!("--- TrapezoidalfilterMCA Registers (if this firmware) ---");
    let trap_regs: [(u32, &str); 8] = [
        (0x0, "POLARITY"),
        (0x2, "THRS"),
        (0x5, "TRAP_K"),
        (0x6, "TRAP_M"),
        (0x7, "DECONV_M"),
        (0x8, "TRAP_GAIN"),
        (0xC, "RUN_CFG"),
        (0x14000, "WINDOW_MAXIM"),
    ];

    for (addr, name) in &trap_regs {
        let byte_addr = addr * 4;
        match handle.get_user_register(byte_addr) {
            Ok(value) => println!("  {:20} (0x{:06X}): {}", name, byte_addr, value),
            Err(e) => println!("  {:20} (0x{:06X}): ERROR {}", name, byte_addr, e),
        }
    }

    println!();
    println!("--- DELILA2 CFD-based AMax Registers (if this firmware) ---");
    let cfd_regs: [(u32, &str); 8] = [
        (0x0, "hooold"),
        (0x7, "GAIN"),
        (0x8000F, "cfd_delay_in"),
        (0x80010, "cfd_fract_in"),
        (0xE000C, "en_sub1_len"),
        (0xE0020, "reg_en"),
        (0xE0021, "reg_amax"),
        (0xE001C, "amax_window_maxim"),
    ];

    for (addr, name) in &cfd_regs {
        let byte_addr = addr * 4;
        match handle.get_user_register(byte_addr) {
            Ok(value) => println!("  {:20} (0x{:06X}): {}", name, byte_addr, value),
            Err(e) => println!("  {:20} (0x{:06X}): ERROR {}", name, byte_addr, e),
        }
    }

    println!();
    println!("=== Firmware Identification ===");
    println!();

    // Try to identify by reading specific addresses that differ
    let trap_test = handle.get_user_register(0x14000 * 4); // WINDOW_MAXIM in Trap
    let cfd_test = handle.get_user_register(0xE001C * 4); // amax_window_maxim in CFD

    println!("Analysis:");
    match (trap_test, cfd_test) {
        (Ok(trap_val), Ok(cfd_val)) => {
            println!("  Both addresses readable");
            println!("  TrapezoidalMCA WINDOW_MAXIM (0x50000): {}", trap_val);
            println!("  DELILA2 CFD amax_window_maxim (0x380070): {}", cfd_val);
            if trap_val != 0 && trap_val != 0xFFFFFFFF {
                println!("  -> Likely TrapezoidalfilterMCA firmware");
            } else if cfd_val != 0 && cfd_val != 0xFFFFFFFF {
                println!("  -> Likely DELILA2 CFD-based firmware");
            } else {
                println!("  -> Cannot determine firmware type from these values");
            }
        }
        (Ok(_), Err(_)) => {
            println!("  -> Likely TrapezoidalfilterMCA firmware");
        }
        (Err(_), Ok(_)) => {
            println!("  -> Likely DELILA2 CFD-based firmware");
        }
        (Err(_), Err(_)) => {
            println!("  -> Neither firmware register accessible - unknown firmware");
        }
    }

    // Read some debug registers if available
    println!();
    println!("--- Debug/Status Registers ---");
    let debug_regs: [(u32, &str); 4] = [
        (0x160006, "debug_maxim_value (Trap)"),
        (0x160007, "debug_amax_out (Trap)"),
        (0xE0012, "norm_maxim (CFD)"),
        (0xE0017, "AMAX (CFD)"),
    ];

    for (addr, name) in &debug_regs {
        let byte_addr = addr * 4;
        if let Ok(value) = handle.get_user_register(byte_addr) {
            println!("  {:30} (0x{:06X}): {}", name, byte_addr, value);
        }
    }
}

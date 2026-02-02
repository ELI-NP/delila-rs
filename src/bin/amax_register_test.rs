//! AMax Register Read/Write Test
//!
//! Tests all AMax firmware registers by reading and writing values.
//! IMPORTANT: Address must be multiplied by 4 for FELib byte addressing.
//!
//! Usage: cargo run --bin amax_register_test [-- <url>]
//! Default URL: dig2://172.18.4.56

use delila_rs::reader::CaenHandle;
use std::env;

/// AMax register definition
struct Register {
    name: &'static str,
    address: u32,    // Logical address (from RegisterFile.json)
    test_value: u32, // Value to write for testing
    read_only: bool, // Some registers are read-only
}

/// All AMax registers from test_param27012026_caenlist.txt and RegisterFile.json
const AMAX_REGISTERS: &[Register] = &[
    // Core Control Registers (0x0000 - 0x000C)
    Register {
        name: "POLARITY",
        address: 0x0,
        test_value: 1,
        read_only: false,
    },
    Register {
        name: "OFFSET",
        address: 0x1,
        test_value: 0,
        read_only: false,
    },
    Register {
        name: "THRS",
        address: 0x2,
        test_value: 100,
        read_only: false,
    },
    Register {
        name: "TRIG_K",
        address: 0x3,
        test_value: 10,
        read_only: false,
    },
    Register {
        name: "TRIG_M",
        address: 0x4,
        test_value: 12,
        read_only: false,
    },
    Register {
        name: "TRAP_K",
        address: 0x5,
        test_value: 500,
        read_only: false,
    },
    Register {
        name: "TRAP_M",
        address: 0x6,
        test_value: 550,
        read_only: false,
    },
    Register {
        name: "DECONV_M",
        address: 0x7,
        test_value: 3499000,
        read_only: false,
    },
    Register {
        name: "TRAP_GAIN",
        address: 0x8,
        test_value: 2500,
        read_only: false,
    },
    Register {
        name: "BL_LEN",
        address: 0x9,
        test_value: 6,
        read_only: false,
    },
    Register {
        name: "BL_INIB",
        address: 0xA,
        test_value: 1200,
        read_only: false,
    },
    Register {
        name: "SAMPLE_POS",
        address: 0xB,
        test_value: 510,
        read_only: false,
    },
    Register {
        name: "RUN_CFG",
        address: 0xC,
        test_value: 1,
        read_only: false,
    },
    // Trigger AND registers
    Register {
        name: "amax_trigger_and",
        address: 0xC00A,
        test_value: 1,
        read_only: false,
    },
    Register {
        name: "en_trigger_and",
        address: 0xC00B,
        test_value: 4409,
        read_only: false,
    },
    // AMax specific registers (high addresses)
    Register {
        name: "WINDOW_MAXIM",
        address: 0x14000,
        test_value: 200,
        read_only: false,
    },
    Register {
        name: "baseline_delay",
        address: 0x160000,
        test_value: 200,
        read_only: false,
    },
    Register {
        name: "baseline_len",
        address: 0x160001,
        test_value: 6,
        read_only: false,
    },
    Register {
        name: "baseline_offset",
        address: 0x160002,
        test_value: 1000,
        read_only: false,
    },
    Register {
        name: "AMAX_window",
        address: 0x160003,
        test_value: 1000,
        read_only: false,
    },
    Register {
        name: "AMAX_delay",
        address: 0x160004,
        test_value: 4,
        read_only: false,
    },
    Register {
        name: "AMAX_len",
        address: 0x160005,
        test_value: 2,
        read_only: false,
    },
];

fn main() {
    let args: Vec<String> = env::args().collect();
    let url = if args.len() > 1 {
        &args[1]
    } else {
        "dig2://172.18.4.56"
    };

    println!("=== AMax Register Test ===");
    println!("URL: {}", url);
    println!("NOTE: Logical address × 4 = Byte address for FELib");
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

    // Test all registers
    println!();
    println!("--- Register Read Test ---");
    println!(
        "{:<20} {:>10} {:>12} {:>12}",
        "Register", "LogAddr", "ByteAddr", "Value"
    );
    println!("{}", "-".repeat(60));

    let mut read_success = 0;
    let mut read_fail = 0;

    for reg in AMAX_REGISTERS {
        let byte_addr = reg.address * 4;
        match handle.get_user_register(byte_addr) {
            Ok(value) => {
                println!(
                    "{:<20} 0x{:08X} 0x{:08X} {}",
                    reg.name, reg.address, byte_addr, value
                );
                read_success += 1;
            }
            Err(e) => {
                println!(
                    "{:<20} 0x{:08X} 0x{:08X} [ERROR: {}]",
                    reg.name, reg.address, byte_addr, e
                );
                read_fail += 1;
            }
        }
    }

    println!();
    println!(
        "Read results: {} success, {} failed",
        read_success, read_fail
    );

    // Write test (only if all reads succeeded)
    if read_fail == 0 {
        println!();
        println!("--- Register Write/Verify Test ---");
        println!(
            "{:<20} {:>10} {:>12} {:>10} {:>10}",
            "Register", "LogAddr", "WriteVal", "ReadBack", "Status"
        );
        println!("{}", "-".repeat(70));

        let mut write_success = 0;
        let mut write_fail = 0;

        for reg in AMAX_REGISTERS {
            if reg.read_only {
                println!(
                    "{:<20} 0x{:08X} {:>12} {:>10} SKIP (RO)",
                    reg.name, reg.address, "-", "-"
                );
                continue;
            }

            let byte_addr = reg.address * 4;

            // Write test value
            if let Err(e) = handle.set_user_register(byte_addr, reg.test_value) {
                println!(
                    "{:<20} 0x{:08X} {:>12} {:>10} WRITE ERR: {}",
                    reg.name, reg.address, reg.test_value, "-", e
                );
                write_fail += 1;
                continue;
            }

            // Read back and verify
            match handle.get_user_register(byte_addr) {
                Ok(readback) => {
                    let status = if readback == reg.test_value {
                        "OK"
                    } else {
                        "MISMATCH"
                    };
                    println!(
                        "{:<20} 0x{:08X} {:>12} {:>10} {}",
                        reg.name, reg.address, reg.test_value, readback, status
                    );
                    if readback == reg.test_value {
                        write_success += 1;
                    } else {
                        write_fail += 1;
                    }
                }
                Err(e) => {
                    println!(
                        "{:<20} 0x{:08X} {:>12} {:>10} READ ERR: {}",
                        reg.name, reg.address, reg.test_value, "-", e
                    );
                    write_fail += 1;
                }
            }
        }

        println!();
        println!(
            "Write/Verify results: {} success, {} failed",
            write_success, write_fail
        );
    }

    println!();
    println!("=== Done ===");
}

fn read_device_info(handle: &CaenHandle) {
    // DIG2 (VX2730) parameter paths
    let params = [
        "/par/ModelName",
        "/par/SerialNum",
        "/par/FwType",
        "/par/LicenseStatus",
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
                match handle.get_value(&lower_path) {
                    Ok(value) => {
                        let name = path.split('/').next_back().unwrap_or(path);
                        println!("  {:<20}: {}", name, value);
                    }
                    Err(e) => {
                        let name = path.split('/').next_back().unwrap_or(path);
                        println!("  {:<20}: [ERROR] {}", name, e);
                    }
                }
            }
        }
    }
}

//! Generate register_defs.json from Sci-Compiler's RegisterFile.json
//!
//! Usage:
//!   cargo run --bin gen_defs -- <RegisterFile.json> [output: register_defs.json]
//!
//! The output file has min=0, max=4294967295, default=0 for all registers.
//! Edit the output manually to set appropriate ranges for your firmware.

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;

/// Entry in Sci-Compiler RegisterFile.json top-level "Registers" array
#[derive(Deserialize)]
struct RawRegister {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Address")]
    address: u32,
}

#[derive(Deserialize)]
struct RegisterFile {
    #[serde(rename = "Registers")]
    registers: Vec<RawRegister>,
}

/// Output format for amax_viewer's register_defs.json
#[derive(Serialize)]
struct RegisterDef {
    section: String,
    name: String,
    address: u32,
    min: u32,
    max: u32,
    default: u32,
}

fn infer_section(name: &str, address: u32) -> &'static str {
    let name_upper = name.to_uppercase();
    if address >= 1_441_792
        || name_upper.starts_with("AMAX")
        || name.starts_with("baseline")
        || name == "WINDOW_MAXIM"
    {
        "AMax"
    } else {
        "Core"
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: gen_defs <RegisterFile.json> [register_defs.json]");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = args.get(2).map(|s| s.as_str()).unwrap_or("register_defs.json");

    let content = match fs::read_to_string(input_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read {}: {}", input_path, e);
            std::process::exit(1);
        }
    };

    let register_file: RegisterFile = match serde_json::from_str(&content) {
        Ok(rf) => rf,
        Err(e) => {
            eprintln!("Failed to parse {}: {}", input_path, e);
            std::process::exit(1);
        }
    };

    let defs: Vec<RegisterDef> = register_file
        .registers
        .iter()
        .map(|r| RegisterDef {
            section: infer_section(&r.name, r.address).to_string(),
            name: r.name.clone(),
            address: r.address,
            min: 0,
            max: u32::MAX,
            default: 0,
        })
        .collect();

    let json = match serde_json::to_string_pretty(&defs) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Serialization failed: {}", e);
            std::process::exit(1);
        }
    };

    match fs::write(output_path, &json) {
        Ok(()) => {
            println!(
                "Wrote {} register definitions to {}",
                defs.len(),
                output_path
            );
            println!("Edit min/max/default values as needed before using.");
        }
        Err(e) => {
            eprintln!("Failed to write {}: {}", output_path, e);
            std::process::exit(1);
        }
    }
}

//! Generate register_defs.json from Sci-Compiler's RegisterFile.json
//!
//! Usage:
//!   cargo run --bin gen_defs -- <RegisterFile.json> [-p fw_params.json] [-o register_defs.json]
//!
//! Without -p: output has min=0, max=4294967295, default=0 for all registers.
//! With -p: applies bit widths, defaults, and readonly flags from the parameter table.
//!
//! See fw_params.json for the parameter table format.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// Firmware parameter definition (from fw_params.json)
#[derive(Deserialize)]
struct FwParam {
    bits: u32,
    default: u32,
}

/// Firmware parameter table (fw_params.json)
#[derive(Deserialize)]
struct FwParams {
    params: HashMap<String, FwParam>,
    #[serde(default)]
    readonly_patterns: Vec<String>,
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
    #[serde(skip_serializing_if = "is_false")]
    readonly: bool,
}

fn is_false(v: &bool) -> bool {
    !v
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

fn print_usage() {
    eprintln!("Usage: gen_defs <RegisterFile.json> [-p fw_params.json] [-o register_defs.json]");
    eprintln!();
    eprintln!("Arguments:");
    eprintln!("  <RegisterFile.json>    Sci-Compiler register file (required)");
    eprintln!("  -p <fw_params.json>    Firmware parameter table with bit widths and defaults");
    eprintln!("  -o <output.json>       Output file (default: register_defs.json)");
    eprintln!();
    eprintln!("Without -p, all registers get min=0, max=4294967295, default=0.");
    eprintln!("With -p, registers are matched by substring and assigned proper ranges.");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    // Parse arguments
    let mut input_path: Option<&str> = None;
    let mut params_path: Option<&str> = None;
    let mut output_path = "register_defs.json";
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: -p requires a file path");
                    std::process::exit(1);
                }
                params_path = Some(&args[i]);
            }
            "-o" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: -o requires a file path");
                    std::process::exit(1);
                }
                output_path = &args[i];
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            _ => {
                if input_path.is_none() {
                    input_path = Some(&args[i]);
                } else {
                    eprintln!("Error: unexpected argument '{}'", args[i]);
                    print_usage();
                    std::process::exit(1);
                }
            }
        }
        i += 1;
    }

    let input_path = match input_path {
        Some(p) => p,
        None => {
            eprintln!("Error: RegisterFile.json is required");
            print_usage();
            std::process::exit(1);
        }
    };

    // Load RegisterFile.json
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

    // Load fw_params.json if provided
    let fw_params: Option<FwParams> = params_path.map(|path| {
        let params_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to read {}: {}", path, e);
                std::process::exit(1);
            }
        };
        match serde_json::from_str(&params_content) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to parse {}: {}", path, e);
                std::process::exit(1);
            }
        }
    });

    let mut matched_count = 0;
    let mut readonly_count = 0;
    let mut unmatched: Vec<String> = Vec::new();

    let defs: Vec<RegisterDef> = register_file
        .registers
        .iter()
        .map(|r| {
            let mut max = u32::MAX;
            let mut default = 0u32;
            let mut readonly = false;

            if let Some(ref fw) = fw_params {
                // Check readonly patterns first
                for pattern in &fw.readonly_patterns {
                    if r.name.contains(pattern.as_str()) {
                        readonly = true;
                        break;
                    }
                }

                // Find matching parameter by substring (longest match wins for specificity)
                let mut best_match: Option<(&str, &FwParam)> = None;
                for (key, param) in &fw.params {
                    if r.name.contains(key.as_str())
                        && best_match
                            .map(|(k, _)| key.len() > k.len())
                            .unwrap_or(true)
                    {
                        best_match = Some((key, param));
                    }
                }

                if let Some((_, param)) = best_match {
                    max = (1u64 << param.bits).saturating_sub(1).min(u32::MAX as u64) as u32;
                    default = param.default;
                    matched_count += 1;

                    // Validate: default must not exceed max
                    if default > max {
                        eprintln!(
                            "WARNING: {} default ({}) exceeds max ({}) for {} bits",
                            r.name, default, max, param.bits
                        );
                    }
                } else if !readonly {
                    unmatched.push(r.name.clone());
                }
            }

            if readonly {
                readonly_count += 1;
            }

            RegisterDef {
                section: infer_section(&r.name, r.address).to_string(),
                name: r.name.clone(),
                address: r.address,
                min: 0,
                max,
                default,
                readonly,
            }
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
                "Wrote {} register definitions to {} ({} readonly)",
                defs.len(),
                output_path,
                readonly_count
            );
            if fw_params.is_some() {
                println!(
                    "Parameter table applied: {} matched, {} unmatched",
                    matched_count,
                    unmatched.len()
                );
                if !unmatched.is_empty() {
                    println!(
                        "Unmatched registers (using default max/value): {}",
                        unmatched.join(", ")
                    );
                }
            } else {
                println!("No parameter table provided. Edit min/max/default values manually.");
            }
        }
        Err(e) => {
            eprintln!("Failed to write {}: {}", output_path, e);
            std::process::exit(1);
        }
    }
}

//! AMax FW codegen — read RegisterFile.json (FW dev output) + fw_params.json (UI metadata)
//! and emit:
//!   - `src/config/amax_generated.rs`                     (AMaxChannelConfig struct)
//!   - `src/reader/caen/amax_registers_generated.rs`      (REG_* + channel_register_byte_addr)
//!   - `web/operator-ui/src/app/models/amax-generated.ts` (interface + AMAX_*_PARAMS arrays)
//!
//! Run when FW is updated:
//!   cargo run --bin amax_codegen -- AMAX_firmware32_channel_4input_caenlist/output/output/RegisterFile.json
//!
//! Default `fw_params.json` path is `tools/amax_viewer/fw_params.json`.
//!
//! After running, `cargo build` (Rust) and `cd web/operator-ui && npm run build` (TS).

use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Generate AMax FW Rust + TypeScript bindings")]
struct Cli {
    /// Path to RegisterFile.json (from the FW developer)
    register_file: PathBuf,

    /// Path to fw_params.json (UI metadata: label/category/type/options/...)
    #[arg(short = 'p', long, default_value = "tools/amax_viewer/fw_params.json")]
    params_file: PathBuf,

    /// Output directory for Rust files (defaults to crate src/)
    #[arg(long, default_value = "src")]
    rust_out: PathBuf,

    /// Output path for the generated TypeScript file
    #[arg(
        long,
        default_value = "web/operator-ui/src/app/models/amax-generated.ts"
    )]
    ts_out: PathBuf,

    /// Channel-page base address (word units, ch0). Defaults to the new
    /// `0x100000` 32-channel-FW layout — pass `0x800000` for the legacy
    /// per-channel-paths firmware.
    #[arg(long, default_value_t = 0x100000)]
    page_base: u32,

    /// Word stride between consecutive channel pages. The 32-channel
    /// firmware exposes a single page that ch0 alone uses today, so the
    /// default stride is `0`. Pass `0x40000` for the legacy firmware.
    #[arg(long, default_value_t = 0)]
    page_stride: u32,
}

// ---- RegisterFile.json schema (subset) ----
// Two FW eras coexist:
//   - 2026-03 era: per-channel paths like `page_amax_energy_0/POLARITY`
//     served via the `Path` field, page base `0x800000`, ch1 stride `0x40000`.
//   - 2026-04 era (caenlist firmware32_4input): one set of `Name` registers
//     like `page_amax_energy_POLARITY` at page base `0x100000`, ch0 only.
// We accept either by treating `Path` and `Name` as alternates and trimming
// any `_<digit>/` channel infix when present.

#[derive(Deserialize)]
struct RegisterFile {
    #[serde(rename = "Registers")]
    registers: Vec<Register>,
}

#[derive(Deserialize)]
struct Register {
    #[serde(rename = "Address")]
    address: u32,
    #[serde(rename = "Path", default)]
    path: Option<String>,
    #[serde(rename = "Name", default)]
    name: Option<String>,
}

impl Register {
    /// Identifier used for matching against `fw_params.json`. Strips the
    /// channel infix so old per-channel paths and new single-channel names
    /// land on the same key (e.g. `THRS`).
    fn fw_key(&self) -> Option<String> {
        let raw = self.path.as_deref().or(self.name.as_deref())?;
        // Drop the leading `page_amax_energy_<infix>` chunk to get just the
        // register name. Old: "page_amax_energy_0/POLARITY" → "POLARITY".
        // New: "page_amax_energy_POLARITY" → "POLARITY".
        let stripped = raw
            .strip_prefix("page_amax_energy_0/")
            .or_else(|| raw.strip_prefix("page_amax_energy_1/"))
            .or_else(|| raw.strip_prefix("page_amax_energy_"))
            .unwrap_or(raw);
        Some(stripped.to_string())
    }

    /// True iff this register belongs to the per-channel `page_amax_energy_*`
    /// family (the only set we surface in `AMaxChannelConfig`).
    fn is_per_channel(&self) -> bool {
        let raw = self.path.as_deref().or(self.name.as_deref()).unwrap_or("");
        raw.starts_with("page_amax_energy_")
    }
}

// ---- fw_params.json schema (extended with UI metadata) ----

#[derive(Deserialize)]
struct FwParams {
    params: HashMap<String, FwParam>,
    #[serde(default)]
    readonly_patterns: Vec<String>,
}

#[derive(Deserialize, Clone)]
struct FwParam {
    bits: u32,
    #[serde(default)]
    default: u32,
    label: Option<String>,
    category: Option<String>,
    #[serde(rename = "type")]
    ty: Option<String>,
    options: Option<Vec<String>>,
    ui_max: Option<u64>,
    unit: Option<String>,
}

// ---- Resolved per-register info used by all three emitters ----

struct ResolvedReg {
    /// Original FW name (from RegisterFile.json, e.g. "POLARITY", "AMAX_window").
    fw_name: String,
    /// snake_case Rust/TS field name (e.g. "polarity", "amax_window").
    field: String,
    /// Word offset within a channel page.
    word_offset: u32,
    bits: u32,
    default: u32,
    label: String,
    category: String,
    ty: String,           // "number" or "enum"
    options: Vec<String>, // empty unless ty == "enum"
    ui_max: u64,
    unit: Option<String>,
}

fn snake_case(name: &str) -> String {
    name.to_lowercase()
}

fn category_label(cat: &str) -> &'static str {
    // Stable ordering for emitted const arrays.
    match cat {
        "input" => "Input",
        "trigger" => "Trigger",
        "energy" => "Energy",
        "waveform" => "Waveform",
        _ => "Other",
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let register_text = fs::read_to_string(&cli.register_file)?;
    let register_file: RegisterFile = serde_json::from_str(&register_text)?;

    let params_text = fs::read_to_string(&cli.params_file)?;
    let fw_params: FwParams = serde_json::from_str(&params_text)?;

    // Pull every per-channel register, regardless of FW era. The new
    // 32-channel firmware lists each register once (no `_0/` infix); the
    // legacy per-channel-paths firmware lists ch0 first, ch1 next — we
    // de-duplicate on the FW key so ch1's copy doesn't double-register.
    let mut ch0: Vec<&Register> = register_file
        .registers
        .iter()
        .filter(|r| r.is_per_channel() && !r.path.as_deref().unwrap_or("").contains("page_amax_energy_1/"))
        .collect();
    ch0.sort_by_key(|r| r.address);

    // Resolve each writable register against fw_params.json.
    let mut resolved: Vec<ResolvedReg> = Vec::new();
    let mut skipped_readonly: Vec<String> = Vec::new();
    let mut skipped_no_meta: Vec<String> = Vec::new();
    for r in &ch0 {
        let Some(name) = r.fw_key() else { continue };
        if fw_params
            .readonly_patterns
            .iter()
            .any(|p| name.contains(p))
        {
            skipped_readonly.push(name);
            continue;
        }
        let Some(meta) = fw_params.params.get(&name) else {
            skipped_no_meta.push(name);
            continue;
        };
        let bits = meta.bits;
        let ui_max = meta
            .ui_max
            .unwrap_or_else(|| if bits >= 32 { (1u64 << 32) - 1 } else { (1u64 << bits) - 1 });
        let ty = meta.ty.clone().unwrap_or_else(|| "number".to_string());
        // The "word offset" we emit is whatever `set_user_register` needs —
        // for the new FW (page_base = 0x100000) we treat the offset as the
        // raw address minus that base. SHAP_TRIGG / SHAP_BL_HOLD live on
        // a separate page (0x1C0000) but the byte-address math still works
        // because we encode the full distance from page_base.
        let word_offset = r.address.saturating_sub(cli.page_base);
        resolved.push(ResolvedReg {
            field: snake_case(&name),
            word_offset,
            bits,
            default: meta.default,
            label: meta.label.clone().unwrap_or_else(|| name.clone()),
            category: meta.category.clone().unwrap_or_else(|| "other".to_string()),
            ty,
            options: meta.options.clone().unwrap_or_default(),
            ui_max,
            unit: meta.unit.clone(),
            fw_name: name,
        });
    }

    if !skipped_no_meta.is_empty() {
        eprintln!(
            "warning: {} register(s) in RegisterFile.json have no entry in fw_params.json (skipped):",
            skipped_no_meta.len()
        );
        for n in &skipped_no_meta {
            eprintln!("  - {}", n);
        }
    }

    eprintln!(
        "amax_codegen: {} writable registers, {} read-only skipped",
        resolved.len(),
        skipped_readonly.len()
    );

    let rust_struct_path = cli.rust_out.join("config/amax_generated.rs");
    let rust_reg_path = cli
        .rust_out
        .join("reader/caen/amax_registers_generated.rs");

    fs::write(
        &rust_struct_path,
        emit_rust_struct(&resolved, &cli.register_file, &cli.params_file),
    )?;
    fs::write(
        &rust_reg_path,
        emit_rust_registers(&resolved, cli.page_base, cli.page_stride, &cli.register_file),
    )?;
    fs::write(
        &cli.ts_out,
        emit_typescript(&resolved, &cli.register_file, &cli.params_file),
    )?;

    eprintln!("wrote {}", rust_struct_path.display());
    eprintln!("wrote {}", rust_reg_path.display());
    eprintln!("wrote {}", cli.ts_out.display());
    Ok(())
}

// ---- Rust struct emitter ----

fn header_rust(register_file: &std::path::Path, params_file: &std::path::Path) -> String {
    format!(
        "//! AUTO-GENERATED by `cargo run --bin amax_codegen`. Do not edit by hand.\n\
         //!\n\
         //! Sources:\n\
         //! - {}\n\
         //! - {}\n",
        register_file.display(),
        params_file.display()
    )
}

fn emit_rust_struct(
    resolved: &[ResolvedReg],
    register_file: &std::path::Path,
    params_file: &std::path::Path,
) -> String {
    let mut out = String::new();
    out.push_str(&header_rust(register_file, params_file));
    out.push_str(
        "//!\n\
         //! AMax custom-firmware per-channel writable register set.\n\
         //! Each field maps 1:1 to one user-register write at\n\
         //! `(PAGE_BASE + channel * PAGE_STRIDE + REG_<NAME>) * 4` byte\n\
         //! address (see `amax_registers_generated::channel_register_byte_addr`).\n\
         \n\
         use serde::{Deserialize, Serialize};\n\
         use utoipa::ToSchema;\n\
         \n\
         #[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]\n\
         pub struct AMaxChannelConfig {\n",
    );
    for r in resolved {
        let unit = r
            .unit
            .as_deref()
            .map(|u| format!(", unit: {}", u))
            .unwrap_or_default();
        out.push_str(&format!(
            "    /// {}-bit {}{} (FW reg `{}`)\n",
            r.bits, r.label, unit, r.fw_name
        ));
        out.push_str("    #[serde(skip_serializing_if = \"Option::is_none\")]\n");
        out.push_str(&format!("    pub {}: Option<u32>,\n", r.field));
    }
    out.push_str("}\n");
    out
}

// ---- Rust registers emitter ----

fn emit_rust_registers(
    resolved: &[ResolvedReg],
    page_base: u32,
    page_stride: u32,
    register_file: &std::path::Path,
) -> String {
    let mut out = String::new();
    out.push_str(&header_rust(register_file, std::path::Path::new("(addresses only)")));
    out.push_str(
        "//!\n\
         //! AMax custom-firmware register address map. Byte address =\n\
         //! `(PAGE_BASE + channel * PAGE_STRIDE + word_offset) * 4` per the\n\
         //! FELib `SetUserRegister` ABI used by `tools/amax_viewer/`.\n\
         \n",
    );
    out.push_str(&format!(
        "/// Channel-page base in word units (channel 0).\n\
         pub const PAGE_BASE: u32 = 0x{:X};\n\
         \n\
         /// Word stride between consecutive channel pages.\n\
         pub const PAGE_STRIDE: u32 = 0x{:X};\n\
         \n",
        page_base, page_stride
    ));
    out.push_str("// ---- Per-channel register offsets (word, relative to the channel page base) ----\n\n");
    for r in resolved {
        out.push_str(&format!("/// {}-bit {}\n", r.bits, r.label));
        out.push_str(&format!(
            "pub const REG_{}: u32 = 0x{:X};\n",
            r.field.to_uppercase(),
            r.word_offset
        ));
    }
    out.push_str(
        "\n\
         /// Compute the FELib byte address for a per-channel register.\n\
         #[inline]\n\
         pub fn channel_register_byte_addr(channel: u8, word_offset: u32) -> u32 {\n\
         \x20\x20\x20\x20let word_addr = PAGE_BASE + (channel as u32) * PAGE_STRIDE + word_offset;\n\
         \x20\x20\x20\x20word_addr * 4\n\
         }\n",
    );

    // Codegen-driven `apply` helper — handle.rs iterates this list so the
    // hand-written field array doesn't go stale every time the FW gains or
    // drops a register. Each tuple is (REG_offset, value, field_name).
    out.push_str("\n/// All writable per-channel register fields, in stable order.\n");
    out.push_str("/// Used by `apply_amax_channel_config` to drive `set_user_register` calls.\n");
    out.push_str("#[allow(dead_code)]\n");
    out.push_str("pub fn channel_writes(\n");
    out.push_str("    config: &crate::config::digitizer::AMaxChannelConfig,\n");
    out.push_str(") -> Vec<(u32, u32, &'static str)> {\n");
    out.push_str("    let mut writes = Vec::new();\n");
    for r in resolved {
        out.push_str(&format!(
            "    if let Some(v) = config.{f} {{ writes.push((REG_{u}, v, \"{f}\")); }}\n",
            f = r.field,
            u = r.field.to_uppercase(),
        ));
    }
    out.push_str("    writes\n");
    out.push_str("}\n");

    // Lightweight test: every offset distinct, byte-addr math stays consistent.
    out.push_str(
        "\n\
         #[cfg(test)]\n\
         mod tests {\n\
         \x20\x20\x20\x20use super::*;\n\
         \n\
         \x20\x20\x20\x20#[test]\n\
         \x20\x20\x20\x20fn register_offsets_are_distinct() {\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let offsets = [\n",
    );
    for r in resolved {
        out.push_str(&format!(
            "\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20REG_{},\n",
            r.field.to_uppercase()
        ));
    }
    out.push_str(
        "\x20\x20\x20\x20\x20\x20\x20\x20];\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let mut sorted = offsets.to_vec();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20sorted.sort_unstable();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20sorted.dedup();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20assert_eq!(sorted.len(), offsets.len());\n\
         \x20\x20\x20\x20}\n\
         \n\
         \x20\x20\x20\x20#[test]\n\
         \x20\x20\x20\x20fn channel_addr_math() {\n\
         \x20\x20\x20\x20\x20\x20\x20\x20// ch0 base byte addr = PAGE_BASE * 4 (word→byte).\n\
         \x20\x20\x20\x20\x20\x20\x20\x20assert_eq!(channel_register_byte_addr(0, 0), PAGE_BASE * 4);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20// Each channel index advances by PAGE_STRIDE words (=4× bytes).\n\
         \x20\x20\x20\x20\x20\x20\x20\x20assert_eq!(\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20channel_register_byte_addr(1, 0),\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20channel_register_byte_addr(0, 0) + PAGE_STRIDE * 4\n\
         \x20\x20\x20\x20\x20\x20\x20\x20);\n\
         \x20\x20\x20\x20}\n\
         }\n",
    );
    out
}

// ---- TypeScript emitter ----

fn emit_typescript(
    resolved: &[ResolvedReg],
    register_file: &std::path::Path,
    params_file: &std::path::Path,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "// AUTO-GENERATED by `cargo run --bin amax_codegen`. Do not edit by hand.\n\
         //\n\
         // Sources:\n\
         // - {}\n\
         // - {}\n\
         \n\
         import type {{ ChannelParamDef }} from '../components/channel-table/channel-table.component';\n\
         \n\
         export interface AMaxChannelConfig {{\n",
        register_file.display(),
        params_file.display()
    ));
    for r in resolved {
        out.push_str(&format!("  /** {}-bit {} */\n", r.bits, r.label));
        out.push_str(&format!("  {}?: number;\n", r.field));
    }
    out.push_str("}\n\n");

    // FW defaults table for the "Reset AMax to FW defaults" UI button.
    out.push_str("/** Per-key default values from `fw_params.json` (FW developer defaults). */\n");
    out.push_str("export const AMAX_DEFAULTS: Record<string, number> = {\n");
    for r in resolved {
        out.push_str(&format!("  'amax.{}': {},\n", r.field, r.default));
    }
    out.push_str("};\n\n");

    // Dotted-key allowlist for `digitizer.service.ts` expand/compress —
    // every per-channel AMax field the UI is allowed to round-trip. Adding
    // a new register here used to require a hand edit; emitting it from the
    // same canonical list keeps Settings tab in sync with the FW.
    out.push_str("/** All AMax dotted-path keys (`amax.<field>`). Used by the\n");
    out.push_str(" *  Settings expand/compress logic in `digitizer.service.ts`. */\n");
    out.push_str("export const AMAX_DOTTED_KEYS: readonly string[] = [\n");
    for r in resolved {
        out.push_str(&format!("  'amax.{}',\n", r.field));
    }
    out.push_str("];\n\n");

    // Emit one const array per category, in stable order.
    for cat in ["input", "trigger", "energy", "waveform"] {
        let const_name = format!("AMAX_{}_PARAMS", category_label(cat).to_uppercase());
        out.push_str(&format!(
            "export const {}: ChannelParamDef[] = [\n",
            const_name
        ));
        for r in resolved.iter().filter(|r| r.category == cat) {
            // Hex address tooltip: original FW name + page-relative word offset
            // + canonical ch0 word address (0x800000-base, what amax_viewer uses).
            let ch0_word = 0x800000 + r.word_offset;
            let tooltip = format!(
                "FW reg {} • word 0x{:02X} (ch0 @ 0x{:06X})",
                r.fw_name, r.word_offset, ch0_word
            );
            let mut line = format!(
                "  {{ key: 'amax.{}', label: {:?}",
                r.field, r.label
            );
            if r.ty == "enum" {
                let opts = r
                    .options
                    .iter()
                    .map(|o| format!("{:?}", o))
                    .collect::<Vec<_>>()
                    .join(", ");
                line.push_str(&format!(", type: 'enum', options: [{}]", opts));
            } else {
                line.push_str(", type: 'number'");
                if let Some(u) = &r.unit {
                    line.push_str(&format!(", unit: {:?}", u));
                }
                line.push_str(&format!(", min: 0, max: {}, step: 1", r.ui_max));
            }
            line.push_str(&format!(", tooltip: {:?}", tooltip));
            line.push_str(" },\n");
            out.push_str(&line);
        }
        out.push_str("];\n\n");
    }
    out
}

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

    /// Channel-page base address (word units, ch0). Defaults to the 13may
    /// caenlist FW broadcast page at `0x200`. Pass `0x100000` for the older
    /// 32-ch FW (single broadcast page, no slash), `0x800000` for the legacy
    /// 2026-03 per-channel-paths firmware.
    #[arg(long, default_value_t = 0x200)]
    page_base: u32,

    /// Word stride between consecutive channel pages. Broadcast-page FWs use
    /// `0` (every channel iteration hits the same address). Pass `0x40000`
    /// for the legacy 2026-03 firmware. The 13may FW also exposes 32
    /// per-channel pages at stride `0x200`, but `apply_amax_channel_config`
    /// still drives the broadcast page — a single write fans out to all
    /// channels in hardware, so the channel loop is intentionally redundant.
    #[arg(long, default_value_t = 0)]
    page_stride: u32,

    /// Pick the ch0 per-channel page instead of the broadcast page as the
    /// canonical register set. Use when the broadcast slash form
    /// (`page_amax_energy/<NAME>`) does NOT fan out to all channels in
    /// hardware (observed on the 13may caenlist FW) and per-channel
    /// pages (`page_amax_energy_4_<N>/<NAME>`) must be written individually.
    /// Pair with `--page-base 0x4000 --page-stride 0x200`.
    #[arg(long, default_value_t = false)]
    prefer_per_channel: bool,
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
    ///
    /// Four FW eras coexist in this strip chain:
    ///   - 2026-03 era: `page_amax_energy_0/THRS` → `THRS` (slash separator)
    ///   - 2026-04 32-ch era: `page_amax_energy_THRS` → `THRS` (no index)
    ///   - 2026-05 16-ch era: `page_amax_energy_15_THRS` → `THRS`
    ///     (underscore-separated index, broadcast page also has bare form)
    ///   - 2026-05 caenlist-13may era: broadcast `page_amax_energy/THRS`
    ///     (slash form, no index), per-channel `page_amax_energy_4_<N>/THRS`
    fn fw_key(&self) -> Option<String> {
        let raw = self.path.as_deref().or(self.name.as_deref())?;
        // 13may broadcast form: `page_amax_energy/<NAME>` — single broadcast
        // page exposed with slash separator (no `_4_<N>/` infix).
        if let Some(rest) = raw.strip_prefix("page_amax_energy/") {
            return Some(rest.to_string());
        }
        // 13may per-channel form: `page_amax_energy_4_<N>/<NAME>` — strip the
        // `_4_<N>/` chunk down to the bare register name.
        if let Some(rest) = raw.strip_prefix("page_amax_energy_4_") {
            if let Some(slash) = rest.find('/') {
                let idx = &rest[..slash];
                if idx.chars().all(|c| c.is_ascii_digit()) {
                    return Some(rest[slash + 1..].to_string());
                }
            }
        }
        let stripped = raw
            .strip_prefix("page_amax_energy_0/")
            .or_else(|| raw.strip_prefix("page_amax_energy_1/"))
            .or_else(|| raw.strip_prefix("page_amax_energy_"))
            .unwrap_or(raw);
        // For the 2026-05 era we may still have a leading `<digits>_` from
        // the per-channel page (e.g. "5_THRS"). Strip it so the key matches
        // the bare register name in fw_params.json.
        let trimmed = match stripped.find('_') {
            Some(idx) if stripped[..idx].chars().all(|c| c.is_ascii_digit()) => {
                &stripped[idx + 1..]
            }
            _ => stripped,
        };
        Some(trimmed.to_string())
    }

    /// True iff this register belongs to the per-channel `page_amax_energy_*`
    /// family (the only set we surface in `AMaxChannelConfig`). Also true for
    /// the 13may era broadcast slash form `page_amax_energy/<NAME>` so the
    /// canonical-filter and codegen treat broadcast + per-channel uniformly.
    fn is_per_channel(&self) -> bool {
        let raw = self.path.as_deref().or(self.name.as_deref()).unwrap_or("");
        raw.starts_with("page_amax_energy_") || raw.starts_with("page_amax_energy/")
    }

    /// Returns the channel index when this register is one of the per-channel
    /// pages (`page_amax_energy_<N>_<NAME>` underscore-form or
    /// `page_amax_energy_<N>/<NAME>` slash-form). Returns `None` for the
    /// broadcast page (`page_amax_energy_<NAME>` with no index — write fans
    /// out to all channels) and for any non-`page_amax_energy_*` register.
    ///
    /// The codegen filter uses this to keep only the broadcast (canonical)
    /// register set; per-channel pages would otherwise emit duplicate
    /// `REG_*` constants. Live AMax operation goes through the broadcast
    /// page anyway (single write hits every channel — see
    /// `apply_amax_channel_config` in `src/reader/caen/handle.rs`).
    fn channel_index(&self) -> Option<u32> {
        let raw = self.path.as_deref().or(self.name.as_deref())?;
        // 13may broadcast (`page_amax_energy/<NAME>`) = canonical, no index.
        if raw.starts_with("page_amax_energy/") {
            return None;
        }
        // 13may per-channel form: `page_amax_energy_4_<N>/<NAME>` — the `_4_`
        // prefix denotes a 4-input page, `<N>` is the channel index.
        if let Some(rest) = raw.strip_prefix("page_amax_energy_4_") {
            if let Some(slash) = rest.find('/') {
                let idx = &rest[..slash];
                if idx.chars().all(|c| c.is_ascii_digit()) {
                    return idx.parse().ok();
                }
            }
        }
        let body = raw.strip_prefix("page_amax_energy_")?;
        // 2026-03 slash form: "<N>/<NAME>"
        if let Some(slash) = body.find('/') {
            return body[..slash].parse().ok();
        }
        // 2026-05 underscore form: "<N>_<NAME>" — only when prefix is all digits.
        if let Some(uscore) = body.find('_') {
            let prefix = &body[..uscore];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
                return prefix.parse().ok();
            }
        }
        None // bare "<NAME>" = broadcast page
    }

    /// True iff this register's name matches a key in `fw_params.board_params`
    /// — i.e. a global / board-level register (e.g. `ENABLE_ACQ` for the AMax
    /// debug FW). Per-channel registers always take priority over board-level
    /// matches so we don't accidentally promote a channel reg to global.
    fn is_board_level(&self, board_param_keys: &[String]) -> bool {
        if self.is_per_channel() {
            return false;
        }
        let raw = self.path.as_deref().or(self.name.as_deref()).unwrap_or("");
        // Strip leading slash so JSON `"Path": "/ENABLE_ACQ"` matches
        // `"ENABLE_ACQ"`.
        let trimmed = raw.trim_start_matches('/');
        board_param_keys
            .iter()
            .any(|k| trimmed == k || trimmed.contains(k))
    }
}

/// Pick the canonical per-channel register entries from a `Register` list.
/// Across FW eras the canonical entry is the one without a channel index:
///   - 2026-03 era: only per-channel pages exist (`page_amax_energy_0/...`,
///     `page_amax_energy_1/...`, ...) — we fall back to ch0 (lowest index)
///     so the page is represented at all.
///   - 2026-04 32-ch era: `page_amax_energy_<NAME>` only (no index, single
///     broadcast page) — picked directly.
///   - 2026-05 16-ch era: broadcast `page_amax_energy_<NAME>` (no index)
///     plus 16 per-channel pages `page_amax_energy_<N>_<NAME>` — broadcast
///     wins, per-channel duplicates dropped (single write fans out to all
///     channels via hardware; see `apply_amax_channel_config`).
#[allow(dead_code)] // retained for unit-test ergonomics; production path uses `_with`
fn select_canonical_per_channel(registers: &[Register]) -> Vec<&Register> {
    select_canonical_per_channel_with(registers, false)
}

/// Same as `select_canonical_per_channel` but switchable: when
/// `prefer_per_channel` is true the ch0 per-channel page wins over the
/// broadcast page. Use when the broadcast page does not fan out to all
/// channels in hardware (observed on the 13may caenlist FW).
fn select_canonical_per_channel_with(
    registers: &[Register],
    prefer_per_channel: bool,
) -> Vec<&Register> {
    let has_broadcast = registers
        .iter()
        .any(|r| r.is_per_channel() && r.channel_index().is_none());
    let has_ch0 = registers
        .iter()
        .any(|r| r.is_per_channel() && r.channel_index() == Some(0));
    let use_per_channel = prefer_per_channel && has_ch0;
    registers
        .iter()
        .filter(|r| {
            if !r.is_per_channel() {
                return false;
            }
            match r.channel_index() {
                None => !use_per_channel, // broadcast — preferred unless we asked for per-channel
                Some(0) => use_per_channel || !has_broadcast, // ch0 wins when asked or as 2026-03 fallback
                Some(_) => false,                             // skip ch1+ duplicates
            }
        })
        .collect()
}

// ---- fw_params.json schema (extended with UI metadata) ----

#[derive(Deserialize)]
struct FwParams {
    params: HashMap<String, FwParam>,
    /// Board-level (global, non-per-channel) registers — e.g. `ENABLE_ACQ`
    /// for the AMax debug FW. Same `FwParam` shape as `params` but routed to
    /// a separate `AMaxBoardConfig` struct in the generated code.
    #[serde(default)]
    board_params: HashMap<String, FwParam>,
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
        "debug" => "Debug",
        _ => "Other",
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let register_text = fs::read_to_string(&cli.register_file)?;
    let register_file: RegisterFile = serde_json::from_str(&register_text)?;

    let params_text = fs::read_to_string(&cli.params_file)?;
    let fw_params: FwParams = serde_json::from_str(&params_text)?;

    let mut ch0: Vec<&Register> =
        select_canonical_per_channel_with(&register_file.registers, cli.prefer_per_channel);
    ch0.sort_by_key(|r| r.address);

    // Resolve each writable register against fw_params.json.
    let mut resolved: Vec<ResolvedReg> = Vec::new();
    let mut skipped_readonly: Vec<String> = Vec::new();
    let mut skipped_no_meta: Vec<String> = Vec::new();
    for r in &ch0 {
        let Some(name) = r.fw_key() else { continue };
        if fw_params.readonly_patterns.iter().any(|p| name.contains(p)) {
            skipped_readonly.push(name);
            continue;
        }
        let Some(meta) = fw_params.params.get(&name) else {
            skipped_no_meta.push(name);
            continue;
        };
        let bits = meta.bits;
        let ui_max = meta.ui_max.unwrap_or_else(|| {
            if bits >= 32 {
                (1u64 << 32) - 1
            } else {
                (1u64 << bits) - 1
            }
        });
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

    // Second pass: pick up board-level (global) registers declared in
    // `fw_params.board_params`. Currently this is just `ENABLE_ACQ` for the
    // AMax debug FW, but the same path supports any future global toggle.
    let board_param_keys: Vec<String> = fw_params.board_params.keys().cloned().collect();
    let board_resolved: Vec<ResolvedReg> = if board_param_keys.is_empty() {
        Vec::new()
    } else {
        let mut br: Vec<&Register> = register_file
            .registers
            .iter()
            .filter(|r| r.is_board_level(&board_param_keys))
            .collect();
        br.sort_by_key(|r| r.address);

        let mut out = Vec::new();
        for r in &br {
            // Use the raw name (no `page_amax_energy_` prefix to strip).
            let raw = r.path.as_deref().or(r.name.as_deref()).unwrap_or("");
            let trimmed = raw.trim_start_matches('/');
            let name = board_param_keys
                .iter()
                .find(|k| trimmed == k.as_str() || trimmed.contains(k.as_str()))
                .cloned()
                .unwrap_or_else(|| trimmed.to_string());
            let Some(meta) = fw_params.board_params.get(&name) else {
                continue;
            };
            let bits = meta.bits;
            let ui_max = meta.ui_max.unwrap_or_else(|| {
                if bits >= 32 {
                    (1u64 << 32) - 1
                } else {
                    (1u64 << bits) - 1
                }
            });
            let ty = meta.ty.clone().unwrap_or_else(|| "number".to_string());
            // Board registers carry the *raw* address from the FW because
            // there's no per-channel page offset to factor out.
            out.push(ResolvedReg {
                field: snake_case(&name),
                word_offset: r.address,
                bits,
                default: meta.default,
                label: meta.label.clone().unwrap_or_else(|| name.clone()),
                category: meta.category.clone().unwrap_or_else(|| "debug".to_string()),
                ty,
                options: meta.options.clone().unwrap_or_default(),
                ui_max,
                unit: meta.unit.clone(),
                fw_name: name,
            });
        }
        out
    };

    eprintln!(
        "amax_codegen: {} per-channel writable, {} board-level, {} read-only skipped",
        resolved.len(),
        board_resolved.len(),
        skipped_readonly.len()
    );

    let rust_struct_path = cli.rust_out.join("config/amax_generated.rs");
    let rust_reg_path = cli.rust_out.join("reader/caen/amax_registers_generated.rs");

    fs::write(
        &rust_struct_path,
        emit_rust_struct(
            &resolved,
            &board_resolved,
            &cli.register_file,
            &cli.params_file,
        ),
    )?;
    fs::write(
        &rust_reg_path,
        emit_rust_registers(
            &resolved,
            &board_resolved,
            cli.page_base,
            cli.page_stride,
            &cli.register_file,
        ),
    )?;
    fs::write(
        &cli.ts_out,
        emit_typescript(
            &resolved,
            &board_resolved,
            &cli.register_file,
            &cli.params_file,
        ),
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
    board_resolved: &[ResolvedReg],
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

    // Board-level (global) writable register set. Each field maps to one
    // single-address user-register write (no channel stride). Currently
    // populated for the AMax debug FW's `ENABLE_ACQ` toggle.
    out.push_str(
        "\n/// AMax board-level (global) writable register set. Used for\n\
         /// firmware-wide toggles like the debug-FW `ENABLE_ACQ` switch.\n\
         #[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]\n\
         pub struct AMaxBoardConfig {\n",
    );
    for r in board_resolved {
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
    board_resolved: &[ResolvedReg],
    page_base: u32,
    page_stride: u32,
    register_file: &std::path::Path,
) -> String {
    let mut out = String::new();
    out.push_str(&header_rust(
        register_file,
        std::path::Path::new("(addresses only)"),
    ));
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
    out.push_str(
        "// ---- Per-channel register offsets (word, relative to the channel page base) ----\n\n",
    );
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

    // ---- Board-level (global) register addresses + apply helper ----
    //
    // Always emit `board_writes` and `all_board_registers` (even when empty)
    // so callers can iterate unconditionally. BOARD_REG_* constants only
    // appear when `board_resolved` is non-empty.
    if !board_resolved.is_empty() {
        out.push_str("\n// ---- Board-level (global) register byte addresses ----\n//\n");
        out.push_str(
            "// These registers live outside the per-channel `page_amax_energy_*`\n\
             // family — they're firmware-wide toggles. Byte addresses are taken\n\
             // verbatim from RegisterFile.json (no PAGE_BASE math).\n\n",
        );
        for r in board_resolved {
            out.push_str(&format!(
                "/// {}-bit {} (FW reg `{}`)\n",
                r.bits, r.label, r.fw_name
            ));
            out.push_str(&format!(
                "pub const BOARD_REG_{}: u32 = 0x{:X};\n",
                r.field.to_uppercase(),
                r.word_offset
            ));
        }
    }

    out.push_str(
        "\n/// All writable board-level register fields, in stable order.\n\
         /// Mirror of `channel_writes` for `AMaxBoardConfig`. Each tuple is\n\
         /// (BOARD_REG byte address, value, field_name). Empty when no\n\
         /// board-level (global) registers are declared in fw_params.json.\n\
         #[allow(dead_code)]\n\
         pub fn board_writes(\n\
         \x20\x20\x20\x20_config: &crate::config::digitizer::AMaxBoardConfig,\n\
         ) -> Vec<(u32, u32, &'static str)> {\n\
         \x20\x20\x20\x20#[allow(unused_mut)]\n\
         \x20\x20\x20\x20let mut writes = Vec::new();\n",
    );
    for r in board_resolved {
        out.push_str(&format!(
            "    if let Some(v) = _config.{f} {{ writes.push((BOARD_REG_{u}, v, \"{f}\")); }}\n",
            f = r.field,
            u = r.field.to_uppercase(),
        ));
    }
    out.push_str("    writes\n}\n");

    out.push_str(
        "\n/// All board-level register (byte_addr, name) pairs, in stable order.\n\
         /// Used by the operator UI's Tune Up debug view to read live\n\
         /// values back from the digitizer (see\n\
         /// `ReadLoopRequest::ReadAmaxBoardRegisters` in `reader/mod.rs`).\n\
         /// Empty when no board-level registers are declared.\n\
         #[allow(dead_code)]\n\
         pub fn all_board_registers() -> Vec<(u32, &'static str)> {\n\
         \x20\x20\x20\x20vec![\n",
    );
    for r in board_resolved {
        out.push_str(&format!(
            "        (BOARD_REG_{u}, \"{f}\"),\n",
            f = r.field,
            u = r.field.to_uppercase(),
        ));
    }
    out.push_str("    ]\n}\n");

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
    board_resolved: &[ResolvedReg],
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
            let mut line = format!("  {{ key: 'amax.{}', label: {:?}", r.field, r.label);
            if r.ty == "enum" {
                let opts = r
                    .options
                    .iter()
                    .map(|o| format!("{:?}", o))
                    .collect::<Vec<_>>()
                    .join(", ");
                // AMax enums map 1:1 to firmware register bits, so the value
                // must reach the backend as a Number — see channel-table's
                // `coerceEnum` helper which honours `numeric: true`.
                line.push_str(&format!(
                    ", type: 'enum', options: [{}], numeric: true",
                    opts
                ));
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

    // ---- Board-level (global) interface, defaults, and params ----
    out.push_str("/** AMax board-level (global) writable register set —\n");
    out.push_str(" *  firmware-wide toggles like the debug-FW `ENABLE_ACQ`. */\n");
    out.push_str("export interface AMaxBoardConfig {\n");
    for r in board_resolved {
        out.push_str(&format!("  /** {}-bit {} */\n", r.bits, r.label));
        out.push_str(&format!("  {}?: number;\n", r.field));
    }
    out.push_str("}\n\n");

    out.push_str("/** Per-key default values for board-level registers. */\n");
    out.push_str("export const AMAX_BOARD_DEFAULTS: Record<string, number> = {\n");
    for r in board_resolved {
        out.push_str(&format!("  '{}': {},\n", r.field, r.default));
    }
    out.push_str("};\n\n");

    out.push_str("/** Board-level dotted-path keys (`amax.board.<field>`). */\n");
    out.push_str("export const AMAX_BOARD_DOTTED_KEYS: readonly string[] = [\n");
    for r in board_resolved {
        out.push_str(&format!("  'amax.board.{}',\n", r.field));
    }
    out.push_str("];\n\n");

    out.push_str("/** Board-level parameter UI definitions. */\n");
    out.push_str("export const AMAX_BOARD_PARAMS: ChannelParamDef[] = [\n");
    for r in board_resolved {
        let tooltip = format!("FW reg {} • addr 0x{:X}", r.fw_name, r.word_offset);
        let mut line = format!("  {{ key: 'amax.board.{}', label: {:?}", r.field, r.label);
        if r.ty == "enum" {
            let opts = r
                .options
                .iter()
                .map(|o| format!("{:?}", o))
                .collect::<Vec<_>>()
                .join(", ");
            line.push_str(&format!(
                ", type: 'enum', options: [{}], numeric: true",
                opts
            ));
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
    out.push_str("];\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(name_or_path: &str) -> Register {
        // Two of the same string lets the test cover both `Path` and `Name`
        // alternates without two helpers.
        Register {
            address: 0,
            path: Some(name_or_path.to_string()),
            name: Some(name_or_path.to_string()),
        }
    }

    #[test]
    fn fw_key_handles_thirteen_may_era() {
        // 13may broadcast slash form
        assert_eq!(
            reg("page_amax_energy/POLARITY").fw_key().as_deref(),
            Some("POLARITY")
        );
        assert_eq!(
            reg("page_amax_energy/baseline_delay").fw_key().as_deref(),
            Some("baseline_delay")
        );
        // 13may per-channel slash form with `_4_<N>/` infix
        assert_eq!(
            reg("page_amax_energy_4_0/POLARITY").fw_key().as_deref(),
            Some("POLARITY")
        );
        assert_eq!(
            reg("page_amax_energy_4_31/THRS").fw_key().as_deref(),
            Some("THRS")
        );
        assert_eq!(
            reg("page_amax_energy_4_15/baseline_delay")
                .fw_key()
                .as_deref(),
            Some("baseline_delay")
        );
    }

    #[test]
    fn fw_key_handles_three_eras() {
        // 2026-03 era — slash separator after channel index
        assert_eq!(
            reg("page_amax_energy_0/THRS").fw_key().as_deref(),
            Some("THRS")
        );
        assert_eq!(
            reg("page_amax_energy_1/POLARITY").fw_key().as_deref(),
            Some("POLARITY")
        );

        // 2026-04 32-ch era — no channel index, just bare name
        assert_eq!(
            reg("page_amax_energy_THRS").fw_key().as_deref(),
            Some("THRS")
        );
        assert_eq!(
            reg("page_amax_energy_baseline_delay").fw_key().as_deref(),
            Some("baseline_delay")
        );

        // 2026-05 16-ch era — underscore-separated channel index
        assert_eq!(
            reg("page_amax_energy_15_THRS").fw_key().as_deref(),
            Some("THRS")
        );
        assert_eq!(
            reg("page_amax_energy_0_POLARITY").fw_key().as_deref(),
            Some("POLARITY")
        );
        assert_eq!(
            reg("page_amax_energy_7_baseline_delay").fw_key().as_deref(),
            Some("baseline_delay")
        );
    }

    #[test]
    fn fw_key_preserves_unprefixed_names() {
        // Global registers (no `page_amax_energy_` prefix) come through as-is.
        assert_eq!(reg("ENABLE_ACQ").fw_key().as_deref(), Some("ENABLE_ACQ"));
        assert_eq!(reg("AMAX_gol").fw_key().as_deref(), Some("AMAX_gol"));
    }

    #[test]
    fn fw_key_does_not_misclassify_underscore_names() {
        // Names starting with a non-digit followed by underscore (e.g.
        // `baseline_delay`) must NOT have their leading word stripped —
        // only digit prefixes get treated as channel indices.
        assert_eq!(
            reg("page_amax_energy_baseline_delay").fw_key().as_deref(),
            Some("baseline_delay")
        );
    }

    #[test]
    fn channel_index_handles_thirteen_may_era() {
        // 13may broadcast slash form — canonical (no index)
        assert_eq!(reg("page_amax_energy/POLARITY").channel_index(), None);
        assert_eq!(reg("page_amax_energy/baseline_delay").channel_index(), None);
        // 13may per-channel `_4_<N>/` form
        assert_eq!(
            reg("page_amax_energy_4_0/POLARITY").channel_index(),
            Some(0)
        );
        assert_eq!(reg("page_amax_energy_4_15/THRS").channel_index(), Some(15));
        assert_eq!(reg("page_amax_energy_4_31/THRS").channel_index(), Some(31));
    }

    #[test]
    fn is_per_channel_recognises_broadcast_slash_form() {
        // 13may broadcast slash form must be considered "per-channel" so the
        // canonical filter picks it up as the broadcast (None index) entry.
        assert!(reg("page_amax_energy/POLARITY").is_per_channel());
        assert!(reg("page_amax_energy/baseline_delay").is_per_channel());
        assert!(reg("page_amax_energy_4_0/POLARITY").is_per_channel());
    }

    #[test]
    fn channel_index_distinguishes_broadcast_from_per_channel() {
        // 2026-03 slash form
        assert_eq!(reg("page_amax_energy_0/THRS").channel_index(), Some(0));
        assert_eq!(reg("page_amax_energy_15/THRS").channel_index(), Some(15));

        // 2026-05 underscore form
        assert_eq!(reg("page_amax_energy_0_THRS").channel_index(), Some(0));
        assert_eq!(reg("page_amax_energy_15_THRS").channel_index(), Some(15));

        // Broadcast page (no index) — None means "this is the canonical entry"
        assert_eq!(reg("page_amax_energy_THRS").channel_index(), None);
        assert_eq!(reg("page_amax_energy_baseline_delay").channel_index(), None);

        // Non-page_amax_energy_ registers — None
        assert_eq!(reg("ENABLE_ACQ").channel_index(), None);
        assert_eq!(reg("AMAX_gol").channel_index(), None);
    }

    #[test]
    fn is_per_channel_matches_page_amax_energy_prefix() {
        assert!(reg("page_amax_energy_THRS").is_per_channel());
        assert!(reg("page_amax_energy_0_THRS").is_per_channel());
        assert!(reg("page_amax_energy_15_baseline_delay").is_per_channel());
        assert!(reg("page_amax_energy_0/THRS").is_per_channel()); // legacy slash form
        assert!(!reg("ENABLE_ACQ").is_per_channel());
        assert!(!reg("AMAX_gol").is_per_channel());
        assert!(!reg("/ENABLE_ACQ").is_per_channel());
    }

    // -----------------------------------------------------------------------
    // Integration tests: drive `select_canonical_per_channel` against
    // synthetic register lists representative of each FW era. These pin the
    // filter behaviour so future FW revisions that introduce a new naming
    // scheme don't silently break codegen output.
    // -----------------------------------------------------------------------

    /// Helper: build a Register with a specific address (so we can assert on
    /// which entry "won" when broadcast and per-channel both exist).
    fn reg_at(addr: u32, name_or_path: &str) -> Register {
        Register {
            address: addr,
            path: Some(name_or_path.to_string()),
            name: Some(name_or_path.to_string()),
        }
    }

    /// Sort + project the result to (addr, name) tuples for stable assertions.
    fn picked(regs: &[&Register]) -> Vec<(u32, String)> {
        let mut out: Vec<(u32, String)> = regs
            .iter()
            .map(|r| {
                (
                    r.address,
                    r.path.clone().or(r.name.clone()).unwrap_or_default(),
                )
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn canonical_filter_2026_03_era_keeps_ch0_only() {
        // 2026-03 era: per-channel pages with slash separator. NO broadcast
        // page — the filter must fall back to ch0 (lowest index) so the
        // generated AMaxChannelConfig has every register.
        let regs = vec![
            reg_at(0x800002, "page_amax_energy_0/POLARITY"),
            reg_at(0x800004, "page_amax_energy_0/THRS"),
            reg_at(0x840002, "page_amax_energy_1/POLARITY"),
            reg_at(0x840004, "page_amax_energy_1/THRS"),
            reg_at(0x880002, "page_amax_energy_2/POLARITY"),
            reg_at(0x880004, "page_amax_energy_2/THRS"),
            // Sibling globals (not per-channel) must be filtered out.
            reg_at(0x80008, "ENABLE_ACQ"),
            reg_at(0x80007, "AMAX_gol"),
        ];
        let picked = picked(&select_canonical_per_channel(&regs));
        assert_eq!(
            picked,
            vec![
                (0x800002, "page_amax_energy_0/POLARITY".to_string()),
                (0x800004, "page_amax_energy_0/THRS".to_string()),
            ],
            "2026-03 era: ch0 wins, ch1+ + globals dropped"
        );
    }

    #[test]
    fn canonical_filter_2026_04_era_keeps_bare_names() {
        // 2026-04 32-ch era: a single broadcast page with no channel index.
        // No per-channel pages exist at all — the filter keeps everything
        // that lives under `page_amax_energy_*` and isn't otherwise.
        let regs = vec![
            reg_at(0x100002, "page_amax_energy_POLARITY"),
            reg_at(0x100004, "page_amax_energy_THRS"),
            reg_at(0x100006, "page_amax_energy_baseline_delay"),
            // Globals filtered out.
            reg_at(0x80008, "ENABLE_ACQ"),
        ];
        let picked = picked(&select_canonical_per_channel(&regs));
        assert_eq!(
            picked,
            vec![
                (0x100002, "page_amax_energy_POLARITY".to_string()),
                (0x100004, "page_amax_energy_THRS".to_string()),
                (0x100006, "page_amax_energy_baseline_delay".to_string()),
            ],
            "2026-04 era: all bare-name broadcast entries kept"
        );
    }

    #[test]
    fn canonical_filter_2026_05_caenlist_13may_picks_broadcast_drops_per_channel() {
        // 13may caenlist era: broadcast page lives at `page_amax_energy/<NAME>`
        // (slash form, no channel index) and per-channel pages at
        // `page_amax_energy_4_<N>/<NAME>`. Broadcast wins; the 32 per-channel
        // duplicates must drop so codegen emits one REG_* per param.
        let mut regs = vec![
            reg_at(0x200, "page_amax_energy/POLARITY"),
            reg_at(0x203, "page_amax_energy/THRS"),
            reg_at(0x211, "page_amax_energy/baseline_delay"),
        ];
        for ch in 0..32u32 {
            regs.push(reg_at(
                0x4001 + ch * 0x200,
                &format!("page_amax_energy_4_{ch}/POLARITY"),
            ));
            regs.push(reg_at(
                0x4003 + ch * 0x200,
                &format!("page_amax_energy_4_{ch}/THRS"),
            ));
        }
        let picked = picked(&select_canonical_per_channel(&regs));
        assert_eq!(
            picked,
            vec![
                (0x200, "page_amax_energy/POLARITY".to_string()),
                (0x203, "page_amax_energy/THRS".to_string()),
                (0x211, "page_amax_energy/baseline_delay".to_string()),
            ],
            "13may era: 3 broadcast entries kept; 64 per-channel duplicates dropped"
        );
    }

    #[test]
    fn canonical_filter_2026_05_era_prefers_broadcast_drops_per_channel() {
        // 2026-05 16-ch era: broadcast page (no index) + 16 per-channel
        // pages (`_<N>_<NAME>`). Broadcast wins; per-channel duplicates
        // are dropped to avoid emitting 16 duplicate `REG_*` constants.
        let mut regs = vec![
            // The broadcast (canonical) entries.
            reg_at(0x100002, "page_amax_energy_POLARITY"),
            reg_at(0x100004, "page_amax_energy_THRS"),
            // Globals filtered out.
            reg_at(0x80008, "ENABLE_ACQ"),
        ];
        // 16 per-channel duplicates that must NOT win over the broadcast.
        for ch in 0..16u32 {
            regs.push(reg_at(
                0x200000 + ch * 0x40000,
                &format!("page_amax_energy_{ch}_POLARITY"),
            ));
            regs.push(reg_at(
                0x200004 + ch * 0x40000,
                &format!("page_amax_energy_{ch}_THRS"),
            ));
        }

        let picked = picked(&select_canonical_per_channel(&regs));
        assert_eq!(
            picked,
            vec![
                (0x100002, "page_amax_energy_POLARITY".to_string()),
                (0x100004, "page_amax_energy_THRS".to_string()),
            ],
            "2026-05 era: only the 2 broadcast entries kept; 32 per-channel duplicates dropped"
        );
    }

    #[test]
    fn canonical_filter_does_not_double_count_register_names() {
        // Cross-era invariant: after filtering, every distinct fw_key must
        // appear exactly once. If this regresses, the codegen would emit
        // duplicate `REG_*` constants and the channel-page filter would
        // be silently wrong.
        let regs = vec![
            reg_at(0x100002, "page_amax_energy_POLARITY"),
            reg_at(0x200002, "page_amax_energy_0_POLARITY"), // 2026-05 ch0 dup
            reg_at(0x300002, "page_amax_energy_1_POLARITY"), // 2026-05 ch1 dup
            reg_at(0x100004, "page_amax_energy_THRS"),
            reg_at(0x200004, "page_amax_energy_0_THRS"),
        ];
        let canonical = select_canonical_per_channel(&regs);
        let mut keys: Vec<String> = canonical.iter().filter_map(|r| r.fw_key()).collect();
        keys.sort();
        let unique = {
            let mut v = keys.clone();
            v.dedup();
            v
        };
        assert_eq!(keys, unique, "every fw_key must appear exactly once");
        assert_eq!(keys, vec!["POLARITY", "THRS"]);
    }

    #[test]
    fn canonical_filter_handles_empty_input() {
        let picked = picked(&select_canonical_per_channel(&[]));
        assert!(picked.is_empty());
    }

    #[test]
    fn canonical_filter_handles_globals_only() {
        // Boards with no `page_amax_energy_*` registers (e.g. a stripped
        // FW dump for ENABLE_ACQ probing) should yield zero per-channel
        // entries — globals are picked up by the `is_board_level` path.
        let regs = vec![reg_at(0x80008, "ENABLE_ACQ"), reg_at(0x80007, "AMAX_gol")];
        assert!(select_canonical_per_channel(&regs).is_empty());
    }
}

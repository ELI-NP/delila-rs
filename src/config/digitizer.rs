//! Digitizer configuration module
//!
//! Provides data structures for CAEN digitizer configuration.
//! Supports serialization to/from JSON for REST API and file storage.
//!
//! # Parameter Path Format
//! CAEN FELib uses path-based parameter access:
//! - `/par/<parameter>` - Board-level settings
//! - `/ch/<N>/par/<parameter>` - Per-channel settings
//! - `/ch/0..31/par/<parameter>` - Channel range (expanded by FELib)
//!
//! # Design Decision
//! All parameter values are stored as `String` rather than enums because:
//! - CAEN FELib validates values at `SetValue` time
//! - DevTree JSON provides valid choices dynamically
//! - Different firmware versions may have different valid values

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Serde helper: serialize/deserialize HashMap with numeric keys as string keys.
/// Required for BSON compatibility (MongoDB requires string document keys).
/// JSON format is unchanged (serde_json already uses string keys for numeric HashMap keys).
mod string_key_map {
    use super::*;

    pub fn serialize<K, V, S>(map: &HashMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: std::fmt::Display + Eq + std::hash::Hash,
        V: Serialize,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut ser_map = serializer.serialize_map(Some(map.len()))?;
        for (k, v) in map {
            ser_map.serialize_entry(&k.to_string(), v)?;
        }
        ser_map.end()
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<HashMap<K, V>, D::Error>
    where
        K: std::str::FromStr + Eq + std::hash::Hash,
        K::Err: std::fmt::Display,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let string_map: HashMap<String, V> = HashMap::deserialize(deserializer)?;
        string_map
            .into_iter()
            .map(|(k, v)| {
                K::from_str(&k)
                    .map(|k| (k, v))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

/// Serde helper: same as string_key_map but for Option<HashMap<...>>
mod opt_string_key_map {
    use super::*;

    pub fn serialize<K, V, S>(opt: &Option<HashMap<K, V>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: std::fmt::Display + Eq + std::hash::Hash,
        V: Serialize,
        S: Serializer,
    {
        match opt {
            Some(map) => string_key_map::serialize(map, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<Option<HashMap<K, V>>, D::Error>
    where
        K: std::str::FromStr + Eq + std::hash::Hash,
        K::Err: std::fmt::Display,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let opt: Option<HashMap<String, V>> = Option::deserialize(deserializer)?;
        match opt {
            Some(string_map) => {
                let map = string_map
                    .into_iter()
                    .map(|(k, v)| {
                        K::from_str(&k)
                            .map(|k| (k, v))
                            .map_err(serde::de::Error::custom)
                    })
                    .collect::<Result<HashMap<K, V>, _>>()?;
                Ok(Some(map))
            }
            None => Ok(None),
        }
    }
}

/// Time step in ns for PSD1/PHA1 (500 MS/s → 1 sample = 2 ns).
/// Used to convert ns config values to samples before writing to DevTree.
///
/// Digitizer configuration
///
/// Represents complete configuration for a CAEN digitizer.
/// Follows the "defaults + overrides" pattern from architecture design.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DigitizerConfig {
    /// Digitizer identifier (matches source.id in network config)
    pub digitizer_id: u32,

    /// Human-readable name
    pub name: String,

    /// Firmware type (e.g., "PSD1", "PSD2", "PHA")
    pub firmware: FirmwareType,

    /// Hardware serial number (populated by Detect command)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,

    /// Hardware model name (e.g., "VX2730", "DT5730B")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Number of channels on this digitizer
    #[serde(default = "default_num_channels")]
    pub num_channels: u8,

    /// Master digitizer flag for synchronized start
    ///
    /// In multi-digitizer setups:
    /// - Master: Receives Start command, generates TrgOut for slaves
    /// - Slave: Listens on SIN for start signal from master
    #[serde(default)]
    pub is_master: bool,

    /// Synchronization settings (optional, for Master/Slave setup)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncConfig>,

    /// Board-level parameters
    pub board: BoardConfig,

    /// Default channel settings (applied to all channels)
    #[serde(default)]
    pub channel_defaults: ChannelConfig,

    /// Per-channel overrides (sparse - only channels that differ from defaults)
    #[serde(
        default,
        skip_serializing_if = "HashMap::is_empty",
        serialize_with = "string_key_map::serialize",
        deserialize_with = "string_key_map::deserialize"
    )]
    pub channel_overrides: HashMap<u8, ChannelConfig>,

    /// Optional per-channel display names (key = channel index, value = name).
    /// Channels without entries default to "{digitizer_name}/Ch{n}".
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "opt_string_key_map::serialize",
        deserialize_with = "opt_string_key_map::deserialize"
    )]
    pub channel_names: Option<HashMap<u32, String>>,

    /// V1743-specific configuration (CAENDigitizer Library).
    /// Only used when firmware is X743CI or X743Std.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x743: Option<X743Config>,
}

/// V1743-specific configuration parameters
///
/// Controls SAM correction, sampling frequency, acquisition mode,
/// and other V1743-specific settings not shared with FELib digitizers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct X743Config {
    /// Connection type: "optical" or "usb"
    #[serde(default = "X743Config::default_link_type")]
    pub link_type: String,

    /// Link/port number (0 for first optical port)
    #[serde(default)]
    pub link_num: u32,

    /// CONET daisy chain node (0 for first/only board)
    #[serde(default)]
    pub conet_node: u32,

    /// VME base address (0 for auto)
    #[serde(default)]
    pub vme_base_address: u32,

    /// SAM sampling frequency: "3.2ghz", "1.6ghz", "800mhz", "400mhz"
    #[serde(default = "X743Config::default_sampling_frequency")]
    pub sampling_frequency: String,

    /// SAM correction level: "all", "pedestal_only", "inl", "disabled"
    #[serde(default = "X743Config::default_correction_level")]
    pub correction_level: String,

    /// Record length in samples (16-1024, step 16)
    #[serde(default = "X743Config::default_record_length")]
    pub record_length: u32,

    /// Post-trigger size per group (1-255, SAMLONG write clock units)
    #[serde(default = "X743Config::default_post_trigger_size")]
    pub post_trigger_size: u32,

    /// Max events per block transfer
    #[serde(default = "X743Config::default_max_num_events_blt")]
    pub max_num_events_blt: u32,

    /// I/O level: "nim" or "ttl"
    #[serde(default = "X743Config::default_io_level")]
    pub io_level: String,

    /// Trigger source: "software", "external", "self"
    #[serde(default = "X743Config::default_trigger_source")]
    pub trigger_source: String,

    /// Group enable mask (0xFF = all 8 groups enabled)
    #[serde(default = "X743Config::default_group_enable_mask")]
    pub group_enable_mask: u32,

    /// Enable test pulse generator
    #[serde(default)]
    pub pulse_gen_enabled: bool,

    /// Pulse generator pattern (16-bit)
    #[serde(default = "X743Config::default_pulse_pattern")]
    pub pulse_pattern: u16,

    /// Pulse generator source: "software" or "continuous"
    #[serde(default = "X743Config::default_pulse_source")]
    pub pulse_source: String,

    // ---- Event decode parameters (Standard mode) ----
    /// **DEPRECATED** — the CAEN lib does not populate `PosEdgeTimeStamp`
    /// or `NegEdgeTimeStamp` in V1743 Standard mode (only TDC + waveform),
    /// so fine timing is always computed by the Rust-side software CFD.
    /// Kept for TOML backward-compatibility.
    #[serde(default = "X743Config::default_fine_time_source")]
    pub fine_time_source: String,

    /// Energy source:
    /// - "amplitude" (default) — `|peak − baseline|` from the Rust post-processor.
    /// - "charge" — CAEN-lib `Charge` field (always 0 in Standard mode; don't use).
    /// - "soft" — reserved for a future proper charge-integration implementation.
    #[serde(default = "X743Config::default_energy_source")]
    pub energy_source: String,

    /// Linear scale applied to the amplitude before clamping to u16 energy.
    #[serde(default = "X743Config::default_energy_scale")]
    pub energy_scale: f32,

    /// Linear offset applied after `energy_scale`. Final value is clamped to [0, 65535].
    #[serde(default)]
    pub energy_offset: f32,

    /// Record a copy of `DataChannel[ch]` as the waveform alongside the event.
    /// Off by default — 256 float samples × 16 ch is heavy on PC bandwidth.
    #[serde(default)]
    pub save_waveform: bool,

    /// Pre-trigger samples averaged for the baseline estimate.
    #[serde(default = "X743Config::default_baseline_samples")]
    pub baseline_samples: u32,

    /// Software-CFD delay in samples. For PMT-like pulses (< 10 ns rise time)
    /// at 3.2 GSa/s, `4` (≈1.25 ns) is a good starting point; shorten for
    /// faster pulses, lengthen for slower ones.
    #[serde(default = "X743Config::default_cfd_delay_samples")]
    pub cfd_delay_samples: u32,

    /// Software-CFD fraction. Typical range 0.2–0.5; `0.3` works well for PMTs.
    #[serde(default = "X743Config::default_cfd_fraction")]
    pub cfd_fraction: f32,

    /// TTF (Trigger and Timing Filter) smoothing — N-tap moving average applied
    /// to the raw waveform *before* baseline / software CFD computation. Mirrors
    /// WaveDemo's `TTF_SMOOTHING` option. Default `Off` keeps existing behavior.
    #[serde(default)]
    pub ttf_smoothing: TtfSmoothing,

    /// Arbitrary register writes applied at the end of `apply_config_standard`
    /// (after `wait_for_board_ready`). Mirrors WaveDemo's `WRITE_REGISTER` —
    /// escape hatch for parameters not covered by the high-level API.
    /// Order is preserved; later entries override earlier writes to the same address.
    #[serde(default)]
    pub extra_registers: Vec<RegisterWrite>,
    // DPP-CI (Charge Mode) fields were removed 2026-04-20.
    // See TODO/47_v1743_standard_mode_redesign.md — Charge Mode wire format has no TDC,
    // so V1743 is now Standard-mode only. Any legacy dpp_ci_*/pair_*/board_* keys in TOML
    // are ignored via serde's default-on-unknown behavior.
}

/// TTF (Trigger and Timing Filter) smoothing N-tap selection.
/// `Off`/`N1` are equivalent (pass-through). 2/4/8/16 mirror WaveDemo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TtfSmoothing {
    #[default]
    Off,
    N2,
    N4,
    N8,
    N16,
}

impl TtfSmoothing {
    /// Number of taps as used by the moving-average filter. 0/1 = pass-through.
    pub fn taps(self) -> usize {
        match self {
            TtfSmoothing::Off => 0,
            TtfSmoothing::N2 => 2,
            TtfSmoothing::N4 => 4,
            TtfSmoothing::N8 => 8,
            TtfSmoothing::N16 => 16,
        }
    }
}

/// Convert an **input-referred** trigger threshold in volts to the V1743
/// channel-threshold DAC code, accounting for the channel's DC offset.
///
/// The V1743 comparator operates on the post-DC-offset signal, so the
/// register threshold corresponds to a voltage at the ADC, not at the
/// input. To trigger when the **input** crosses `v_input_volts`, the
/// register threshold must be `v_input_volts + dc_offset_v` (DC offset
/// adds to the signal before digitization, per UM1935).
///
/// Inputs:
/// - `v_input_volts`: desired threshold at the input, range nominally -1.25..=+1.25 V
/// - `dc_offset_pct`: same percentage stored in `ChannelConfig::dc_offset`
///   (50% = 0 V, 0% = -1.25 V, 100% = +1.25 V).
///
/// The post-offset target is clamped to ±1.25 V before mapping to DAC,
/// so out-of-range inputs saturate at the rails (0 or 65535).
pub fn x743_threshold_v_to_dac(v_input_volts: f32, dc_offset_pct: f32) -> u32 {
    let dc_offset_v = (dc_offset_pct - 50.0) / 50.0 * 1.25;
    let v_at_adc = (v_input_volts + dc_offset_v).clamp(-1.25, 1.25);
    // WaveDemo formula: lower V → higher DAC (inverted range, -1.25 V = 65535).
    let dac = (1.25 - v_at_adc) / 2.5 * 65535.0;
    dac.round().clamp(0.0, 65535.0) as u32
}

/// Single arbitrary register write entry. Order in `Vec<RegisterWrite>` is preserved
/// and applied verbatim at the end of `apply_config_standard`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterWrite {
    /// 32-bit register address. Accepts `0x` hex string, decimal string, or integer.
    #[serde(deserialize_with = "deserialize_u32_hex_or_dec")]
    pub addr: u32,
    /// 32-bit data word. Accepts `0x` hex string, decimal string, or integer.
    #[serde(deserialize_with = "deserialize_u32_hex_or_dec")]
    pub data: u32,
    /// Optional human-readable note (logged when applied).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Accept `"0x8108"`, `"0X8108"`, `"33032"` (string forms) or a raw integer literal.
/// TOML integer literals come through as `i64`/`u64`; JSON forms typically as strings
/// (when the user wants hex). Both are normalized to `u32`.
fn deserialize_u32_hex_or_dec<'de, D>(de: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Int(i64),
        UInt(u64),
        Str(String),
    }

    match Repr::deserialize(de)? {
        Repr::Int(i) => {
            u32::try_from(i).map_err(|_| Error::custom(format!("value {i} out of u32 range")))
        }
        Repr::UInt(u) => {
            u32::try_from(u).map_err(|_| Error::custom(format!("value {u} out of u32 range")))
        }
        Repr::Str(s) => {
            let trimmed = s.trim();
            let parsed = if let Some(hex) = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
            {
                u32::from_str_radix(hex, 16)
            } else {
                trimmed.parse::<u32>()
            };
            parsed.map_err(|e| Error::custom(format!("invalid u32 literal '{s}': {e}")))
        }
    }
}

// AMax custom-firmware per-channel writable register set.
// The struct itself is auto-generated from `RegisterFile.json` + `fw_params.json`
// — see `cargo run --bin amax_codegen`. Re-exported here so existing imports
// (`crate::config::digitizer::AMaxChannelConfig`) keep working.
pub use super::amax_generated::AMaxChannelConfig;

impl X743Config {
    fn default_link_type() -> String {
        "optical".to_string()
    }
    fn default_sampling_frequency() -> String {
        "3.2ghz".to_string()
    }
    fn default_correction_level() -> String {
        "all".to_string()
    }
    fn default_record_length() -> u32 {
        256
    }
    fn default_post_trigger_size() -> u32 {
        20
    }
    fn default_max_num_events_blt() -> u32 {
        1000
    }
    fn default_io_level() -> String {
        "nim".to_string()
    }
    fn default_trigger_source() -> String {
        "external".to_string()
    }
    fn default_group_enable_mask() -> u32 {
        0xFF
    }
    fn default_pulse_pattern() -> u16 {
        0xFFFF
    }
    fn default_pulse_source() -> String {
        "continuous".to_string()
    }
    fn default_fine_time_source() -> String {
        "cfd_soft".to_string()
    }
    fn default_energy_source() -> String {
        "amplitude".to_string()
    }
    fn default_energy_scale() -> f32 {
        1.0
    }
    fn default_baseline_samples() -> u32 {
        32
    }
    fn default_cfd_delay_samples() -> u32 {
        4
    }
    fn default_cfd_fraction() -> f32 {
        0.3
    }
}

/// Synchronization configuration for Master/Slave setups
///
/// Controls TrgOut (master) and SIN (slave) behavior for synchronized start.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct SyncConfig {
    /// TrgOut source (master only)
    /// PSD2: "Run", "TestPulse", "SWcmd", "GlobalTrg", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trgout_source: Option<String>,

    /// SIN source for sync input (slave only)
    /// PSD2: "Disabled", "SIN", "GPIO", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sin_source: Option<String>,

    /// Start source override
    /// Master: "SWcmd" (software start)
    /// Slave: "SIN" (start on SIN signal)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_source: Option<String>,
}

fn default_num_channels() -> u8 {
    32
}

/// Supported firmware types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum FirmwareType {
    /// DPP-PSD firmware (legacy x725/x730)
    PSD1,
    /// DPP-PSD2 firmware (x274x series)
    PSD2,
    /// DPP-PHA1 firmware (for spectroscopy, x725/x730)
    PHA1,
    /// DPP-PHA2 firmware (trapezoidal-filter spectroscopy, x274x series)
    PHA2,
    /// DELILA AMax firmware (Trapezoidal Filter MCA, custom DPP_OPEN on VX2730)
    AMax,
    /// V1743 Charge Integration mode (CAENDigitizer Library)
    X743CI,
    /// V1743 Standard waveform mode (CAENDigitizer Library)
    X743Std,
}

impl FirmwareType {
    /// Get the URL scheme prefix for this firmware (FELib only)
    pub fn url_scheme(&self) -> &'static str {
        match self {
            FirmwareType::PSD1 => "dig1://",
            FirmwareType::PSD2 => "dig2://",
            FirmwareType::PHA1 => "dig1://",
            FirmwareType::PHA2 => "dig2://",
            FirmwareType::AMax => "dig2://",
            FirmwareType::X743CI | FirmwareType::X743Std => {
                panic!("x743 does not use FELib URL scheme")
            }
        }
    }

    /// Whether the readout endpoint needs N_EVENTS configured.
    /// DIG2 (PSD2/PHA2/AMax) requires DATA + SIZE + N_EVENTS; DIG1 (PSD1/PHA1) uses DATA + SIZE only.
    pub fn includes_n_events(&self) -> bool {
        matches!(
            self,
            FirmwareType::PSD2 | FirmwareType::PHA2 | FirmwareType::AMax
        )
    }

    /// Whether this firmware uses the DIG1 (legacy FELib) protocol.
    pub fn is_dig1(&self) -> bool {
        matches!(self, FirmwareType::PSD1 | FirmwareType::PHA1)
    }

    /// Whether this firmware uses the CAENDigitizer Library (not FELib).
    pub fn is_legacy_api(&self) -> bool {
        matches!(self, FirmwareType::X743CI | FirmwareType::X743Std)
    }

    /// Whether this firmware uses group-based channel structure (2ch/group).
    pub fn is_group_based(&self) -> bool {
        matches!(self, FirmwareType::X743CI | FirmwareType::X743Std)
    }

    /// Whether this firmware uses FELib (modern API).
    pub fn is_felib(&self) -> bool {
        !self.is_legacy_api()
    }

    /// Aggregate capability descriptor for this firmware.
    ///
    /// Centralizes the per-FW capability flags that were previously scattered
    /// across `is_dig1()` / `is_x743()` / `is_felib()` / per-firmware match
    /// arms in `add_channel_params` etc. Phase 2 R-C1 will consume this to
    /// drive parameter-table generation; Phase 1 ships the descriptor without
    /// rewriting call sites yet (BC: existing helpers still work).
    pub const fn capabilities(&self) -> FirmwareCapabilities {
        match self {
            FirmwareType::PSD1 => FirmwareCapabilities {
                api: FirmwareApi::Dig1,
                num_channels: 16,
                supports_waveforms: true,
                includes_n_events: false,
                is_group_based: false,
            },
            FirmwareType::PHA1 => FirmwareCapabilities {
                api: FirmwareApi::Dig1,
                num_channels: 16,
                supports_waveforms: true,
                includes_n_events: false,
                is_group_based: false,
            },
            FirmwareType::PSD2 => FirmwareCapabilities {
                api: FirmwareApi::Dig2,
                num_channels: 32,
                supports_waveforms: true,
                includes_n_events: true,
                is_group_based: false,
            },
            FirmwareType::PHA2 => FirmwareCapabilities {
                api: FirmwareApi::Dig2,
                num_channels: 32,
                supports_waveforms: true,
                includes_n_events: true,
                is_group_based: false,
            },
            FirmwareType::AMax => FirmwareCapabilities {
                api: FirmwareApi::Dig2,
                // VX2730 with AMax FW exposes 32 channels via OpenDPP, but the
                // current FW build emits ch0/ch1 only (memory: AMax FW notes).
                num_channels: 32,
                supports_waveforms: true,
                includes_n_events: true,
                is_group_based: false,
            },
            FirmwareType::X743CI | FirmwareType::X743Std => FirmwareCapabilities {
                api: FirmwareApi::CaenDigitizer,
                num_channels: 8, // 4 groups × 2 channels
                supports_waveforms: true,
                includes_n_events: false,
                is_group_based: true,
            },
        }
    }
}

/// Which library / API surface this firmware uses to talk to the digitizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareApi {
    /// Legacy FELib endpoint protocol (DIG1: PSD1, PHA1).
    Dig1,
    /// Modern FELib endpoint protocol (DIG2: PSD2, PHA2, AMax).
    Dig2,
    /// CAENDigitizer.h C library (V1743: X743CI, X743Std).
    CaenDigitizer,
}

impl FirmwareApi {
    /// Whether this API surface uses FELib (DIG1 or DIG2).
    pub const fn is_felib(self) -> bool {
        matches!(self, Self::Dig1 | Self::Dig2)
    }
}

/// Capability summary for a `FirmwareType`. Constructed via
/// [`FirmwareType::capabilities`].
///
/// Lifted out of scattered `is_*()` helper calls so per-FW differences are
/// readable in one place. Phase 2 R-C1 (parameter-table refactor) will consume
/// this; Phase 1 just ships the descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirmwareCapabilities {
    /// Which API surface this firmware uses (DIG1 / DIG2 / CAENDigitizer).
    pub api: FirmwareApi,
    /// Total channel count exposed by the FW. For group-based devices
    /// (V1743) this counts the per-channel "logical" channels, not groups.
    pub num_channels: u8,
    /// Whether the FW supports per-event waveform recording.
    pub supports_waveforms: bool,
    /// Whether the FELib readout endpoint requires `N_EVENTS` to be
    /// configured (DIG2-only — DIG1 uses DATA + SIZE alone).
    pub includes_n_events: bool,
    /// Whether channels are grouped at the hardware level (V1743: 2ch / group).
    pub is_group_based: bool,
}

/// Fine Timestamp calculation mode (DIG1 only: PSD1/PHA1)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FineTsMode {
    /// Use FPGA-computed 10-bit Fine TS (Extra option 0b010). Default.
    #[default]
    Hardware,
    /// Use raw zero-crossing samples (SAZC/SBZC) for software Fine TS (Extra option 0b101).
    /// Eliminates FPGA integer rounding errors but loses 6-bit event flags.
    Software,
}

/// Board-level configuration parameters
///
/// All values are strings to match CAEN FELib's parameter format.
/// Validation is done by FELib at SetValue time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct BoardConfig {
    /// Start trigger source (e.g., "SWcmd", "ITLA")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_source: Option<String>,

    /// GPIO mode (e.g., "Run")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpio_mode: Option<String>,

    /// Test pulse period in nanoseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_pulse_period: Option<u32>,

    /// Test pulse width in nanoseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_pulse_width: Option<u32>,

    /// Test pulse low level (DAC counts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_pulse_low_level: Option<u32>,

    /// Test pulse high level (DAC counts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_pulse_high_level: Option<u32>,

    /// Global trigger source (e.g., "SwTrg", "TestPulse", "ITLA")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_trigger_source: Option<String>,

    /// Record length in samples (PSD1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_length: Option<u32>,

    /// Enable waveform readout
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waveforms_enabled: Option<bool>,

    // ---- Virtual Probes (PSD1/PHA1 only) ----
    /// Analog Probe 1 (PSD1: "VPROBE_INPUT", "VPROBE_CFD")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vtrace_probe_0: Option<String>,

    /// Analog Probe 2 (PSD1: "VPROBE_NONE", "VPROBE_BASELINE", "VPROBE_CFD")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vtrace_probe_1: Option<String>,

    /// Digital Probe 1 (PSD1: "VPROBE_GATE", "VPROBE_OVERTHRESHOLD", etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vtrace_probe_2: Option<String>,

    /// Digital Probe 2 (PSD1: "VPROBE_GATESHORT", "VPROBE_OVERTHRESHOLD", etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vtrace_probe_3: Option<String>,

    // ---- PSD1/PHA1 Trigger Configuration ----
    /// External trigger enable (PSD1/PHA1: "FALSE", "TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext_trigger_enable: Option<String>,

    /// Software trigger enable (PSD1/PHA1: "FALSE", "TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sw_trigger_enable: Option<String>,

    /// I/O level (PSD1/PHA1: "FPIOTYPE_NIM", "FPIOTYPE_TTL")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_level: Option<String>,

    /// External clock enable (PSD1/PHA1: "FALSE", "TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext_clock: Option<String>,

    /// Start delay in ns (PSD1/PHA1: 0-4080, DevTree expuom=-9)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_delay: Option<u32>,

    /// Extras enable (PSD1/PHA1: "FALSE", "TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras_enabled: Option<String>,

    /// Event aggregation (PSD1/PHA1: 1-1023)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_aggregation: Option<u32>,

    /// Coincidence TrgOut window in ns (PSD1/PHA1: 0-8184, DevTree expuom=-9)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinc_trgout: Option<u32>,

    /// Fine Timestamp mode (DIG1 only: PSD1/PHA1).
    /// "hardware" = FPGA Fine TS (default), "software" = SAZC/SBZC zero-crossing samples.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fine_ts_mode: Option<FineTsMode>,

    /// Per-VGA-group input delay (PHA2 only). VX2730 analog input is organized
    /// into 16 groups of 2 channels sharing one VGA. `inputdelay` compensates
    /// inter-channel skew (cable length differences) at the hardware level —
    /// critical for coincidence timing. Vector length must be 16; `None` means
    /// use FPGA defaults. Phase 1 plumbing only — UI surfacing TBD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_input_delay: Option<Vec<u16>>,

    /// Additional board parameters as key-value pairs
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Channel configuration parameters
///
/// All fields are optional to support sparse overrides.
/// `None` means "use default" or "unchanged".
/// String values match CAEN FELib parameter format exactly.
///
/// Fields match the frontend ChannelConfig interface in types.ts.
/// `add_channel_params` maps these field names to firmware-specific DevTree parameter names.
///
/// `MergeOverride` derives [`Self::merge_from`], used by [`Self::get_channel_config`]
/// to fold per-channel overrides over the defaults. Adding a new `Option<T>` /
/// `HashMap<K, V>` field automatically participates in the merge — no manual list
/// to update (replaces the previous hand-maintained `merge_field!` macro that had
/// already silently dropped 3 overrides; see R-C2 in `TODO/52_refactor_sprint_2026-q2.md`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, delila_derive::MergeOverride)]
pub struct ChannelConfig {
    // ---- Input ----
    /// Channel enable (e.g., "True", "False", "TRUE", "FALSE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<String>,
    /// Pulse polarity — direction of the input pulse, NOT trigger edge.
    /// Used by all FW for input-stage settings (DevTree `PulsePolarity`/`ch_polarity`).
    /// X743Std: drives the software-side waveform inversion in the decoder.
    /// For X743Std, when `trigger_edge` is unset this is also used as a fallback
    /// to derive `SetTriggerPolarity` (Positive→Rising, Negative→Falling) so that
    /// existing configs that only set `polarity` keep working.
    /// Values: "Positive", "Negative", "POLARITY_POSITIVE", "POLARITY_NEGATIVE".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polarity: Option<String>,
    /// Trigger edge — discriminator edge, independent of pulse polarity.
    /// X743Std only (consumed by `SetTriggerPolarity`); other FW currently ignore it.
    /// When unset on X743Std, falls back to `polarity` (see above).
    /// Values: "Rising", "Falling".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_edge: Option<String>,
    /// DC offset as percentage (0-100%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dc_offset: Option<f32>,
    /// VGA Gain in dB (PSD2, 0-29)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vga_gain: Option<u32>,
    /// Baseline averaging mode (e.g., "Fixed", "Low", "BLINE_NSMEAN_1024")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_avg: Option<String>,
    /// Fixed baseline value in ADC counts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_baseline: Option<u32>,
    /// Record length in ns (PSD2 per-channel)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_length_ns: Option<u32>,
    /// Pre-trigger in ns (all FW). PSD1/PHA1: converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none", alias = "pre_trigger")]
    pub pre_trigger_ns: Option<u32>,
    /// Waveform downsampling factor (PSD2: "1","2","4","8")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wave_downsampling: Option<String>,
    /// Input dynamic range (PSD1: "INDYN_2_0_VPP", "INDYN_0_5_VPP")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_dynamic: Option<String>,
    /// Coarse gain (PHA1: "COARSE_GAIN_X1", "COARSE_GAIN_X4")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coarse_gain: Option<String>,

    // ---- Trigger ----
    /// Discriminator mode (PSD2: "LeadingEdge"/"CFD", PSD1: "DISCR_MODE_LED"/"DISCR_MODE_CFD")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discriminator_mode: Option<String>,
    /// Trigger threshold in ADC counts (raw DAC for X743Std: 0-65535).
    /// X743Std: prefer `trigger_threshold_v` instead — this DAC field stays
    /// for legacy / advanced use and is overridden when `trigger_threshold_v` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_threshold: Option<u32>,
    /// Trigger threshold in **input-referred volts** (X743Std only).
    /// Range: -1.25 V to +1.25 V (clamped to V1743 input dynamic range).
    /// The hardware comparator runs on the post-DC-offset signal, so the V→DAC
    /// conversion accounts for the channel's `dc_offset` automatically:
    ///   `DAC = clamp((1.25 - (V_input + dc_offset_v)) / 2.5 * 65535, 0, 65535)`
    /// — i.e. the user enters the threshold as it appears at the **input**, not at the ADC.
    /// When set, takes priority over `trigger_threshold` (DAC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_threshold_v: Option<f32>,
    /// CFD delay in ns (PSD2/PSD1). PSD1: converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfd_delay_ns: Option<u32>,
    /// CFD fraction (PSD2: "25","50","75","100")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfd_fraction: Option<String>,
    /// CFD interpolation point for Fine TS (DIG1 only: PSD1/PHA1).
    /// 0=1st sample (highest resolution), 1=2nd, 2=3rd, 3=4th (most stable).
    /// No FELib DevTree parameter — set via direct register write (0x1n3C bits[11:10]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfd_interpolation_point: Option<u8>,
    /// Trigger holdoff in ns (all FW). PSD1/PHA1: converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none", alias = "trigger_holdoff")]
    pub trigger_holdoff_ns: Option<u32>,
    /// Smoothing factor (PSD2: "1","2","4","8","16")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smoothing_factor: Option<String>,
    /// Time filter smoothing (PSD2: "Enabled","Disabled")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_filter_smoothing: Option<String>,
    /// Input smoothing (PSD1: "CFD_SMOOTH_EXP_*")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_smoothing: Option<String>,
    /// Fast discriminator smoothing (PHA1: "RCCR2_SMTH_*")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_discr_smoothing: Option<String>,
    /// Input rise time in ns (PHA1). Converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none", alias = "input_rise_time")]
    pub input_rise_time_ns: Option<u32>,
    /// Event trigger source (PSD2: "GlobalTriggerSource", "ChSelfTrigger", ...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_trigger_source: Option<String>,
    /// Wave trigger source (PSD2: "Disabled", "ChSelfTrigger", ...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wave_trigger_source: Option<String>,
    /// Self trigger enable (PSD1/PHA1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_trigger: Option<String>,
    /// Global trigger generation (PSD1/PHA1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_trigger_gen: Option<String>,
    /// Trigger output propagation (PSD1/PHA1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_out_propagate: Option<String>,

    // ---- Energy ----
    /// Energy coarse gain (PSD2: "x1","x4",..., PSD1: "CHARGESENS_*")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_coarse_gain: Option<String>,
    /// Long gate length in ns (PSD). PSD1: converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_long_ns: Option<u32>,
    /// Short gate length in ns (PSD). PSD1: converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_short_ns: Option<u32>,
    /// Pre-gate length in ns (PSD). PSD1: converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_pre_ns: Option<u32>,
    /// Charge pedestal value (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_pedestal: Option<u32>,
    /// Short charge pedestal value (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_charge_pedestal: Option<u32>,
    /// Charge smoothing (PSD2: "Enabled","Disabled")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_smoothing: Option<String>,
    /// Charge pedestal enable (PSD1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_pedestal_en: Option<String>,
    /// Trapezoid rise time in ns (PHA1). Converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none", alias = "trap_rise_time")]
    pub trap_rise_time_ns: Option<u32>,
    /// Trapezoid flat top in ns (PHA1). Converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none", alias = "trap_flat_top")]
    pub trap_flat_top_ns: Option<u32>,
    /// Trapezoid pole-zero in ns (PHA1). Converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none", alias = "trap_pole_zero")]
    pub trap_pole_zero_ns: Option<u32>,
    /// Peaking time as percentage (PHA1, 0.0-100.0%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peaking_time: Option<f64>,
    /// N samples for peak mean (PHA1: "PEAK_NSMEAN_1",...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_nsmean: Option<String>,
    /// Peak holdoff in ns (PHA1). Converted to samples at apply time.
    #[serde(skip_serializing_if = "Option::is_none", alias = "peak_holdoff")]
    pub peak_holdoff_ns: Option<u32>,
    /// Energy fine gain (PHA1, 1.0-10.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_fine_gain: Option<f32>,

    // ---- PHA2 trapezoidal-filter (energy + time) ----
    /// PHA2 time-filter rise time in ns (16-500, step 2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_filter_rise_time_ns: Option<u32>,
    /// PHA2 time-filter retrigger guard in ns (0-8000, step 8)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_filter_retrigger_guard_ns: Option<u32>,
    /// PHA2 energy-filter rise time in ns (16-13000, step 8)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_rise_time_ns: Option<u32>,
    /// PHA2 energy-filter flat-top in ns (32-3000, step 8)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_flat_top_ns: Option<u32>,
    /// PHA2 energy-filter pole-zero in ns (32-131000, step 2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_pole_zero_ns: Option<u32>,
    /// PHA2 energy-filter peaking position as % of flat-top (10-90)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_peaking_position: Option<u8>,
    /// PHA2 energy-filter peak averaging ("LowAVG"/"MediumAVG"/"HighAVG")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_peaking_avg: Option<String>,
    /// PHA2 energy-filter baseline averaging
    /// ("Fixed"/"VeryLow"/"Low"/"MediumLow"/"Medium"/"MediumHigh"/"High")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_baseline_avg: Option<String>,
    /// PHA2 energy-filter baseline guard in ns (0-8000, step 8) — setinrun
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_baseline_guard_ns: Option<u32>,
    /// PHA2 energy-filter pile-up guard in ns (0-64000, step 64) — setinrun
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_pileup_guard_ns: Option<u32>,
    /// PHA2 energy-filter fine gain (1.000-10.000)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_fine_gain: Option<f32>,
    /// PHA2 energy-filter low-frequency limitation ("On"/"Off") — setinrun
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_filter_lf_limitation: Option<String>,
    /// PHA2 per-channel S_IN function ("None"/"ResetTimestamp"). Default None;
    /// see TODO/51 for sync-strategy notes (NOT a synchronization tool).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sin_function: Option<String>,
    /// PHA2 per-channel GPI function ("None"/"ResetTimestamp"). Default None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpi_function: Option<String>,

    // ---- Coincidence ----
    /// Channel trigger mask (PSD2, hex string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ch_trigger_mask: Option<String>,
    /// Coincidence mask (PSD2: "Disabled","Ch64Trigger",...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coincidence_mask: Option<String>,
    /// Anti-coincidence mask (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anti_coincidence_mask: Option<String>,
    /// Coincidence window in ns (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coincidence_window_ns: Option<u32>,
    /// Coincidence mode (PSD1/PHA1: "TRIGGER_MODE_NORMAL",...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coincidence_mode: Option<String>,
    /// Channel veto source (PSD2: "Disabled","BoardVeto",...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ch_veto_source: Option<String>,
    /// Channel veto width in ns (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ch_veto_width_ns: Option<u32>,
    /// Event selector (PSD2: "All","PileUp","EnergySkim")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_selector: Option<String>,
    /// Pileup rejection enable (PSD1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pileup_rejection: Option<String>,

    // ---- PSD1/PHA1 Extended Coincidence ----
    /// Trigger latency mode (PSD1: "TRG_LATENCY_MODE_NONE", "_COUPLES", "_ONETOALL")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_latency: Option<String>,
    /// Coincidence mask (PSD1/PHA1: 0-15)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinc_mask: Option<u32>,
    /// Coincidence operation (PSD1/PHA1: "COINC_OPERATION_OR", "_AND", "_MAJ")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinc_operation: Option<String>,
    /// Majority level (PSD1/PHA1: 0-7)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinc_majority_level: Option<u32>,
    /// External trigger coincidence (PSD1/PHA1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinc_trgext: Option<String>,
    /// Software trigger coincidence (PSD1/PHA1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinc_trgsw: Option<String>,
    /// Pileup gap in LSB (PSD1: 0-4095)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pileup_gap: Option<u32>,
    /// Pileup counting enable (PSD1: "FALSE","TRUE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pileup_counting_en: Option<String>,
    /// Pileup flag enable (PHA1: "FALSE","TRUE") — controls bit[27] of DPP Algorithm Control
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pileup_flag_en: Option<String>,

    // ---- Waveform ----
    /// Wave saving mode (PSD2: "Always","OnRequest")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wave_saving: Option<String>,
    /// Analog probe 0 (PSD2: "ADCInput","ADCInputBaseline","CFDFilter")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analog_probe_0: Option<String>,
    /// Analog probe 1 (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analog_probe_1: Option<String>,
    /// Digital probe 0 (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digital_probe_0: Option<String>,
    /// Digital probe 1 (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digital_probe_1: Option<String>,
    /// Digital probe 2 (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digital_probe_2: Option<String>,
    /// Digital probe 3 (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digital_probe_3: Option<String>,

    /// AMax custom-firmware per-channel registers (FirmwareType::AMax only).
    /// All other firmwares leave this `None`. Mirrors the X743Config nested
    /// pattern at the board level — keeps the 24 AMax-specific knobs out of
    /// the flat `ChannelConfig` namespace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amax: Option<AMaxChannelConfig>,

    /// Additional channel parameters (for future extensibility)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

/// CAEN parameter path-value pair
#[derive(Debug, Clone)]
pub struct CaenParameter {
    pub path: String,
    pub value: String,
}

/// Error type for digitizer configuration
#[derive(Debug)]
pub enum DigitizerConfigError {
    /// IO error reading config file
    Io(std::io::Error),
    /// JSON parse error
    Json(serde_json::Error),
}

impl std::fmt::Display for DigitizerConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DigitizerConfigError::Io(e) => write!(f, "Failed to read config file: {}", e),
            DigitizerConfigError::Json(e) => write!(f, "Failed to parse JSON: {}", e),
        }
    }
}

impl std::error::Error for DigitizerConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DigitizerConfigError::Io(e) => Some(e),
            DigitizerConfigError::Json(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for DigitizerConfigError {
    fn from(err: std::io::Error) -> Self {
        DigitizerConfigError::Io(err)
    }
}

impl From<serde_json::Error> for DigitizerConfigError {
    fn from(err: serde_json::Error) -> Self {
        DigitizerConfigError::Json(err)
    }
}

/// SetInRun-capable DevTree parameters for PSD2 / AMax.
/// PSD2 and AMax share the FELib API so the set is identical.
static SET_IN_RUN_PSD2: &[&str] = &[
    // Board
    "testpulseperiod",
    "testpulsewidth",
    "syncoutmode",
    "boardvetosource",
    "boardvetopolarity",
    "boardvetowidth",
    // Channel
    "chenable",
    "chpretriggert",
    "absolutebaseline",
    "dcoffset",
    "chgain",
    "triggerthr",
    "smoothingfactor",
    "chargesmoothing",
    "timefiltersmoothing",
    "channelvetosource",
    "adcvetowidth",
    "channelstriggermask",
    "coincidencemask",
    "anticoincidencemask",
    "coincidencelengtht",
    "eventselector",
    "eventtriggersource",
    "wavetriggersource",
    "wavesaving",
    "waveanalogprobe0",
    "waveanalogprobe1",
    "wavedigitalprobe0",
    "wavedigitalprobe1",
    "wavedigitalprobe2",
    "wavedigitalprobe3",
];

/// SetInRun-capable DevTree parameters for PHA2. Differs from PSD2 by
/// dropping the PSD-specific channel params (`absolutebaseline`,
/// `smoothingfactor`, `chargesmoothing`, `timefiltersmoothing`) and adding
/// the trapezoidal-filter params (`energyfilter*`).
static SET_IN_RUN_PHA2: &[&str] = &[
    // Board (subset of PSD2)
    "testpulseperiod",
    "testpulsewidth",
    "syncoutmode",
    "boardvetosource",
    "boardvetopolarity",
    "boardvetowidth",
    // Channel — common with PSD2
    "chenable",
    "chpretriggert",
    "dcoffset",
    "chgain",
    "triggerthr",
    "channelvetosource",
    "adcvetowidth",
    "channelstriggermask",
    "coincidencemask",
    "anticoincidencemask",
    "coincidencelengtht",
    "eventselector",
    "eventtriggersource",
    "wavetriggersource",
    "wavesaving",
    "waveanalogprobe0",
    "waveanalogprobe1",
    "wavedigitalprobe0",
    "wavedigitalprobe1",
    "wavedigitalprobe2",
    "wavedigitalprobe3",
    // PHA2-specific (trapezoidal filter)
    "energyfilterbaselineguardt",
    "energyfilterpileupguardt",
    "energyfilterlflimitation",
];

/// SetInRun-capable parameters for PSD1 (DIG1 RAW endpoint).
static SET_IN_RUN_PSD1: &[&str] = &[
    // Board
    "dt_ext_clock",
    "start_delay",
    "coinc_trgout",
    "iolevel",
    // Channel
    "ch_enabled",
    "ch_polarity",
    "ch_dcoffset",
    "ch_indyn",
    "ch_bline_fixed",
    "ch_discr_mode",
    "ch_threshold",
    "ch_cfd_delay",
    "ch_cfd_fraction",
    "ch_trg_holdoff",
    "ch_self_trg_enable",
    "ch_trg_global_gen",
    "ch_out_propagate",
    "ch_energy_cgain",
    "ch_gate",
    "ch_gateshort",
    "ch_veto_src",
    "ch_pur_en",
    // Extended Coincidence
    "ch_trg_latency",
    "ch_coinc_mask",
    "ch_coinc_operation",
    "ch_coinc_majlev",
    "ch_coinc_trgext",
    "ch_coinc_trgsw",
    "ch_purgap",
    "ch_pu_count_en",
];

/// SetInRun-capable parameters for PHA1 (DIG1 RAW endpoint).
/// Differs from PSD1 by dropping PSD-specific gates and adding
/// trapezoid-filter params.
static SET_IN_RUN_PHA1: &[&str] = &[
    // Board
    "dt_ext_clock",
    "start_delay",
    "coinc_trgout",
    "iolevel",
    // Channel
    "ch_enabled",
    "ch_polarity",
    "ch_dcoffset",
    "ch_cgain",
    "ch_threshold",
    "ch_trg_holdoff",
    "ch_self_trg_enable",
    "ch_trg_global_gen",
    "ch_out_propagate",
    "ch_trap_ftd",
    "ch_fgain",
    "ch_veto_src",
    // Extended Coincidence
    "ch_trg_latency",
    "ch_coinc_mask",
    "ch_coinc_operation",
    "ch_coinc_majlev",
    "ch_coinc_trgext",
    "ch_coinc_trgsw",
    "ch_pu_flag_en",
];

impl DigitizerConfig {
    /// Load digitizer configuration from a JSON file
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, DigitizerConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Save digitizer configuration to a JSON file
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), DigitizerConfigError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Override start source to software trigger.
    /// Used in Tune Up mode where external trigger signals are unavailable.
    pub fn force_software_trigger(&mut self) {
        let sw_value = match self.firmware {
            FirmwareType::PSD1 | FirmwareType::PHA1 => "START_MODE_SW",
            FirmwareType::PSD2 | FirmwareType::PHA2 | FirmwareType::AMax => "SWcmd",
            FirmwareType::X743CI | FirmwareType::X743Std => return, // x743 uses SW_CONTROLLED via CAENDigitizer API
        };
        self.board.start_source = Some(sw_value.to_string());
        // SyncConfig.start_source takes priority over BoardConfig in to_caen_parameters(),
        // so we must override both to ensure software trigger.
        if let Some(ref mut sync) = self.sync {
            sync.start_source = Some(sw_value.to_string());
        }
    }

    /// Sanitize configuration by removing fields that don't apply to this firmware.
    ///
    /// This should be called before saving to ensure the config file doesn't contain
    /// invalid fields (e.g., `global_trigger_source` for PSD1/PHA1).
    pub fn sanitize_for_firmware(&mut self) {
        if self.firmware.is_dig1() {
            // PSD1/PHA1: Remove PSD2-only board fields
            self.board.global_trigger_source = None;

            // Remove PSD2-only channel fields from defaults
            self.channel_defaults.event_trigger_source = None;
            self.channel_defaults.wave_trigger_source = None;
            self.channel_defaults.wave_saving = None;
            self.channel_defaults.ch_trigger_mask = None;
            self.channel_defaults.anti_coincidence_mask = None;
            self.channel_defaults.coincidence_window_ns = None;
            self.channel_defaults.ch_veto_width_ns = None;
            self.channel_defaults.event_selector = None;

            // Remove from all channel overrides
            for (_, ch_config) in self.channel_overrides.iter_mut() {
                ch_config.event_trigger_source = None;
                ch_config.wave_trigger_source = None;
                ch_config.wave_saving = None;
                ch_config.ch_trigger_mask = None;
                ch_config.anti_coincidence_mask = None;
                ch_config.coincidence_window_ns = None;
                ch_config.ch_veto_width_ns = None;
                ch_config.event_selector = None;
            }
        }
    }

    /// Create a new digitizer config with defaults for the given firmware
    pub fn new(digitizer_id: u32, name: impl Into<String>, firmware: FirmwareType) -> Self {
        let num_channels = match firmware {
            FirmwareType::PSD1 => 8,
            FirmwareType::PSD2 | FirmwareType::PHA1 | FirmwareType::PHA2 | FirmwareType::AMax => 32,
            FirmwareType::X743CI | FirmwareType::X743Std => 16, // 8 groups × 2 ch/group
        };

        Self {
            digitizer_id,
            name: name.into(),
            firmware,
            serial_number: None,
            model: None,
            num_channels,
            is_master: false,
            sync: None,
            board: BoardConfig::default(),
            channel_defaults: ChannelConfig::default(),
            channel_overrides: HashMap::new(),
            channel_names: None,
            x743: None,
        }
    }

    /// Create a master digitizer config
    pub fn new_master(digitizer_id: u32, name: impl Into<String>, firmware: FirmwareType) -> Self {
        let mut config = Self::new(digitizer_id, name, firmware);
        config.is_master = true;
        config.sync = Some(SyncConfig {
            trgout_source: Some("Run".to_string()),
            sin_source: None,
            start_source: Some("SWcmd".to_string()),
        });
        config
    }

    /// Create a slave digitizer config
    pub fn new_slave(digitizer_id: u32, name: impl Into<String>, firmware: FirmwareType) -> Self {
        let mut config = Self::new(digitizer_id, name, firmware);
        config.is_master = false;
        config.sync = Some(SyncConfig {
            trgout_source: None,
            sin_source: Some("SIN".to_string()),
            start_source: Some("SIN".to_string()),
        });
        config
    }

    /// Get effective channel configuration (defaults merged with overrides).
    ///
    /// Merge logic is generated by `#[derive(MergeOverride)]` on
    /// [`ChannelConfig`] — every `Option<T>` field with a `Some` override wins,
    /// every `HashMap<K, V>` override entry is folded in. New fields added to
    /// the struct participate automatically.
    pub fn get_channel_config(&self, channel: u8) -> ChannelConfig {
        let mut config = self.channel_defaults.clone();
        if let Some(ov) = self.channel_overrides.get(&channel) {
            config.merge_from(ov);
        }
        config
    }

    /// Generate CAEN parameter path-value pairs for hardware configuration
    ///
    /// Returns parameters in the order they should be applied:
    /// 1. Board-level parameters
    /// 2. Channel defaults (using range syntax)
    /// 3. Channel-specific overrides
    pub fn to_caen_parameters(&self) -> Vec<CaenParameter> {
        let mut params = Vec::new();

        // Board parameters
        self.add_board_parameters(&mut params);

        // Channel defaults using range syntax
        self.add_channel_defaults(&mut params);

        // Channel-specific overrides
        self.add_channel_overrides(&mut params);

        params
    }

    /// Generate only SetInRun CAEN parameters (safe to apply while Running)
    ///
    /// Filters the full parameter list to only include parameters that
    /// the hardware supports changing during acquisition.
    pub fn to_caen_parameters_set_in_run(&self) -> Vec<CaenParameter> {
        let all_params = self.to_caen_parameters();
        let set_in_run = self.set_in_run_param_names();
        all_params
            .into_iter()
            .filter(|p| {
                // Extract the parameter name from the path (last segment after '/')
                let param_name = p.path.rsplit('/').next().unwrap_or("");
                set_in_run.contains(&param_name.to_lowercase().as_str())
            })
            .collect()
    }

    /// Get the set of DevTree parameter names (lowercase) that support SetInRun.
    ///
    /// Each firmware's tables live as module-level `&'static [&'static str]`
    /// slices below so the per-firmware sets can be diffed by `grep` rather
    /// than reading match arms. PSD2 and AMax share the same FELib API so
    /// they share a slice; PHA2 / PSD1 / PHA1 each have their own.
    fn set_in_run_param_names(&self) -> std::collections::HashSet<&'static str> {
        let names: &[&str] = match self.firmware {
            FirmwareType::PSD2 | FirmwareType::AMax => SET_IN_RUN_PSD2,
            FirmwareType::PHA2 => SET_IN_RUN_PHA2,
            FirmwareType::PSD1 => SET_IN_RUN_PSD1,
            FirmwareType::PHA1 => SET_IN_RUN_PHA1,
            // x743 does not use FELib DevTree parameters
            FirmwareType::X743CI | FirmwareType::X743Std => &[],
        };
        names.iter().copied().collect()
    }

    fn add_board_parameters(&self, params: &mut Vec<CaenParameter>) {
        let board = &self.board;

        // DIG1 (PSD1/PHA1) uses different parameter paths than DIG2 (PSD2)
        let is_dig1 = matches!(self.firmware, FirmwareType::PSD1 | FirmwareType::PHA1);
        let start_path = if is_dig1 {
            "/par/startmode"
        } else {
            "/par/startsource"
        };
        let gpio_path = if is_dig1 {
            "/par/out_selection"
        } else {
            "/par/gpiomode"
        };

        // Sync parameters (applied before other board params)
        if let Some(ref sync) = self.sync {
            // Start source (from sync config takes priority)
            if let Some(ref v) = sync.start_source {
                params.push(CaenParameter {
                    path: start_path.to_string(),
                    value: v.clone(),
                });
            }

            // TrgOut source (master only) - PSD2 only
            if !is_dig1 {
                if let Some(ref v) = sync.trgout_source {
                    params.push(CaenParameter {
                        path: "/par/trgoutsource".to_string(),
                        value: v.clone(),
                    });
                }
            }

            // SIN source (slave only) - PSD2 only
            if !is_dig1 {
                if let Some(ref v) = sync.sin_source {
                    params.push(CaenParameter {
                        path: "/par/sinsource".to_string(),
                        value: v.clone(),
                    });
                }
            }
        }

        // Board start source (if not set by sync config)
        if self
            .sync
            .as_ref()
            .and_then(|s| s.start_source.as_ref())
            .is_none()
        {
            if let Some(ref v) = board.start_source {
                params.push(CaenParameter {
                    path: start_path.to_string(),
                    value: v.clone(),
                });
            }
        }

        if let Some(ref v) = board.gpio_mode {
            params.push(CaenParameter {
                path: gpio_path.to_string(),
                value: v.clone(),
            });
        }

        if let Some(v) = board.test_pulse_period {
            params.push(CaenParameter {
                path: "/par/testpulseperiod".to_string(),
                value: v.to_string(),
            });
        }

        if let Some(v) = board.test_pulse_width {
            params.push(CaenParameter {
                path: "/par/testpulsewidth".to_string(),
                value: v.to_string(),
            });
        }

        if let Some(v) = board.test_pulse_low_level {
            params.push(CaenParameter {
                path: "/par/testpulselowlevel".to_string(),
                value: v.to_string(),
            });
        }

        if let Some(v) = board.test_pulse_high_level {
            params.push(CaenParameter {
                path: "/par/testpulsehighlevel".to_string(),
                value: v.to_string(),
            });
        }

        // Global trigger source - PSD2 only (does not exist for PSD1/PHA1)
        if !is_dig1 {
            if let Some(ref v) = board.global_trigger_source {
                params.push(CaenParameter {
                    path: "/par/globaltriggersource".to_string(),
                    value: v.clone(),
                });
            }
        }

        // Record length: PSD1/PHA1 = board-level (ns), PSD2 = per-channel (ns)
        // DevTree expuom: -9 indicates ns unit for all firmware types
        if let Some(v) = board.record_length {
            match self.firmware {
                FirmwareType::PSD1 | FirmwareType::PHA1 => {
                    params.push(CaenParameter {
                        path: "/par/reclen".to_string(),
                        value: v.to_string(),
                    });
                }
                _ => {
                    // PSD2/AMax: per-channel parameter
                    params.push(CaenParameter {
                        path: format!("/ch/0..{}/par/chrecordlengths", self.num_channels - 1),
                        value: v.to_string(),
                    });
                }
            }
        }

        // Waveform enable: PSD1/PHA1 only (PSD2 uses per-channel WaveTriggerSource)
        if let Some(v) = board.waveforms_enabled {
            if matches!(self.firmware, FirmwareType::PSD1 | FirmwareType::PHA1) {
                params.push(CaenParameter {
                    path: "/par/waveforms".to_string(),
                    value: if v { "TRUE" } else { "FALSE" }.to_string(),
                });
            }
            // PSD2: waveform is controlled by channel-level wave_trigger_source
        }

        // Virtual Probes (VTrace): PSD1/PHA1 only
        // These control which signals are recorded in the waveform data
        if matches!(self.firmware, FirmwareType::PSD1 | FirmwareType::PHA1) {
            if let Some(ref v) = board.vtrace_probe_0 {
                params.push(CaenParameter {
                    path: "/vtrace/0/par/vtrace_probe".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(ref v) = board.vtrace_probe_1 {
                params.push(CaenParameter {
                    path: "/vtrace/1/par/vtrace_probe".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(ref v) = board.vtrace_probe_2 {
                params.push(CaenParameter {
                    path: "/vtrace/2/par/vtrace_probe".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(ref v) = board.vtrace_probe_3 {
                params.push(CaenParameter {
                    path: "/vtrace/3/par/vtrace_probe".to_string(),
                    value: v.clone(),
                });
            }
        }

        // PSD1/PHA1-specific board parameters
        if is_dig1 {
            if let Some(ref v) = board.ext_trigger_enable {
                params.push(CaenParameter {
                    path: "/par/trg_ext_enable".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(ref v) = board.sw_trigger_enable {
                params.push(CaenParameter {
                    path: "/par/trg_sw_enable".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(ref v) = board.io_level {
                params.push(CaenParameter {
                    path: "/par/iolevel".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(ref v) = board.ext_clock {
                params.push(CaenParameter {
                    path: "/par/dt_ext_clock".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(v) = board.start_delay {
                params.push(CaenParameter {
                    path: "/par/start_delay".to_string(),
                    value: v.to_string(),
                });
            }
            if let Some(ref v) = board.extras_enabled {
                params.push(CaenParameter {
                    path: "/par/extras".to_string(),
                    value: v.clone(),
                });
            }
            if let Some(v) = board.event_aggregation {
                params.push(CaenParameter {
                    path: "/par/eventaggr".to_string(),
                    value: v.to_string(),
                });
            }
            if let Some(v) = board.coinc_trgout {
                params.push(CaenParameter {
                    path: "/par/coinc_trgout".to_string(),
                    value: v.to_string(),
                });
            }
        }

        // PHA2-specific: per-VGA-group input delay (16 groups of 2 ch).
        // VX2730 channels share a VGA per pair; this compensates inter-channel
        // skew at the hardware level. Required path: /group/N/par/inputdelay.
        if matches!(self.firmware, FirmwareType::PHA2) {
            if let Some(ref delays) = board.group_input_delay {
                for (group_idx, &delay) in delays.iter().enumerate() {
                    params.push(CaenParameter {
                        path: format!("/group/{}/par/inputdelay", group_idx),
                        value: delay.to_string(),
                    });
                }
            }
        }

        // Extra parameters
        for (key, value) in &board.extra {
            let path = if key.starts_with('/') {
                key.clone()
            } else {
                format!("/par/{}", key)
            };
            params.push(CaenParameter {
                path,
                value: json_value_to_string(value),
            });
        }
    }

    fn add_channel_defaults(&self, params: &mut Vec<CaenParameter>) {
        let defaults = &self.channel_defaults;
        let ch_range = format!("/ch/0..{}/par", self.num_channels - 1);

        self.add_channel_params(params, &ch_range, defaults);
    }

    fn add_channel_overrides(&self, params: &mut Vec<CaenParameter>) {
        for (ch, config) in &self.channel_overrides {
            let ch_path = format!("/ch/{}/par", ch);
            self.add_channel_params(params, &ch_path, config);
        }
    }

    fn add_channel_params(
        &self,
        params: &mut Vec<CaenParameter>,
        ch_path: &str,
        config: &ChannelConfig,
    ) {
        // Per-FW tables of (devtree_path, accessor) live in
        // `channel_param_tables` (R-C1, 2026-05-06). Each accessor returns
        // `Option<String>` so the loop only emits a CAEN parameter when the
        // field is populated. PSD1/PHA1 polarity mapping is folded into the
        // accessor closure rather than living in the loop.
        use crate::config::channel_param_tables::{
            ChannelParamEntry, PHA1_PARAMS, PHA2_PARAMS, PSD1_PARAMS, PSD2_AMAX_PARAMS,
        };

        let table: &[ChannelParamEntry] = match self.firmware {
            FirmwareType::PSD2 | FirmwareType::AMax => PSD2_AMAX_PARAMS,
            FirmwareType::PHA2 => PHA2_PARAMS,
            FirmwareType::PSD1 => PSD1_PARAMS,
            FirmwareType::PHA1 => PHA1_PARAMS,
            // x743 doesn't use FELib DevTree channel parameters.
            FirmwareType::X743CI | FirmwareType::X743Std => &[],
        };

        for (devtree_name, accessor) in table {
            if let Some(value) = accessor(config) {
                params.push(CaenParameter {
                    path: format!("{}/{}", ch_path, devtree_name),
                    value,
                });
            }
        }

        // Extra parameters (for any remaining/future params)
        for (key, value) in &config.extra {
            let path = if key.starts_with('/') {
                key.clone()
            } else {
                format!("{}/{}", ch_path, key)
            };
            params.push(CaenParameter {
                path,
                value: json_value_to_string(value),
            });
        }
    }
}


/// Convert serde_json::Value to string for CAEN parameter
fn json_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_digitizer_config() {
        let config = DigitizerConfig::new(0, "Test Digitizer", FirmwareType::PSD2);
        assert_eq!(config.digitizer_id, 0);
        assert_eq!(config.name, "Test Digitizer");
        assert_eq!(config.firmware, FirmwareType::PSD2);
        assert_eq!(config.num_channels, 32);
    }

    /// PSD2 and AMax share the FELib API; their SetInRun-capable param sets
    /// must stay identical after the R-C5 base+diff refactor (Phase 1).
    /// Hand-rolled diff in match arms used to be the only way to tell — now
    /// they reference the same `SET_IN_RUN_PSD2` slice, so this test is a
    /// regression guard against accidentally splitting them again.
    #[test]
    fn set_in_run_psd2_and_amax_are_identical() {
        let psd2 = DigitizerConfig::new(0, "PSD2", FirmwareType::PSD2).set_in_run_param_names();
        let amax = DigitizerConfig::new(0, "AMax", FirmwareType::AMax).set_in_run_param_names();
        assert_eq!(psd2, amax);
    }

    #[test]
    fn set_in_run_x743_is_empty() {
        let std = DigitizerConfig::new(0, "X743", FirmwareType::X743Std).set_in_run_param_names();
        assert!(std.is_empty());
    }

    /// `FirmwareType::capabilities()` is the new central source of truth. It
    /// must agree with the legacy `is_*()` helper methods for every FW —
    /// otherwise call sites that switch from `fw.is_dig1()` to
    /// `fw.capabilities().api` would drift.
    #[test]
    fn capabilities_match_legacy_predicates_for_all_firmware() {
        let all = [
            FirmwareType::PSD1,
            FirmwareType::PSD2,
            FirmwareType::PHA1,
            FirmwareType::PHA2,
            FirmwareType::AMax,
            FirmwareType::X743CI,
            FirmwareType::X743Std,
        ];
        for fw in all {
            let cap = fw.capabilities();
            assert_eq!(
                fw.is_dig1(),
                cap.api == FirmwareApi::Dig1,
                "is_dig1 disagreed with capabilities for {fw:?}",
            );
            assert_eq!(
                fw.is_legacy_api(),
                cap.api == FirmwareApi::CaenDigitizer,
                "is_legacy_api disagreed with capabilities for {fw:?}",
            );
            assert_eq!(
                fw.is_felib(),
                cap.api.is_felib(),
                "is_felib disagreed with capabilities for {fw:?}",
            );
            assert_eq!(
                fw.is_group_based(),
                cap.is_group_based,
                "is_group_based disagreed with capabilities for {fw:?}",
            );
            assert_eq!(
                fw.includes_n_events(),
                cap.includes_n_events,
                "includes_n_events disagreed with capabilities for {fw:?}",
            );
        }
    }

    #[test]
    fn set_in_run_pha2_includes_energy_filter_params() {
        let pha2 = DigitizerConfig::new(0, "PHA2", FirmwareType::PHA2).set_in_run_param_names();
        assert!(pha2.contains("energyfilterbaselineguardt"));
        assert!(pha2.contains("energyfilterpileupguardt"));
        assert!(pha2.contains("energyfilterlflimitation"));
    }

    #[test]
    fn test_psd1_has_8_channels() {
        let config = DigitizerConfig::new(0, "PSD1", FirmwareType::PSD1);
        assert_eq!(config.num_channels, 8);
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut config = DigitizerConfig::new(1, "Digitizer 1", FirmwareType::PSD2);
        config.board.start_source = Some("SWcmd".to_string());
        config.channel_defaults.enabled = Some("True".to_string());
        config.channel_defaults.dc_offset = Some(20.0);
        config.channel_defaults.polarity = Some("Negative".to_string());
        config.channel_defaults.trigger_threshold = Some(500);

        // Add override for channel 0
        let ch0_override = ChannelConfig {
            trigger_threshold: Some(1000),
            ..Default::default()
        };
        config.channel_overrides.insert(0, ch0_override);

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&config).unwrap();
        println!("{}", json);

        // Deserialize back
        let restored: DigitizerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.digitizer_id, 1);
        assert_eq!(restored.board.start_source, Some("SWcmd".to_string()));
        assert_eq!(restored.channel_defaults.trigger_threshold, Some(500));
        assert_eq!(
            restored
                .channel_overrides
                .get(&0)
                .unwrap()
                .trigger_threshold,
            Some(1000)
        );
    }

    #[test]
    fn test_get_channel_config_with_override() {
        let mut config = DigitizerConfig::new(0, "Test", FirmwareType::PSD2);
        config.channel_defaults.enabled = Some("True".to_string());
        config.channel_defaults.dc_offset = Some(20.0);
        config.channel_defaults.trigger_threshold = Some(500);

        // Override only trigger threshold for channel 0
        let override_config = ChannelConfig {
            trigger_threshold: Some(1000),
            ..Default::default()
        };
        config.channel_overrides.insert(0, override_config);

        // Channel 0 should have overridden threshold but default offset
        let ch0 = config.get_channel_config(0);
        assert_eq!(ch0.enabled, Some("True".to_string()));
        assert_eq!(ch0.dc_offset, Some(20.0));
        assert_eq!(ch0.trigger_threshold, Some(1000)); // Overridden

        // Channel 1 should have all defaults
        let ch1 = config.get_channel_config(1);
        assert_eq!(ch1.trigger_threshold, Some(500)); // Default
    }

    /// Regression test for R-C2: the hand-maintained `merge_field!` listing in
    /// `get_channel_config` had silently dropped overrides for `amax`,
    /// `trigger_edge`, and `trigger_threshold_v` because nobody added the
    /// matching `merge_field!(...)` line when those fields were introduced.
    /// `#[derive(MergeOverride)]` now folds every `Option<T>` field
    /// automatically — this test pins that behavior so the bug class can't
    /// reappear.
    #[test]
    fn test_get_channel_config_merges_previously_dropped_fields() {
        let mut config = DigitizerConfig::new(0, "Test", FirmwareType::X743Std);
        config.channel_defaults.trigger_edge = Some("Rising".to_string());
        config.channel_defaults.trigger_threshold_v = Some(0.5);

        config.channel_defaults.amax = Some(AMaxChannelConfig {
            thrs: Some(100),
            ..Default::default()
        });

        // Override every field that the old `merge_field!` listing was missing.
        let amax_override = AMaxChannelConfig {
            thrs: Some(999),
            ..Default::default()
        };
        let override_ch = ChannelConfig {
            trigger_edge: Some("Falling".to_string()),
            trigger_threshold_v: Some(-0.25),
            amax: Some(amax_override),
            ..Default::default()
        };
        config.channel_overrides.insert(0, override_ch);

        let ch0 = config.get_channel_config(0);

        // Each of the three overrides used to be silently ignored before R-C2.
        assert_eq!(ch0.trigger_edge.as_deref(), Some("Falling"));
        assert_eq!(ch0.trigger_threshold_v, Some(-0.25));
        let amax = ch0.amax.expect("amax override must propagate");
        assert_eq!(amax.thrs, Some(999));

        // Channel 1 (no override) should still see the defaults — proves the
        // derive isn't blanket-cloning the override over every channel.
        let ch1 = config.get_channel_config(1);
        assert_eq!(ch1.trigger_edge.as_deref(), Some("Rising"));
        assert_eq!(ch1.trigger_threshold_v, Some(0.5));
        assert_eq!(ch1.amax.and_then(|a| a.thrs), Some(100));
    }

    #[test]
    fn test_get_channel_config_extends_extra_hashmap() {
        let mut config = DigitizerConfig::new(0, "Test", FirmwareType::PSD2);
        config
            .channel_defaults
            .extra
            .insert("default_only".to_string(), serde_json::json!(1));
        config
            .channel_defaults
            .extra
            .insert("shared_key".to_string(), serde_json::json!("from_default"));

        let mut override_extra = HashMap::new();
        override_extra.insert("override_only".to_string(), serde_json::json!(true));
        override_extra.insert("shared_key".to_string(), serde_json::json!("from_override"));
        let override_ch = ChannelConfig {
            extra: override_extra,
            ..Default::default()
        };
        config.channel_overrides.insert(0, override_ch);

        let ch0 = config.get_channel_config(0);
        assert_eq!(ch0.extra.get("default_only"), Some(&serde_json::json!(1)));
        assert_eq!(
            ch0.extra.get("override_only"),
            Some(&serde_json::json!(true))
        );
        // Override values win on key collision.
        assert_eq!(
            ch0.extra.get("shared_key"),
            Some(&serde_json::json!("from_override"))
        );
    }

    #[test]
    fn test_to_caen_parameters_psd2() {
        let mut config = DigitizerConfig::new(0, "Test", FirmwareType::PSD2);
        config.board.start_source = Some("SWcmd".to_string());
        config.channel_defaults.enabled = Some("True".to_string());
        config.channel_defaults.polarity = Some("Negative".to_string());

        let params = config.to_caen_parameters();

        // Check board parameter (PSD2 uses lowercase parameter names)
        assert!(params
            .iter()
            .any(|p| p.path == "/par/startsource" && p.value == "SWcmd"));

        // Check channel default (should use range syntax)
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ChEnable" && p.value == "True"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/PulsePolarity" && p.value == "Negative"));
    }

    #[test]
    fn test_master_config_sync_params() {
        let config = DigitizerConfig::new_master(0, "Master", FirmwareType::PSD2);
        assert!(config.is_master);
        assert!(config.sync.is_some());

        let sync = config.sync.as_ref().unwrap();
        assert_eq!(sync.start_source, Some("SWcmd".to_string()));
        assert_eq!(sync.trgout_source, Some("Run".to_string()));
        assert!(sync.sin_source.is_none());

        let params = config.to_caen_parameters();

        // Master should have TrgOut set to Run
        assert!(params
            .iter()
            .any(|p| p.path == "/par/trgoutsource" && p.value == "Run"));

        // Master start source should be SWcmd
        assert!(params
            .iter()
            .any(|p| p.path == "/par/startsource" && p.value == "SWcmd"));
    }

    #[test]
    fn test_slave_config_sync_params() {
        let config = DigitizerConfig::new_slave(0, "Slave", FirmwareType::PSD2);
        assert!(!config.is_master);
        assert!(config.sync.is_some());

        let sync = config.sync.as_ref().unwrap();
        assert_eq!(sync.start_source, Some("SIN".to_string()));
        assert_eq!(sync.sin_source, Some("SIN".to_string()));
        assert!(sync.trgout_source.is_none());

        let params = config.to_caen_parameters();

        // Slave should have SIN source set
        assert!(params
            .iter()
            .any(|p| p.path == "/par/sinsource" && p.value == "SIN"));

        // Slave start source should be SIN
        assert!(params
            .iter()
            .any(|p| p.path == "/par/startsource" && p.value == "SIN"));
    }

    #[test]
    fn test_sync_config_json_roundtrip() {
        // Test that sync config can be serialized and deserialized from JSON
        let json = r#"{
            "digitizer_id": 0,
            "name": "Master Digitizer",
            "firmware": "PSD2",
            "is_master": true,
            "sync": {
                "trgout_source": "Run",
                "start_source": "SWcmd"
            },
            "board": {},
            "channel_defaults": {}
        }"#;

        let config: DigitizerConfig = serde_json::from_str(json).unwrap();
        assert!(config.is_master);
        assert!(config.sync.is_some());

        let sync = config.sync.as_ref().unwrap();
        assert_eq!(sync.trgout_source, Some("Run".to_string()));
        assert_eq!(sync.start_source, Some("SWcmd".to_string()));
        assert!(sync.sin_source.is_none());

        let params = config.to_caen_parameters();
        assert!(params.iter().any(|p| p.path == "/par/trgoutsource"));
        assert!(params
            .iter()
            .any(|p| p.path == "/par/startsource" && p.value == "SWcmd"));
    }

    #[test]
    fn test_sync_config_slave_json() {
        let json = r#"{
            "digitizer_id": 1,
            "name": "Slave Digitizer",
            "firmware": "PSD2",
            "is_master": false,
            "sync": {
                "sin_source": "SIN",
                "start_source": "SIN"
            },
            "board": {},
            "channel_defaults": {}
        }"#;

        let config: DigitizerConfig = serde_json::from_str(json).unwrap();
        assert!(!config.is_master);

        let params = config.to_caen_parameters();
        assert!(params
            .iter()
            .any(|p| p.path == "/par/sinsource" && p.value == "SIN"));
        assert!(params
            .iter()
            .any(|p| p.path == "/par/startsource" && p.value == "SIN"));
        // Slave should NOT have trgout set
        assert!(!params.iter().any(|p| p.path == "/par/trgoutsource"));
    }

    #[test]
    fn test_to_caen_parameters_psd1() {
        let mut config = DigitizerConfig::new(0, "Test", FirmwareType::PSD1);
        config.channel_defaults.enabled = Some("TRUE".to_string());
        config.channel_defaults.polarity = Some("POLARITY_NEGATIVE".to_string());

        let params = config.to_caen_parameters();

        // PSD1 uses different parameter names
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_enabled" && p.value == "TRUE"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_polarity" && p.value == "POLARITY_NEGATIVE"));
    }

    #[test]
    fn test_psd1_polarity_value_mapping() {
        // PSD1 maps user-friendly polarity values to register-style enums
        let mut config = DigitizerConfig::new(0, "Test", FirmwareType::PSD1);
        config.channel_defaults.polarity = Some("Negative".to_string());

        let params = config.to_caen_parameters();
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_polarity" && p.value == "POLARITY_NEGATIVE"));

        // Also test Positive
        config.channel_defaults.polarity = Some("Positive".to_string());
        let params = config.to_caen_parameters();
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_polarity" && p.value == "POLARITY_POSITIVE"));

        // Pass-through for already register-style values
        config.channel_defaults.polarity = Some("POLARITY_NEGATIVE".to_string());
        let params = config.to_caen_parameters();
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_polarity" && p.value == "POLARITY_NEGATIVE"));
    }

    #[test]
    fn test_json_example_config() {
        // Example JSON that would come from REST API
        let json = r#"{
            "digitizer_id": 0,
            "name": "LaBr3 Digitizer",
            "firmware": "PSD2",
            "num_channels": 32,
            "board": {
                "start_source": "SWcmd",
                "gpio_mode": "Run",
                "test_pulse_period": 10000,
                "global_trigger_source": "TestPulse"
            },
            "channel_defaults": {
                "enabled": "True",
                "dc_offset": 20.0,
                "polarity": "Negative",
                "trigger_threshold": 500,
                "gate_long_ns": 400,
                "gate_short_ns": 100
            },
            "channel_overrides": {
                "0": {
                    "trigger_threshold": 300
                },
                "1": {
                    "enabled": "False"
                }
            }
        }"#;

        let config: DigitizerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "LaBr3 Digitizer");
        assert_eq!(config.firmware, FirmwareType::PSD2);
        assert_eq!(config.channel_defaults.gate_long_ns, Some(400));

        // Check that overrides work
        let ch0 = config.get_channel_config(0);
        assert_eq!(ch0.trigger_threshold, Some(300)); // Overridden
        assert_eq!(ch0.gate_long_ns, Some(400)); // From default

        let ch1 = config.get_channel_config(1);
        assert_eq!(ch1.enabled, Some("False".to_string())); // Overridden
    }

    #[test]
    fn test_psd1_ns_direct_passthrough() {
        let mut config = DigitizerConfig::new(0, "Test PSD1", FirmwareType::PSD1);
        // Set time params in ns — DevTree accepts nanoseconds directly (expuom: -9)
        config.channel_defaults.pre_trigger_ns = Some(80);
        config.channel_defaults.cfd_delay_ns = Some(20);
        config.channel_defaults.trigger_holdoff_ns = Some(1000);
        config.channel_defaults.gate_long_ns = Some(400);
        config.channel_defaults.gate_short_ns = Some(100);
        config.channel_defaults.gate_pre_ns = Some(60);
        config.board.record_length = Some(2048);

        let params = config.to_caen_parameters();

        // All time params pass through as nanoseconds (no conversion)
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_pretrg" && p.value == "80"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_cfd_delay" && p.value == "20"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_trg_holdoff" && p.value == "1000"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_gate" && p.value == "400"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_gateshort" && p.value == "100"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..7/par/ch_gatepre" && p.value == "60"));
        // Board-level record length in ns (DevTree expuom: -9)
        assert!(params
            .iter()
            .any(|p| p.path == "/par/reclen" && p.value == "2048"));
    }

    #[test]
    fn test_pha1_ns_direct_passthrough() {
        let mut config = DigitizerConfig::new(0, "Test PHA1", FirmwareType::PHA1);
        // Set time params in ns — DevTree accepts nanoseconds directly (expuom: -9)
        config.channel_defaults.pre_trigger_ns = Some(128);
        config.channel_defaults.trigger_holdoff_ns = Some(16);
        config.channel_defaults.input_rise_time_ns = Some(32);
        config.channel_defaults.trap_rise_time_ns = Some(1000);
        config.channel_defaults.trap_flat_top_ns = Some(200);
        config.channel_defaults.trap_pole_zero_ns = Some(50000);
        config.channel_defaults.peak_holdoff_ns = Some(500);
        config.board.record_length = Some(4000);

        let params = config.to_caen_parameters();

        // PHA1 has 32 channels → /ch/0..31/ range
        // All time params pass through as nanoseconds (no conversion)
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ch_pretrg" && p.value == "128"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ch_trg_holdoff" && p.value == "16"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ch_rccr2_rise" && p.value == "32"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ch_trap_trise" && p.value == "1000"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ch_trap_tflat" && p.value == "200"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ch_tdecay" && p.value == "50000"));
        assert!(params
            .iter()
            .any(|p| p.path == "/ch/0..31/par/ch_peak_holdoff" && p.value == "500"));
        // Board-level record length in ns (DevTree expuom: -9)
        assert!(params
            .iter()
            .any(|p| p.path == "/par/reclen" && p.value == "4000"));
    }

    #[test]
    fn test_psd1_serde_alias_backward_compat() {
        // Old config format with sample-based field names should still deserialize
        let json = r#"{
            "digitizer_id": 0,
            "name": "Old PSD1",
            "firmware": "PSD1",
            "num_channels": 8,
            "board": {},
            "channel_defaults": {
                "pre_trigger": 40,
                "trigger_holdoff": 500
            }
        }"#;

        let config: DigitizerConfig = serde_json::from_str(json).unwrap();
        // Old field names should map to the _ns fields via serde alias
        assert_eq!(config.channel_defaults.pre_trigger_ns, Some(40));
        assert_eq!(config.channel_defaults.trigger_holdoff_ns, Some(500));
    }

    #[test]
    fn test_psd1_board_params_use_correct_paths() {
        let mut config = DigitizerConfig::new(0, "Test PSD1", FirmwareType::PSD1);
        config.board.start_source = Some("START_MODE_SW".to_string());
        config.board.gpio_mode = Some("OUT_PROPAGATION_RUN".to_string());
        config.board.global_trigger_source = Some("ITLA".to_string()); // Should be ignored

        let params = config.to_caen_parameters();

        // PSD1 should use /par/startmode, NOT /par/startsource
        assert!(
            params
                .iter()
                .any(|p| p.path == "/par/startmode" && p.value == "START_MODE_SW"),
            "PSD1 should use /par/startmode"
        );
        assert!(
            !params.iter().any(|p| p.path == "/par/startsource"),
            "PSD1 should NOT use /par/startsource"
        );

        // PSD1 should use /par/out_selection, NOT /par/gpiomode
        assert!(
            params
                .iter()
                .any(|p| p.path == "/par/out_selection" && p.value == "OUT_PROPAGATION_RUN"),
            "PSD1 should use /par/out_selection"
        );
        assert!(
            !params.iter().any(|p| p.path == "/par/gpiomode"),
            "PSD1 should NOT use /par/gpiomode"
        );

        // PSD1 should NOT send globaltriggersource
        assert!(
            !params
                .iter()
                .any(|p| p.path.contains("globaltriggersource")),
            "PSD1 should NOT have globaltriggersource"
        );
    }

    #[test]
    fn test_psd2_board_params_use_correct_paths() {
        let mut config = DigitizerConfig::new(0, "Test PSD2", FirmwareType::PSD2);
        config.board.start_source = Some("SWcmd".to_string());
        config.board.gpio_mode = Some("Run".to_string());
        config.board.global_trigger_source = Some("ITLA".to_string());

        let params = config.to_caen_parameters();

        // PSD2 should use /par/startsource
        assert!(
            params
                .iter()
                .any(|p| p.path == "/par/startsource" && p.value == "SWcmd"),
            "PSD2 should use /par/startsource"
        );

        // PSD2 should use /par/gpiomode
        assert!(
            params
                .iter()
                .any(|p| p.path == "/par/gpiomode" && p.value == "Run"),
            "PSD2 should use /par/gpiomode"
        );

        // PSD2 should have globaltriggersource
        assert!(
            params
                .iter()
                .any(|p| p.path == "/par/globaltriggersource" && p.value == "ITLA"),
            "PSD2 should have globaltriggersource"
        );
    }

    #[test]
    fn test_channel_range_8ch_psd1() {
        let mut config = DigitizerConfig::new(0, "DT5730B", FirmwareType::PSD1);
        config.num_channels = 8;
        config.channel_defaults.enabled = Some("TRUE".to_string());
        let params = config.to_caen_parameters();
        assert!(
            params.iter().any(|p| p.path.starts_with("/ch/0..7/par/")),
            "8ch config should use /ch/0..7/ range"
        );
        assert!(
            !params.iter().any(|p| p.path.starts_with("/ch/0..15/par/")),
            "8ch config must not use /ch/0..15/ range"
        );
    }

    #[test]
    fn test_channel_range_16ch_psd1() {
        let mut config = DigitizerConfig::new(0, "VX1730B", FirmwareType::PSD1);
        config.num_channels = 16;
        config.channel_defaults.enabled = Some("TRUE".to_string());
        let params = config.to_caen_parameters();
        assert!(
            params.iter().any(|p| p.path.starts_with("/ch/0..15/par/")),
            "16ch config should use /ch/0..15/ range"
        );
    }

    #[test]
    fn test_force_software_trigger_psd2() {
        let mut config = DigitizerConfig::new(0, "Test PSD2", FirmwareType::PSD2);
        config.board.start_source = Some("SINlevel".to_string());
        config.sync = Some(SyncConfig {
            start_source: Some("SINlevel".to_string()),
            ..Default::default()
        });
        config.force_software_trigger();
        assert_eq!(config.board.start_source, Some("SWcmd".to_string()));
        assert_eq!(
            config.sync.as_ref().and_then(|s| s.start_source.as_deref()),
            Some("SWcmd")
        );
        let params = config.to_caen_parameters();
        assert!(
            params
                .iter()
                .any(|p| p.path == "/par/startsource" && p.value == "SWcmd"),
            "PSD2 should use SWcmd after force_software_trigger"
        );
    }

    #[test]
    fn test_force_software_trigger_psd1() {
        let mut config = DigitizerConfig::new(0, "Test PSD1", FirmwareType::PSD1);
        config.board.start_source = Some("START_MODE_S_IN".to_string());
        config.force_software_trigger();
        assert_eq!(config.board.start_source, Some("START_MODE_SW".to_string()));
        let params = config.to_caen_parameters();
        assert!(
            params
                .iter()
                .any(|p| p.path == "/par/startmode" && p.value == "START_MODE_SW"),
            "PSD1 should use START_MODE_SW after force_software_trigger"
        );
    }

    #[test]
    fn test_force_software_trigger_no_sync() {
        let mut config = DigitizerConfig::new(0, "Test PSD2", FirmwareType::PSD2);
        config.board.start_source = Some("SINlevel".to_string());
        config.force_software_trigger();
        assert_eq!(config.board.start_source, Some("SWcmd".to_string()));
        assert!(config.sync.is_none());
    }

    #[test]
    fn test_force_software_trigger_amax() {
        let mut config = DigitizerConfig::new(0, "Test AMax", FirmwareType::AMax);
        config.board.start_source = Some("P0".to_string());
        config.force_software_trigger();
        assert_eq!(config.board.start_source, Some("SWcmd".to_string()));
    }

    /// `deserialize_u32_hex_or_dec` accepts hex strings, decimal strings, and
    /// integer literals. All four forms below must produce 0x8108 = 33032.
    #[test]
    fn test_register_write_hex_or_dec_parsing() {
        let hex_lower =
            serde_json::from_str::<RegisterWrite>(r#"{"addr":"0x8108","data":"0x10"}"#).unwrap();
        assert_eq!(hex_lower.addr, 0x8108);
        assert_eq!(hex_lower.data, 0x10);

        let hex_upper =
            serde_json::from_str::<RegisterWrite>(r#"{"addr":"0X8108","data":"0X10"}"#).unwrap();
        assert_eq!(hex_upper.addr, 0x8108);

        let dec_str =
            serde_json::from_str::<RegisterWrite>(r#"{"addr":"33032","data":"16"}"#).unwrap();
        assert_eq!(dec_str.addr, 0x8108);
        assert_eq!(dec_str.data, 16);

        let dec_int = serde_json::from_str::<RegisterWrite>(r#"{"addr":33032,"data":16}"#).unwrap();
        assert_eq!(dec_int.addr, 0x8108);
        assert_eq!(dec_int.data, 16);

        // Junk should fail.
        let bad = serde_json::from_str::<RegisterWrite>(r#"{"addr":"xyz","data":0}"#);
        assert!(bad.is_err());
    }

    /// `trigger_edge` must round-trip through serde alongside `polarity`,
    /// without one clobbering the other.
    #[test]
    fn test_channel_config_trigger_edge_roundtrip() {
        let json = r#"{"polarity":"Negative","trigger_edge":"Rising"}"#;
        let cfg: ChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.polarity.as_deref(), Some("Negative"));
        assert_eq!(cfg.trigger_edge.as_deref(), Some("Rising"));

        // polarity-only (no trigger_edge) must still parse — backward compat.
        let json_legacy = r#"{"polarity":"Negative"}"#;
        let cfg_legacy: ChannelConfig = serde_json::from_str(json_legacy).unwrap();
        assert_eq!(cfg_legacy.polarity.as_deref(), Some("Negative"));
        assert_eq!(cfg_legacy.trigger_edge, None);
    }

    /// `TtfSmoothing` enum maps to the WaveDemo tap counts.
    #[test]
    fn test_ttf_smoothing_taps() {
        assert_eq!(TtfSmoothing::Off.taps(), 0);
        assert_eq!(TtfSmoothing::N2.taps(), 2);
        assert_eq!(TtfSmoothing::N4.taps(), 4);
        assert_eq!(TtfSmoothing::N8.taps(), 8);
        assert_eq!(TtfSmoothing::N16.taps(), 16);
        assert_eq!(TtfSmoothing::default().taps(), 0);
    }

    /// V1743 input-referred threshold → DAC, with DC offset = 50% (0 V):
    /// the WaveDemo formula passes through unchanged.
    #[test]
    fn test_x743_threshold_v_to_dac_no_offset() {
        // 50% DC offset = 0 V, so threshold V_at_adc = V_input.
        // 0 V → mid-scale (32768 ± rounding)
        assert_eq!(x743_threshold_v_to_dac(0.0, 50.0), 32768);
        // -1.25 V → 65535 (rail, fully negative)
        assert_eq!(x743_threshold_v_to_dac(-1.25, 50.0), 65535);
        // +1.25 V → 0 (rail, fully positive)
        assert_eq!(x743_threshold_v_to_dac(1.25, 50.0), 0);
        // -0.2 V → (1.25 - (-0.2)) / 2.5 * 65535 = 1.45 / 2.5 * 65535 ≈ 38010
        let dac = x743_threshold_v_to_dac(-0.2, 50.0);
        assert!((dac as i32 - 38010).abs() <= 1, "got {dac}");
    }

    /// With DC offset = 70% (= +0.5 V), input-referred V=-0.2 V means the
    /// signal hits the comparator at +0.3 V, i.e. DAC ≈ 24903.
    /// This is what the user actually wants when adjusting baseline.
    #[test]
    fn test_x743_threshold_v_to_dac_with_positive_offset() {
        // dc_offset_v = (70 - 50) / 50 * 1.25 = +0.5 V
        // V_at_adc = -0.2 + 0.5 = +0.3 V
        // DAC = (1.25 - 0.3) / 2.5 * 65535 ≈ 24903
        let dac = x743_threshold_v_to_dac(-0.2, 70.0);
        assert!((dac as i32 - 24903).abs() <= 1, "got {dac}");

        // Same input-referred 0 V at offset 90% (+1.0 V) → V_at_adc = +1.0
        // DAC = (1.25 - 1.0) / 2.5 * 65535 ≈ 6553
        let dac = x743_threshold_v_to_dac(0.0, 90.0);
        assert!((dac as i32 - 6553).abs() <= 1, "got {dac}");
    }

    /// Negative DC offset shifts the signal down; same input-referred V means
    /// the threshold register has to go further negative to compensate.
    #[test]
    fn test_x743_threshold_v_to_dac_with_negative_offset() {
        // dc_offset_v = (30 - 50) / 50 * 1.25 = -0.5 V
        // V_at_adc = -0.2 + (-0.5) = -0.7 V
        // DAC = (1.25 - (-0.7)) / 2.5 * 65535 = 1.95 / 2.5 * 65535 ≈ 51117
        let dac = x743_threshold_v_to_dac(-0.2, 30.0);
        assert!((dac as i32 - 51117).abs() <= 1, "got {dac}");
    }

    /// Out-of-range inputs saturate at the rails — never panic.
    #[test]
    fn test_x743_threshold_v_to_dac_clamping() {
        // V_input way below -1.25, any DC offset → DAC saturates at 65535
        assert_eq!(x743_threshold_v_to_dac(-5.0, 50.0), 65535);
        // V_input way above +1.25 → DAC saturates at 0
        assert_eq!(x743_threshold_v_to_dac(5.0, 50.0), 0);
        // Combined edge: V=-1.0 with DC offset = 0% (= -1.25 V) → V_at_adc = -2.25 → clamp -1.25
        assert_eq!(x743_threshold_v_to_dac(-1.0, 0.0), 65535);
    }

    /// `trigger_threshold_v` round-trips through serde alongside `trigger_threshold`.
    #[test]
    fn test_channel_config_trigger_threshold_v_roundtrip() {
        let json = r#"{"trigger_threshold":1234,"trigger_threshold_v":-0.2}"#;
        let cfg: ChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.trigger_threshold, Some(1234));
        assert_eq!(cfg.trigger_threshold_v, Some(-0.2));

        // V-only (preferred path) — backward compat with no DAC
        let json_v = r#"{"trigger_threshold_v":0.5}"#;
        let cfg_v: ChannelConfig = serde_json::from_str(json_v).unwrap();
        assert_eq!(cfg_v.trigger_threshold, None);
        assert_eq!(cfg_v.trigger_threshold_v, Some(0.5));
    }
}

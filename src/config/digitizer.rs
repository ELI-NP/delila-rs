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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_overrides: HashMap<u8, ChannelConfig>,
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
    /// DELILA AMax firmware (Trapezoidal Filter MCA, custom DPP_OPEN on VX2730)
    AMax,
}

impl FirmwareType {
    /// Get the URL scheme prefix for this firmware
    pub fn url_scheme(&self) -> &'static str {
        match self {
            FirmwareType::PSD1 => "dig1://",
            FirmwareType::PSD2 => "dig2://",
            FirmwareType::PHA1 => "dig1://", // PHA1 uses dig1 (same as PSD1)
            FirmwareType::AMax => "dig2://", // AMax uses dig2 (VX2730 with DPP_OPEN)
        }
    }

    /// Whether the readout endpoint needs N_EVENTS configured.
    /// DIG2 (PSD2, AMax) requires DATA + SIZE + N_EVENTS; DIG1 (PSD1/PHA) uses DATA + SIZE only.
    pub fn includes_n_events(&self) -> bool {
        matches!(self, FirmwareType::PSD2 | FirmwareType::AMax)
    }

    /// Whether this firmware uses the DIG1 (legacy) protocol.
    pub fn is_dig1(&self) -> bool {
        matches!(self, FirmwareType::PSD1 | FirmwareType::PHA1)
    }
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

    /// Global trigger source (e.g., "SwTrg", "TestPulse", "ITLA")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_trigger_source: Option<String>,

    /// Record length in samples (PSD1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_length: Option<u32>,

    /// Enable waveform readout
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waveforms_enabled: Option<bool>,

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
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ChannelConfig {
    // ---- Input ----
    /// Channel enable (e.g., "True", "False", "TRUE", "FALSE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<String>,
    /// Pulse polarity (e.g., "Positive", "Negative", "POLARITY_POSITIVE")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polarity: Option<String>,
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
    /// Pre-trigger in ns (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_trigger_ns: Option<u32>,
    /// Pre-trigger in samples (PSD1/PHA1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_trigger: Option<u32>,
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
    /// Trigger threshold in ADC counts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_threshold: Option<u32>,
    /// CFD delay in ns (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfd_delay_ns: Option<u32>,
    /// CFD fraction (PSD2: "25","50","75","100")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfd_fraction: Option<String>,
    /// Trigger holdoff in ns (PSD2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_holdoff_ns: Option<u32>,
    /// Trigger holdoff in samples (PSD1/PHA1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_holdoff: Option<u32>,
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
    /// Input rise time in samples (PHA1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_rise_time: Option<u32>,
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
    /// Long gate length in ns/samples (PSD)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_long_ns: Option<u32>,
    /// Short gate length in ns/samples (PSD)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_short_ns: Option<u32>,
    /// Pre-gate length in ns/samples (PSD)
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
    /// Trapezoid rise time in samples (PHA1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trap_rise_time: Option<u32>,
    /// Trapezoid flat top in samples (PHA1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trap_flat_top: Option<u32>,
    /// Trapezoid pole-zero in samples (PHA1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trap_pole_zero: Option<u32>,
    /// Peaking time as percentage (PHA1, 0.0-100.0%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peaking_time: Option<f64>,
    /// N samples for peak mean (PHA1: "PEAK_NSMEAN_1",...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_nsmean: Option<String>,
    /// Peak holdoff in samples (PHA1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_holdoff: Option<u32>,
    /// Energy fine gain (PHA1, 1.0-10.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_fine_gain: Option<f32>,

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

    /// Create a new digitizer config with defaults for the given firmware
    pub fn new(digitizer_id: u32, name: impl Into<String>, firmware: FirmwareType) -> Self {
        let num_channels = match firmware {
            FirmwareType::PSD1 => 8,
            FirmwareType::PSD2 | FirmwareType::PHA1 | FirmwareType::AMax => 32,
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

    /// Get effective channel configuration (defaults merged with overrides)
    pub fn get_channel_config(&self, channel: u8) -> ChannelConfig {
        let mut config = self.channel_defaults.clone();

        if let Some(ov) = self.channel_overrides.get(&channel) {
            // Merge override into defaults: override field wins if Some
            macro_rules! merge_field {
                ($field:ident) => {
                    if ov.$field.is_some() {
                        config.$field = ov.$field.clone();
                    }
                };
            }
            // Input
            merge_field!(enabled);
            merge_field!(polarity);
            merge_field!(dc_offset);
            merge_field!(vga_gain);
            merge_field!(baseline_avg);
            merge_field!(fixed_baseline);
            merge_field!(record_length_ns);
            merge_field!(pre_trigger_ns);
            merge_field!(pre_trigger);
            merge_field!(wave_downsampling);
            merge_field!(input_dynamic);
            merge_field!(coarse_gain);
            // Trigger
            merge_field!(discriminator_mode);
            merge_field!(trigger_threshold);
            merge_field!(cfd_delay_ns);
            merge_field!(cfd_fraction);
            merge_field!(trigger_holdoff_ns);
            merge_field!(trigger_holdoff);
            merge_field!(smoothing_factor);
            merge_field!(time_filter_smoothing);
            merge_field!(input_smoothing);
            merge_field!(fast_discr_smoothing);
            merge_field!(input_rise_time);
            merge_field!(event_trigger_source);
            merge_field!(wave_trigger_source);
            merge_field!(self_trigger);
            merge_field!(global_trigger_gen);
            merge_field!(trigger_out_propagate);
            // Energy
            merge_field!(energy_coarse_gain);
            merge_field!(gate_long_ns);
            merge_field!(gate_short_ns);
            merge_field!(gate_pre_ns);
            merge_field!(charge_pedestal);
            merge_field!(short_charge_pedestal);
            merge_field!(charge_smoothing);
            merge_field!(charge_pedestal_en);
            merge_field!(trap_rise_time);
            merge_field!(trap_flat_top);
            merge_field!(trap_pole_zero);
            merge_field!(peaking_time);
            merge_field!(peak_nsmean);
            merge_field!(peak_holdoff);
            merge_field!(energy_fine_gain);
            // Coincidence
            merge_field!(ch_trigger_mask);
            merge_field!(coincidence_mask);
            merge_field!(anti_coincidence_mask);
            merge_field!(coincidence_window_ns);
            merge_field!(coincidence_mode);
            merge_field!(ch_veto_source);
            merge_field!(ch_veto_width_ns);
            merge_field!(event_selector);
            merge_field!(pileup_rejection);
            // Waveform
            merge_field!(wave_saving);
            merge_field!(analog_probe_0);
            merge_field!(analog_probe_1);
            merge_field!(digital_probe_0);
            merge_field!(digital_probe_1);
            merge_field!(digital_probe_2);
            merge_field!(digital_probe_3);
            // Extra
            for (k, v) in &ov.extra {
                config.extra.insert(k.clone(), v.clone());
            }
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

    /// Get the set of DevTree parameter names (lowercase) that support SetInRun
    fn set_in_run_param_names(&self) -> std::collections::HashSet<&'static str> {
        use std::collections::HashSet;
        match self.firmware {
            FirmwareType::PSD2 | FirmwareType::AMax => HashSet::from([
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
            ]),
            FirmwareType::PSD1 => HashSet::from([
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
            ]),
            FirmwareType::PHA1 => HashSet::from([
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
            ]),
        }
    }

    fn add_board_parameters(&self, params: &mut Vec<CaenParameter>) {
        let board = &self.board;

        // Sync parameters (applied before other board params)
        if let Some(ref sync) = self.sync {
            // Start source (from sync config takes priority)
            if let Some(ref v) = sync.start_source {
                params.push(CaenParameter {
                    path: "/par/startsource".to_string(),
                    value: v.clone(),
                });
            }

            // TrgOut source (master only)
            if let Some(ref v) = sync.trgout_source {
                params.push(CaenParameter {
                    path: "/par/trgoutsource".to_string(),
                    value: v.clone(),
                });
            }

            // SIN source (slave only)
            if let Some(ref v) = sync.sin_source {
                params.push(CaenParameter {
                    path: "/par/sinsource".to_string(),
                    value: v.clone(),
                });
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
                    path: "/par/startsource".to_string(),
                    value: v.clone(),
                });
            }
        }

        if let Some(ref v) = board.gpio_mode {
            params.push(CaenParameter {
                path: "/par/gpiomode".to_string(),
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

        if let Some(ref v) = board.global_trigger_source {
            params.push(CaenParameter {
                path: "/par/globaltriggersource".to_string(),
                value: v.clone(),
            });
        }

        // Record length: PSD1 = board-level, PSD2 = per-channel
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

        // Waveform enable: PSD1 only (PSD2 uses per-channel WaveTriggerSource)
        if let Some(v) = board.waveforms_enabled {
            if matches!(self.firmware, FirmwareType::PSD1 | FirmwareType::PHA1) {
                params.push(CaenParameter {
                    path: "/par/waveforms".to_string(),
                    value: v.to_string().to_lowercase(),
                });
            }
            // PSD2: waveform is controlled by channel-level wave_trigger_source
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
        // Helpers: push string/numeric parameters with DevTree name
        macro_rules! push_str {
            ($devtree:expr, $value:expr) => {
                params.push(CaenParameter {
                    path: format!("{}/{}", ch_path, $devtree),
                    value: $value.to_string(),
                });
            };
        }
        macro_rules! push_num {
            ($devtree:expr, $value:expr) => {
                params.push(CaenParameter {
                    path: format!("{}/{}", ch_path, $devtree),
                    value: $value.to_string(),
                });
            };
        }

        match self.firmware {
            FirmwareType::PSD2 | FirmwareType::AMax => {
                // ---- Input ----
                if let Some(ref v) = config.enabled {
                    push_str!("ChEnable", v);
                }
                if let Some(ref v) = config.polarity {
                    push_str!("PulsePolarity", v);
                }
                if let Some(v) = config.dc_offset {
                    push_num!("DCOffset", v);
                }
                if let Some(v) = config.vga_gain {
                    push_num!("ChGain", v);
                }
                if let Some(ref v) = config.baseline_avg {
                    push_str!("ADCInputBaselineAvg", v);
                }
                if let Some(v) = config.fixed_baseline {
                    push_num!("AbsoluteBaseline", v);
                }
                if let Some(v) = config.record_length_ns {
                    push_num!("ChRecordLengthT", v);
                }
                if let Some(v) = config.pre_trigger_ns {
                    push_num!("ChPreTriggerT", v);
                }
                if let Some(ref v) = config.wave_downsampling {
                    push_str!("WaveDownSamplingFactor", v);
                }
                // ---- Trigger ----
                if let Some(ref v) = config.discriminator_mode {
                    push_str!("TriggerFilterSelection", v);
                }
                if let Some(v) = config.trigger_threshold {
                    push_num!("TriggerThr", v);
                }
                if let Some(v) = config.cfd_delay_ns {
                    push_num!("CFDDelayT", v);
                }
                if let Some(ref v) = config.cfd_fraction {
                    push_str!("CFDFraction", v);
                }
                if let Some(v) = config.trigger_holdoff_ns {
                    push_num!("TimeFilterRetriggerGuardT", v);
                }
                if let Some(ref v) = config.smoothing_factor {
                    push_str!("SmoothingFactor", v);
                }
                if let Some(ref v) = config.time_filter_smoothing {
                    push_str!("TimeFilterSmoothing", v);
                }
                if let Some(ref v) = config.event_trigger_source {
                    push_str!("EventTriggerSource", v);
                }
                if let Some(ref v) = config.wave_trigger_source {
                    push_str!("WaveTriggerSource", v);
                }
                // ---- Energy ----
                if let Some(ref v) = config.energy_coarse_gain {
                    push_str!("EnergyGain", v);
                }
                if let Some(v) = config.gate_long_ns {
                    push_num!("GateLongLengthT", v);
                }
                if let Some(v) = config.gate_short_ns {
                    push_num!("GateShortLengthT", v);
                }
                if let Some(v) = config.gate_pre_ns {
                    push_num!("GateOffsetT", v);
                }
                if let Some(v) = config.charge_pedestal {
                    push_num!("LongChargeIntegratorPedestal", v);
                }
                if let Some(v) = config.short_charge_pedestal {
                    push_num!("ShortChargeIntegratorPedestal", v);
                }
                if let Some(ref v) = config.charge_smoothing {
                    push_str!("ChargeSmoothing", v);
                }
                // ---- Coincidence ----
                if let Some(ref v) = config.ch_trigger_mask {
                    push_str!("ChannelsTriggerMask", v);
                }
                if let Some(ref v) = config.coincidence_mask {
                    push_str!("CoincidenceMask", v);
                }
                if let Some(ref v) = config.anti_coincidence_mask {
                    push_str!("AntiCoincidenceMask", v);
                }
                if let Some(v) = config.coincidence_window_ns {
                    push_num!("CoincidenceLengthT", v);
                }
                if let Some(ref v) = config.ch_veto_source {
                    push_str!("ChannelVetoSource", v);
                }
                if let Some(v) = config.ch_veto_width_ns {
                    push_num!("ADCVetoWidth", v);
                }
                if let Some(ref v) = config.event_selector {
                    push_str!("EventSelector", v);
                }
                // ---- Waveform ----
                if let Some(ref v) = config.wave_saving {
                    push_str!("WaveSaving", v);
                }
                if let Some(ref v) = config.analog_probe_0 {
                    push_str!("WaveAnalogProbe0", v);
                }
                if let Some(ref v) = config.analog_probe_1 {
                    push_str!("WaveAnalogProbe1", v);
                }
                if let Some(ref v) = config.digital_probe_0 {
                    push_str!("WaveDigitalProbe0", v);
                }
                if let Some(ref v) = config.digital_probe_1 {
                    push_str!("WaveDigitalProbe1", v);
                }
                if let Some(ref v) = config.digital_probe_2 {
                    push_str!("WaveDigitalProbe2", v);
                }
                if let Some(ref v) = config.digital_probe_3 {
                    push_str!("WaveDigitalProbe3", v);
                }
            }
            FirmwareType::PSD1 => {
                // ---- Input ----
                if let Some(ref v) = config.enabled {
                    push_str!("ch_enabled", v);
                }
                if let Some(ref v) = config.polarity {
                    // PSD1 uses register-style enums
                    let mapped = match v.to_lowercase().as_str() {
                        "negative" => "POLARITY_NEGATIVE",
                        "positive" => "POLARITY_POSITIVE",
                        _ => v.as_str(),
                    };
                    push_str!("ch_polarity", mapped);
                }
                if let Some(v) = config.dc_offset {
                    push_num!("ch_dcoffset", v);
                }
                if let Some(ref v) = config.input_dynamic {
                    push_str!("ch_indyn", v);
                }
                if let Some(ref v) = config.baseline_avg {
                    push_str!("ch_bline_nsmean", v);
                }
                if let Some(v) = config.fixed_baseline {
                    push_num!("ch_bline_fixed", v);
                }
                if let Some(v) = config.pre_trigger {
                    push_num!("ch_pretrg", v);
                }
                // ---- Trigger ----
                if let Some(ref v) = config.discriminator_mode {
                    push_str!("ch_discr_mode", v);
                }
                if let Some(v) = config.trigger_threshold {
                    push_num!("ch_threshold", v);
                }
                if let Some(v) = config.cfd_delay_ns {
                    push_num!("ch_cfd_delay", v);
                }
                if let Some(ref v) = config.cfd_fraction {
                    push_str!("ch_cfd_fraction", v);
                }
                if let Some(ref v) = config.input_smoothing {
                    push_str!("ch_cfd_smoothexp", v);
                }
                if let Some(v) = config.trigger_holdoff {
                    push_num!("ch_trg_holdoff", v);
                }
                if let Some(ref v) = config.self_trigger {
                    push_str!("ch_self_trg_enable", v);
                }
                if let Some(ref v) = config.global_trigger_gen {
                    push_str!("ch_trg_global_gen", v);
                }
                if let Some(ref v) = config.trigger_out_propagate {
                    push_str!("ch_out_propagate", v);
                }
                // ---- Energy ----
                if let Some(ref v) = config.energy_coarse_gain {
                    push_str!("ch_energy_cgain", v);
                }
                if let Some(v) = config.gate_long_ns {
                    push_num!("ch_gate", v);
                }
                if let Some(v) = config.gate_short_ns {
                    push_num!("ch_gateshort", v);
                }
                if let Some(v) = config.gate_pre_ns {
                    push_num!("ch_gatepre", v);
                }
                if let Some(ref v) = config.charge_pedestal_en {
                    push_str!("ch_pedestal_en", v);
                }
                // ---- Coincidence ----
                if let Some(ref v) = config.coincidence_mode {
                    push_str!("ch_trg_mode", v);
                }
                if let Some(ref v) = config.ch_veto_source {
                    push_str!("ch_veto_src", v);
                }
                if let Some(ref v) = config.pileup_rejection {
                    push_str!("ch_pur_en", v);
                }
            }
            FirmwareType::PHA1 => {
                // ---- Input ----
                if let Some(ref v) = config.enabled {
                    push_str!("ch_enabled", v);
                }
                if let Some(ref v) = config.polarity {
                    let mapped = match v.to_lowercase().as_str() {
                        "negative" => "POLARITY_NEGATIVE",
                        "positive" => "POLARITY_POSITIVE",
                        _ => v.as_str(),
                    };
                    push_str!("ch_polarity", mapped);
                }
                if let Some(v) = config.dc_offset {
                    push_num!("ch_dcoffset", v);
                }
                if let Some(ref v) = config.coarse_gain {
                    push_str!("ch_cgain", v);
                }
                if let Some(ref v) = config.baseline_avg {
                    push_str!("ch_bline_nsmean", v);
                }
                if let Some(v) = config.pre_trigger {
                    push_num!("ch_pretrg", v);
                }
                // ---- Trigger ----
                if let Some(v) = config.trigger_threshold {
                    push_num!("ch_threshold", v);
                }
                if let Some(v) = config.trigger_holdoff {
                    push_num!("ch_trg_holdoff", v);
                }
                if let Some(ref v) = config.fast_discr_smoothing {
                    push_str!("ch_rccr2_smooth", v);
                }
                if let Some(v) = config.input_rise_time {
                    push_num!("ch_rccr2_rise", v);
                }
                if let Some(ref v) = config.self_trigger {
                    push_str!("ch_self_trg_enable", v);
                }
                if let Some(ref v) = config.global_trigger_gen {
                    push_str!("ch_trg_global_gen", v);
                }
                if let Some(ref v) = config.trigger_out_propagate {
                    push_str!("ch_out_propagate", v);
                }
                // ---- Energy ----
                if let Some(v) = config.trap_rise_time {
                    push_num!("ch_trap_trise", v);
                }
                if let Some(v) = config.trap_flat_top {
                    push_num!("ch_trap_tflat", v);
                }
                if let Some(v) = config.trap_pole_zero {
                    push_num!("ch_tdecay", v);
                }
                if let Some(v) = config.peaking_time {
                    push_num!("ch_trap_ftd", v);
                }
                if let Some(ref v) = config.peak_nsmean {
                    push_str!("ch_peak_nsmean", v);
                }
                if let Some(v) = config.peak_holdoff {
                    push_num!("ch_peak_holdoff", v);
                }
                if let Some(v) = config.energy_fine_gain {
                    push_num!("ch_fgain", v);
                }
                // ---- Coincidence ----
                if let Some(ref v) = config.coincidence_mode {
                    push_str!("ch_trg_mode", v);
                }
                if let Some(ref v) = config.ch_veto_source {
                    push_str!("ch_veto_src", v);
                }
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
}

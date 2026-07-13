//! Event Builder configuration
//!
//! ELIFANT-Event 互換の JSON 形式をサポート。
//! KISS 原則: 最小限の構造で ELIFANT-Event と同じことができる。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// Configuration error type
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    Invalid(String),
}

/// Per-channel settings (ELIFANT-Event 互換)
///
/// Pure hardware descriptor: location, tags, and per-channel calibration.
/// Trigger / AC / threshold logic now lives in `eb_config.json` (L1 / L2
/// named-ops, SPEC § 6 + § 7). Old chSettings.json files carrying
/// `ID` / `IsEventTrigger` / `HasAC` / `ACModule` / `ACChannel` /
/// `ThresholdADC` still parse — serde silently ignores unknown fields — but
/// those fields are no longer wired to anything; users should migrate them
/// into the L1 / L2 ops as documented in SPEC v0.5.1 § 4.2.
///
/// JSON フィールド名は ELIFANT-Event と同じ (PascalCase)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChSettings {
    /// Module number
    pub module: u8,

    /// Channel number
    pub channel: u8,

    /// Detector type (Si, HPGe, AC, PMT, etc.)
    pub detector_type: String,

    /// User-defined tags (used by L2 `counter` ops to select hits — SPEC § 7).
    #[serde(default)]
    pub tags: Vec<String>,

    /// Energy calibration coefficients `E = p0 + p1·ADC + p2·ADC² + p3·ADC³`.
    /// Explicit `rename` keeps them lowercase — ELIFANT-Event and every
    /// existing chSettings.json uses `"p0".."p3"`. Without this the struct's
    /// `rename_all = "PascalCase"` would expect `"P0"`, drop the lowercase
    /// keys silently, and load zeros (regression caught 2026-05-21).
    #[serde(rename = "p0", default)]
    pub p0: f64,
    #[serde(rename = "p1", default)]
    pub p1: f64,
    #[serde(rename = "p2", default)]
    pub p2: f64,
    #[serde(rename = "p3", default)]
    pub p3: f64,
}

impl ChSettings {
    /// Get channel key (module << 8 | channel)
    #[inline]
    pub fn channel_key(&self) -> u16 {
        ((self.module as u16) << 8) | (self.channel as u16)
    }

    /// Apply energy calibration: E = p0 + p1*ADC + p2*ADC^2 + p3*ADC^3
    pub fn calibrate_energy(&self, adc: u16) -> f64 {
        let x = adc as f64;
        self.p0 + self.p1 * x + self.p2 * x * x + self.p3 * x * x * x
    }
}

/// Channel configuration (2D array: [module][channel])
///
/// ELIFANT-Event の chSettings.json と同じ構造
pub type ChannelConfig = Vec<Vec<ChSettings>>;

/// Load channel configuration from JSON file
pub fn load_channel_config(path: &Path) -> Result<ChannelConfig, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let config: ChannelConfig = serde_json::from_str(&content)?;
    Ok(config)
}

/// Save channel configuration to JSON file
pub fn save_channel_config(config: &ChannelConfig, path: &Path) -> Result<(), ConfigError> {
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Build a HashMap for fast channel lookup
pub fn build_channel_map(config: &ChannelConfig) -> HashMap<(u8, u8), ChSettings> {
    let mut map = HashMap::new();
    for module_channels in config {
        for ch in module_channels {
            map.insert((ch.module, ch.channel), ch.clone());
        }
    }
    map
}

/// Time calibration offsets
///
/// 簡素化: 単一参照チャンネルに対するオフセット HashMap
/// JSON では "module_channel" 形式の文字列キーで保存
#[derive(Debug, Clone, Default)]
pub struct TimeCalibration {
    /// Reference trigger module
    pub ref_module: u8,
    /// Reference trigger channel
    pub ref_channel: u8,
    /// Offsets: (module, channel) -> offset_ns
    offsets: HashMap<(u8, u8), f64>,
}

// Custom serialization for JSON compatibility
// Keys are zero-padded ("00_01") and sorted by (module, channel)
impl Serialize for TimeCalibration {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct CalibJson {
            ref_module: u8,
            ref_channel: u8,
            offsets: std::collections::BTreeMap<String, f64>,
        }

        // Zero-padded keys ensure BTreeMap lexicographic order = numeric order
        let string_offsets: std::collections::BTreeMap<String, f64> = self
            .offsets
            .iter()
            .map(|((m, c), v)| (format!("{:02}_{:02}", m, c), *v))
            .collect();

        let json = CalibJson {
            ref_module: self.ref_module,
            ref_channel: self.ref_channel,
            offsets: string_offsets,
        };

        json.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TimeCalibration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct CalibJson {
            ref_module: u8,
            ref_channel: u8,
            offsets: HashMap<String, f64>,
        }

        let json = CalibJson::deserialize(deserializer)?;

        let mut offsets = HashMap::new();
        for (key, value) in json.offsets {
            let parts: Vec<&str> = key.split('_').collect();
            let mut parsed = None;
            if parts.len() == 2 {
                if let (Ok(m), Ok(c)) = (parts[0].parse::<u8>(), parts[1].parse::<u8>()) {
                    parsed = Some((m, c));
                }
            }
            match parsed {
                Some(mc) => {
                    offsets.insert(mc, value);
                }
                // TODO 58 L6: a typo'd key ("O_3", "0-3", "0_3_1", …) used to be
                // skipped silently — the channel then ran with offset 0 and the
                // calibration looked mysteriously ineffective.
                None => tracing::warn!(
                    key = %key,
                    "Malformed time-calibration offset key (expected \"<module>_<channel>\") — entry ignored"
                ),
            }
        }

        Ok(TimeCalibration {
            ref_module: json.ref_module,
            ref_channel: json.ref_channel,
            offsets,
        })
    }
}

impl TimeCalibration {
    /// Create new empty calibration
    pub fn new(ref_module: u8, ref_channel: u8) -> Self {
        Self {
            ref_module,
            ref_channel,
            offsets: HashMap::new(),
        }
    }

    /// Get time offset for a channel
    pub fn get_offset(&self, module: u8, channel: u8) -> f64 {
        *self.offsets.get(&(module, channel)).unwrap_or(&0.0)
    }

    /// Set time offset for a channel
    pub fn set_offset(&mut self, module: u8, channel: u8, offset: f64) {
        self.offsets.insert((module, channel), offset);
    }

    /// Load from JSON file
    pub fn from_json_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let calib: Self = serde_json::from_str(&content)?;
        Ok(calib)
    }

    /// Save to JSON file
    pub fn to_json_file(&self, path: &Path) -> Result<(), ConfigError> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Check if calibration is empty
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Get all offsets
    pub fn offsets(&self) -> &HashMap<(u8, u8), f64> {
        &self.offsets
    }
}

/// Event building parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBuildingParams {
    /// Coincidence window [ns]
    pub coincidence_window_ns: f64,
    /// Time calibration window for histogram [ns]
    pub time_window_ns: f64,
}

impl Default for EventBuildingParams {
    fn default() -> Self {
        Self {
            coincidence_window_ns: 500.0,
            time_window_ns: 1000.0,
        }
    }
}

// ============================================================================
// L2 Settings - Level 2 filtering (Counter, Flag, Accept)
// ============================================================================

/// L2 Setting item - Counter, Flag, or Accept
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "Type")]
pub enum L2Setting {
    /// Counter: count hits matching tags
    Counter {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Tags")]
        tags: Vec<String>,
    },
    /// Flag: boolean condition (monitor > value, etc.)
    Flag {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Monitor")]
        monitor: String,
        #[serde(rename = "Operator")]
        operator: L2Operator,
        #[serde(rename = "Value")]
        value: i32,
    },
    /// Accept: logical combination of flags
    Accept {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Monitor")]
        monitor: Vec<String>,
        #[serde(rename = "Operator")]
        operator: L2LogicalOperator,
    },
}

impl L2Setting {
    /// Get the name of this setting
    pub fn name(&self) -> &str {
        match self {
            L2Setting::Counter { name, .. } => name,
            L2Setting::Flag { name, .. } => name,
            L2Setting::Accept { name, .. } => name,
        }
    }
}

/// Comparison operators for L2 Flag
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum L2Operator {
    #[serde(rename = ">")]
    GreaterThan,
    #[serde(rename = ">=")]
    GreaterEqual,
    #[serde(rename = "<")]
    LessThan,
    #[serde(rename = "<=")]
    LessEqual,
    #[serde(rename = "==")]
    Equal,
    #[serde(rename = "!=")]
    NotEqual,
}

/// Logical operators for L2 Accept
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum L2LogicalOperator {
    AND,
    OR,
}

/// L2 Settings collection
pub type L2Settings = Vec<L2Setting>;

/// Load L2 settings from JSON file
pub fn load_l2_settings(path: &Path) -> Result<L2Settings, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let settings: L2Settings = serde_json::from_str(&content)?;
    Ok(settings)
}

/// Save L2 settings to JSON file
pub fn save_l2_settings(settings: &L2Settings, path: &Path) -> Result<(), ConfigError> {
    let content = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ch_settings_channel_key() {
        let ch = ChSettings {
            module: 5,
            channel: 10,
            detector_type: "Si".to_string(),
            tags: vec![],
            p0: 0.0,
            p1: 1.0,
            p2: 0.0,
            p3: 0.0,
        };
        assert_eq!(ch.channel_key(), (5 << 8) | 10);
    }

    // `test_ch_settings_ac_pair` was deleted in Phase J — AC pairing moved
    // to L2 `ac_veto` op (see `l2_eval.rs::tests::ac_veto_*`).

    #[test]
    fn test_ch_settings_calibrate_energy() {
        let ch = ChSettings {
            module: 0,
            channel: 0,
            detector_type: "Si".to_string(),
            tags: vec![],
            p0: 10.0,
            p1: 2.0,
            p2: 0.0,
            p3: 0.0,
        };
        assert_eq!(ch.calibrate_energy(100), 10.0 + 2.0 * 100.0);
    }

    #[test]
    fn test_time_calibration() {
        let mut calib = TimeCalibration::new(0, 0);
        assert!(calib.is_empty());

        calib.set_offset(1, 0, 10.5);
        calib.set_offset(1, 1, -5.0);

        assert_eq!(calib.get_offset(1, 0), 10.5);
        assert_eq!(calib.get_offset(1, 1), -5.0);
        assert_eq!(calib.get_offset(2, 0), 0.0); // Non-existent returns 0
        assert!(!calib.is_empty());
    }

    #[test]
    fn test_build_channel_map() {
        let config: ChannelConfig = vec![vec![
            ChSettings {
                module: 0,
                channel: 0,
                detector_type: "Si".to_string(),
                tags: vec![],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
            ChSettings {
                module: 0,
                channel: 1,
                detector_type: "Si".to_string(),
                tags: vec!["dE".to_string()],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
        ]];

        let map = build_channel_map(&config);
        assert_eq!(map.len(), 2);
        // Pure-descriptor checks (Phase J).
        assert_eq!(map.get(&(0, 0)).unwrap().detector_type, "Si");
        assert_eq!(map.get(&(0, 1)).unwrap().tags, vec!["dE"]);
    }

    #[test]
    fn test_lowercase_p_fields_load_correctly() {
        // ELIFANT-Event and every existing chSettings.json file uses lowercase
        // `"p0".."p3"`. The struct's `rename_all = "PascalCase"` would
        // otherwise demand `"P0"` and silently drop the data into defaults.
        // This test guards against that regression.
        let json = r#"[[
            {"Module": 0, "Channel": 0, "DetectorType": "Si", "Tags": [],
             "p0": 11.0, "p1": 0.5, "p2": 0.25, "p3": -0.125}
        ]]"#;
        let cfg: ChannelConfig = serde_json::from_str(json).unwrap();
        let ch = &cfg[0][0];
        assert_eq!(ch.p0, 11.0);
        assert_eq!(ch.p1, 0.5);
        assert_eq!(ch.p2, 0.25);
        assert_eq!(ch.p3, -0.125);
    }

    #[test]
    fn test_legacy_id_field_ignored() {
        // chSettings.json files predating SPEC v0.7.1 carry an `"ID"` key.
        // serde must drop it silently so old configs keep loading.
        let json = r#"[[
            {"ID": 7, "Module": 0, "Channel": 0, "DetectorType": "Si",
             "Tags": ["dE"], "p0": 0.0, "p1": 1.0, "p2": 0.0, "p3": 0.0}
        ]]"#;
        let cfg: ChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg[0][0].module, 0);
        assert_eq!(cfg[0][0].tags, vec!["dE"]);
    }

    // `test_get_trigger_channels` removed in Phase J — the helper was a
    // wrapper around `ch.is_event_trigger`, which is no longer a field.
    // Trigger channels now come from `EbRuntimeConfig::build_trigger_config()`.

    #[test]
    fn test_l2_settings_counter_serialization() {
        let counter = L2Setting::Counter {
            name: "E_Sector_Counter".to_string(),
            tags: vec!["E_Sector".to_string()],
        };

        let json = serde_json::to_string(&counter).unwrap();
        assert!(json.contains("\"Type\":\"Counter\""));
        assert!(json.contains("\"Name\":\"E_Sector_Counter\""));
        assert!(json.contains("\"Tags\":[\"E_Sector\"]"));

        // Round-trip
        let parsed: L2Setting = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name(), "E_Sector_Counter");
    }

    #[test]
    fn test_l2_settings_flag_serialization() {
        let flag = L2Setting::Flag {
            name: "E_More_Than_0".to_string(),
            monitor: "E_Sector_Counter".to_string(),
            operator: L2Operator::GreaterThan,
            value: 0,
        };

        let json = serde_json::to_string(&flag).unwrap();
        assert!(json.contains("\"Type\":\"Flag\""));
        assert!(json.contains("\"Operator\":\">\""));

        // Round-trip
        let parsed: L2Setting = serde_json::from_str(&json).unwrap();
        if let L2Setting::Flag {
            operator, value, ..
        } = parsed
        {
            assert_eq!(operator, L2Operator::GreaterThan);
            assert_eq!(value, 0);
        } else {
            panic!("Expected Flag");
        }
    }

    #[test]
    fn test_l2_settings_accept_serialization() {
        let accept = L2Setting::Accept {
            name: "Si_Both".to_string(),
            monitor: vec!["E_More_Than_0".to_string(), "dE_More_Than_0".to_string()],
            operator: L2LogicalOperator::AND,
        };

        let json = serde_json::to_string(&accept).unwrap();
        assert!(json.contains("\"Type\":\"Accept\""));
        assert!(json.contains("\"Operator\":\"AND\""));

        // Round-trip
        let parsed: L2Setting = serde_json::from_str(&json).unwrap();
        if let L2Setting::Accept { operator, .. } = parsed {
            assert_eq!(operator, L2LogicalOperator::AND);
        } else {
            panic!("Expected Accept");
        }
    }

    #[test]
    fn test_l2_settings_full_example() {
        // Parse the actual ELIFANT-Event L2Settings.json format
        let json = r#"[
            {
                "Type": "Counter",
                "Name": "E_Sector_Counter",
                "Tags": ["E_Sector"]
            },
            {
                "Type": "Flag",
                "Name": "E_More_Than_0",
                "Monitor": "E_Sector_Counter",
                "Operator": ">",
                "Value": 0
            },
            {
                "Type": "Accept",
                "Name": "Si_Both",
                "Monitor": ["E_More_Than_0", "dE_More_Than_0"],
                "Operator": "AND"
            }
        ]"#;

        let settings: L2Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.len(), 3);

        assert_eq!(settings[0].name(), "E_Sector_Counter");
        assert_eq!(settings[1].name(), "E_More_Than_0");
        assert_eq!(settings[2].name(), "Si_Both");
    }
}

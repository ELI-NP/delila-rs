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
/// JSON フィールド名は ELIFANT-Event と同じ (PascalCase)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChSettings {
    /// Unique detector ID
    #[serde(rename = "ID")]
    pub id: i32,

    /// Module number
    pub module: u8,

    /// Channel number
    pub channel: u8,

    /// Is this a trigger channel?
    pub is_event_trigger: bool,

    /// ADC threshold for accepting hits
    #[serde(rename = "ThresholdADC")]
    pub threshold_adc: u32,

    /// Has associated AC detector?
    #[serde(rename = "HasAC")]
    pub has_ac: bool,

    /// AC detector module
    #[serde(rename = "ACModule")]
    pub ac_module: u8,

    /// AC detector channel
    #[serde(rename = "ACChannel")]
    pub ac_channel: u8,

    /// Detector type (Si, HPGe, AC, PMT, etc.)
    pub detector_type: String,

    /// User-defined tags
    #[serde(default)]
    pub tags: Vec<String>,

    /// Energy calibration coefficients
    #[serde(default)]
    pub p0: f64,
    #[serde(default)]
    pub p1: f64,
    #[serde(default)]
    pub p2: f64,
    #[serde(default)]
    pub p3: f64,
}

impl ChSettings {
    /// Get channel key (module << 8 | channel)
    #[inline]
    pub fn channel_key(&self) -> u16 {
        ((self.module as u16) << 8) | (self.channel as u16)
    }

    /// Get AC pair channel key if has_ac is true
    pub fn ac_channel_key(&self) -> Option<u16> {
        if self.has_ac && self.ac_module != 128 {
            // 128 is "no AC" marker in ELIFANT-Event
            Some(((self.ac_module as u16) << 8) | (self.ac_channel as u16))
        } else {
            None
        }
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

/// Get all trigger channels from configuration
pub fn get_trigger_channels(config: &ChannelConfig) -> Vec<(u8, u8)> {
    let mut triggers = Vec::new();
    for module_channels in config {
        for ch in module_channels {
            if ch.is_event_trigger {
                triggers.push((ch.module, ch.channel));
            }
        }
    }
    triggers
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
            if parts.len() == 2 {
                if let (Ok(m), Ok(c)) = (parts[0].parse::<u8>(), parts[1].parse::<u8>()) {
                    offsets.insert((m, c), value);
                }
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
            id: 0,
            module: 5,
            channel: 10,
            is_event_trigger: false,
            threshold_adc: 0,
            has_ac: false,
            ac_module: 128,
            ac_channel: 128,
            detector_type: "Si".to_string(),
            tags: vec![],
            p0: 0.0,
            p1: 1.0,
            p2: 0.0,
            p3: 0.0,
        };
        assert_eq!(ch.channel_key(), (5 << 8) | 10);
    }

    #[test]
    fn test_ch_settings_ac_pair() {
        let ch_with_ac = ChSettings {
            id: 0,
            module: 0,
            channel: 0,
            is_event_trigger: true,
            threshold_adc: 0,
            has_ac: true,
            ac_module: 0,
            ac_channel: 1,
            detector_type: "HPGe".to_string(),
            tags: vec![],
            p0: 0.0,
            p1: 1.0,
            p2: 0.0,
            p3: 0.0,
        };
        assert_eq!(ch_with_ac.ac_channel_key(), Some(1));

        let ch_no_ac = ChSettings {
            has_ac: false,
            ac_module: 128,
            ac_channel: 128,
            ..ch_with_ac.clone()
        };
        assert_eq!(ch_no_ac.ac_channel_key(), None);
    }

    #[test]
    fn test_ch_settings_calibrate_energy() {
        let ch = ChSettings {
            id: 0,
            module: 0,
            channel: 0,
            is_event_trigger: false,
            threshold_adc: 0,
            has_ac: false,
            ac_module: 128,
            ac_channel: 128,
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
                id: 0,
                module: 0,
                channel: 0,
                is_event_trigger: true,
                threshold_adc: 0,
                has_ac: false,
                ac_module: 128,
                ac_channel: 128,
                detector_type: "Si".to_string(),
                tags: vec![],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
            ChSettings {
                id: 1,
                module: 0,
                channel: 1,
                is_event_trigger: false,
                threshold_adc: 0,
                has_ac: false,
                ac_module: 128,
                ac_channel: 128,
                detector_type: "Si".to_string(),
                tags: vec![],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
        ]];

        let map = build_channel_map(&config);
        assert_eq!(map.len(), 2);
        assert!(map.get(&(0, 0)).unwrap().is_event_trigger);
        assert!(!map.get(&(0, 1)).unwrap().is_event_trigger);
    }

    #[test]
    fn test_get_trigger_channels() {
        let config: ChannelConfig = vec![vec![
            ChSettings {
                id: 0,
                module: 0,
                channel: 0,
                is_event_trigger: true,
                threshold_adc: 0,
                has_ac: false,
                ac_module: 128,
                ac_channel: 128,
                detector_type: "Si".to_string(),
                tags: vec![],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
            ChSettings {
                id: 1,
                module: 0,
                channel: 1,
                is_event_trigger: false,
                threshold_adc: 0,
                has_ac: false,
                ac_module: 128,
                ac_channel: 128,
                detector_type: "Si".to_string(),
                tags: vec![],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
        ]];

        let triggers = get_trigger_channels(&config);
        assert_eq!(triggers, vec![(0, 0)]);
    }

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

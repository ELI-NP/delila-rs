//! Configuration module for DELILA DAQ system
//!
//! Supports loading configuration from:
//! - TOML files (network topology, infrastructure)
//! - JSON files (digitizer settings)
//! - MongoDB (operational settings) - future
//!
//! # Example
//! ```ignore
//! let config = Config::load("config.toml")?;
//! let merger_net = config.network.merger.as_ref().unwrap();
//! ```

pub mod digitizer;

pub use digitizer::{
    BoardConfig, CaenParameter, ChannelConfig, DigitizerConfig, DigitizerConfigError, FirmwareType,
    SyncConfig,
};

use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

/// Configuration errors
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse TOML: {0}")]
    TomlError(#[from] toml::de::Error),

    #[error("Failed to parse digitizer JSON config: {0}")]
    DigitizerConfigError(#[from] DigitizerConfigError),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("MongoDB not yet supported")]
    MongoDbNotSupported,
}

/// Top-level configuration
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub network: NetworkConfig,
    #[serde(default)]
    pub settings: SettingsConfig,
    /// Operator configuration
    #[serde(default)]
    pub operator: OperatorFileConfig,
}

/// Operator configuration from config file
#[derive(Debug, Clone, Deserialize)]
pub struct OperatorFileConfig {
    /// Experiment name (server-authoritative, not editable by UI)
    #[serde(default = "default_experiment_name")]
    pub experiment_name: String,
    /// HTTP server port (default: 9090)
    #[serde(default = "default_operator_port")]
    pub port: u16,
    /// Directory containing built Angular UI files (serves at /)
    /// If not set, auto-detects from web/operator-ui/dist/operator-ui/browser/
    pub web_ui_dir: Option<String>,
}

fn default_operator_port() -> u16 {
    9090
}

impl Default for OperatorFileConfig {
    fn default() -> Self {
        Self {
            experiment_name: default_experiment_name(),
            port: default_operator_port(),
            web_ui_dir: None,
        }
    }
}

fn default_experiment_name() -> String {
    "DefaultExp".to_string()
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from a TOML string (useful for testing)
    pub fn from_toml(content: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(content)?;
        Ok(config)
    }

    /// Get source configuration by ID
    pub fn get_source(&self, source_id: u32) -> Option<&SourceNetworkConfig> {
        self.network.sources.iter().find(|s| s.id == source_id)
    }

    /// Get resolved Merger subscribe addresses.
    /// If `merger.subscribe` is empty, auto-generate from source `data_connect_address()`.
    /// This ensures the Merger always connects to the correct host for each source.
    pub fn resolved_merger_subscribe(&self) -> Vec<String> {
        if let Some(ref merger) = self.network.merger {
            if merger.subscribe.is_empty() {
                // Auto-generate from sources using port_base_data
                let port_base = self.network.port_base_data;
                self.network
                    .sources
                    .iter()
                    .map(|s| s.data_connect_address_with_base(port_base))
                    .collect()
            } else {
                merger.subscribe.clone()
            }
        } else {
            Vec::new()
        }
    }
}

// =============================================================================
// Network Configuration
// =============================================================================

/// Network topology configuration
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    /// Cluster name for identification
    #[serde(default = "default_cluster_name")]
    pub cluster_name: String,

    /// Base port for source data (PUB) sockets: port = port_base_data + source_id.
    /// Only used when `bind` is omitted in a `[[network.sources]]` entry.
    #[serde(default = "default_port_base_data")]
    pub port_base_data: u16,

    /// Base port for source command (REP) sockets: port = port_base_command + source_id.
    /// Only used when `command` is omitted in a `[[network.sources]]` entry.
    #[serde(default = "default_port_base_command")]
    pub port_base_command: u16,

    /// Data source configurations
    #[serde(default)]
    pub sources: Vec<SourceNetworkConfig>,

    /// Merger configuration
    pub merger: Option<MergerNetworkConfig>,

    /// Recorder configuration
    pub recorder: Option<RecorderNetworkConfig>,

    /// Monitor configuration
    pub monitor: Option<MonitorNetworkConfig>,
}

fn default_port_base_data() -> u16 {
    6000
}

fn default_port_base_command() -> u16 {
    6100
}

fn default_cluster_name() -> String {
    "default".to_string()
}

/// Data source type
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    /// Emulator (dummy data generator for testing)
    #[default]
    Emulator,
    /// CAEN PSD firmware (legacy, via CAEN library)
    #[serde(alias = "PSD1", alias = "psd1")]
    Psd1,
    /// CAEN PSD2 firmware (via dig2 library)
    #[serde(alias = "PSD2", alias = "psd2")]
    Psd2,
    /// CAEN PHA firmware (via CAEN library)
    #[serde(alias = "PHA1", alias = "pha1")]
    Pha1,
    /// CAEN DPP-ZLE firmware (future)
    #[serde(alias = "ZLE", alias = "zle")]
    Zle,
    /// DELILA AMax firmware (Trapezoidal Filter MCA, custom DPP_OPEN)
    #[serde(alias = "AMax", alias = "AMAX")]
    AMax,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceType::Emulator => write!(f, "Emulator"),
            SourceType::Psd1 => write!(f, "PSD1"),
            SourceType::Psd2 => write!(f, "PSD2"),
            SourceType::Pha1 => write!(f, "PHA1"),
            SourceType::Zle => write!(f, "ZLE"),
            SourceType::AMax => write!(f, "AMax"),
        }
    }
}

/// Data source (emulator/digitizer) network config
#[derive(Debug, Clone, Deserialize)]
pub struct SourceNetworkConfig {
    /// Unique source ID
    pub id: u32,

    /// Human-readable name
    #[serde(default)]
    pub name: String,

    /// Source type (emulator, psd1, psd2, pha1, zle)
    #[serde(default, rename = "type")]
    pub source_type: SourceType,

    /// ZMQ bind address for data (e.g., "tcp://*:5555").
    /// If omitted, auto-allocated as "tcp://*:{port_base_data + id}".
    #[serde(default)]
    pub bind: Option<String>,

    /// ZMQ bind address for commands (e.g., "tcp://*:5560")
    #[serde(default)]
    pub command: Option<String>,

    /// Path to digitizer configuration file (JSON)
    /// e.g., "config/digitizers/digitizer_0.json"
    #[serde(default)]
    pub config_file: Option<String>,

    /// Digitizer URL (e.g., "dig2://172.18.4.56")
    /// Required for PSD2; optional for PSD1/PHA1 (uses USB/Optical)
    #[serde(default)]
    pub digitizer_url: Option<String>,

    /// Module ID for event tagging
    #[serde(default)]
    pub module_id: Option<u8>,

    /// ADC time step in nanoseconds (default: 2.0 for 500 MHz)
    #[serde(default)]
    pub time_step_ns: Option<f64>,

    /// Pipeline order for Start/Stop sequencing (1 = upstream, default: 1)
    #[serde(default = "default_source_pipeline_order")]
    pub pipeline_order: u32,

    /// Master digitizer flag for synchronized start
    ///
    /// In a multi-digitizer setup with TrgOut cascade:
    /// - Master (is_master=true): Receives SWstart command
    /// - Slaves (is_master=false): Auto-start via TrgOut cascade from master
    ///
    /// Start sequence:
    /// 1. Arm all digitizers (parallel)
    /// 2. Start master only → Slaves auto-start via TrgOut
    #[serde(default)]
    pub is_master: bool,

    /// Hostname or IP address where this Reader is running.
    /// Used by the Operator to resolve `tcp://*:PORT` bind addresses
    /// into connect addresses (`tcp://{host}:PORT`).
    /// Defaults to "localhost" (same machine).
    ///
    /// Example: `host = "172.18.4.147"` for a USB digitizer Reader
    /// running on a remote Linux machine.
    #[serde(default = "default_host")]
    pub host: String,
}

fn default_source_pipeline_order() -> u32 {
    1 // Sources are upstream
}

fn default_host() -> String {
    "localhost".to_string()
}

impl SourceNetworkConfig {
    /// Check if this source is a real digitizer (not emulator)
    pub fn is_digitizer(&self) -> bool {
        self.source_type != SourceType::Emulator
    }

    /// Check if this source is an emulator
    pub fn is_emulator(&self) -> bool {
        self.source_type == SourceType::Emulator
    }

    /// Check if this source is the master digitizer
    pub fn is_master_digitizer(&self) -> bool {
        self.is_master && self.is_digitizer()
    }

    /// Get data bind address with auto-allocation fallback.
    /// `port_base` is typically `NetworkConfig::port_base_data`.
    pub fn data_address(&self, port_base: u16) -> String {
        self.bind
            .clone()
            .unwrap_or_else(|| format!("tcp://*:{}", port_base + self.id as u16))
    }

    /// Get command bind address with auto-allocation fallback (legacy default: 5560 + id).
    pub fn command_address(&self) -> String {
        self.command
            .clone()
            .unwrap_or_else(|| format!("tcp://*:{}", 5560 + self.id as u16))
    }

    /// Get command bind address with configurable base port.
    /// `port_base` is typically `NetworkConfig::port_base_command`.
    pub fn command_address_with_base(&self, port_base: u16) -> String {
        self.command
            .clone()
            .unwrap_or_else(|| format!("tcp://*:{}", port_base + self.id as u16))
    }

    /// Resolve a bind address (`tcp://*:PORT`) to a connect address
    /// using the configured `host`. If the address already contains
    /// a specific hostname/IP (not `*`), it is returned as-is.
    pub fn resolve_address(&self, bind_addr: &str) -> String {
        bind_addr.replace("tcp://*:", &format!("tcp://{}:", self.host))
    }

    /// Get the command connect address for the Operator to reach this Reader.
    pub fn command_connect_address(&self) -> String {
        self.resolve_address(&self.command_address())
    }

    /// Get the command connect address with configurable base port.
    pub fn command_connect_address_with_base(&self, port_base: u16) -> String {
        self.resolve_address(&self.command_address_with_base(port_base))
    }

    /// Get the data connect address for Merger to subscribe to this Reader (legacy, uses 6000 base).
    pub fn data_connect_address(&self) -> String {
        self.resolve_address(&self.data_address(default_port_base_data()))
    }

    /// Get the data connect address with configurable base port.
    pub fn data_connect_address_with_base(&self, port_base: u16) -> String {
        self.resolve_address(&self.data_address(port_base))
    }

    /// Check if this source runs on a remote machine (host != localhost).
    pub fn is_remote(&self) -> bool {
        self.host != "localhost" && self.host != "127.0.0.1"
    }

    /// Load digitizer configuration from the config_file path
    /// Returns None if no config_file is specified
    pub fn load_digitizer_config(&self) -> Result<Option<DigitizerConfig>, ConfigError> {
        match &self.config_file {
            Some(path) => {
                let config =
                    DigitizerConfig::load(path).map_err(ConfigError::DigitizerConfigError)?;
                Ok(Some(config))
            }
            None => Ok(None),
        }
    }

    /// Load digitizer configuration, returning an error if config_file is not set
    /// Use this when config is required (e.g., for real digitizers)
    pub fn load_digitizer_config_required(&self) -> Result<DigitizerConfig, ConfigError> {
        match &self.config_file {
            Some(path) => {
                let config =
                    DigitizerConfig::load(path).map_err(ConfigError::DigitizerConfigError)?;
                Ok(config)
            }
            None => Err(ConfigError::MissingField(format!(
                "config_file required for source '{}' (type: {})",
                self.name, self.source_type
            ))),
        }
    }
}

/// Merger network configuration
#[derive(Debug, Clone, Deserialize)]
pub struct MergerNetworkConfig {
    /// ZMQ addresses to subscribe to (upstream sources).
    /// If empty, auto-generated from source `data_connect_address()`.
    #[serde(default)]
    pub subscribe: Vec<String>,

    /// ZMQ address to publish to (downstream)
    pub publish: String,

    /// ZMQ bind address for commands (e.g., "tcp://*:5570")
    #[serde(default)]
    pub command: Option<String>,

    /// Pipeline order for Start/Stop sequencing (default: 2)
    #[serde(default = "default_merger_pipeline_order")]
    pub pipeline_order: u32,
}

fn default_merger_pipeline_order() -> u32 {
    2 // Merger is in the middle
}

/// Recorder network configuration
#[derive(Debug, Clone, Deserialize)]
pub struct RecorderNetworkConfig {
    /// ZMQ address to subscribe to
    pub subscribe: String,

    /// ZMQ bind address for commands (e.g., "tcp://*:5580")
    #[serde(default)]
    pub command: Option<String>,

    /// Output directory for data files
    #[serde(default = "default_output_dir")]
    pub output_dir: String,

    /// Maximum file size in MB (default: 1024 = 1GB)
    #[serde(default = "default_max_file_size_mb")]
    pub max_file_size_mb: u64,

    /// Maximum file duration in seconds (default: 600 = 10 minutes)
    #[serde(default = "default_max_file_duration_sec")]
    pub max_file_duration_sec: u64,

    /// Pipeline order for Start/Stop sequencing (default: 3)
    #[serde(default = "default_sink_pipeline_order")]
    pub pipeline_order: u32,
}

fn default_output_dir() -> String {
    "./data".to_string()
}

fn default_max_file_size_mb() -> u64 {
    1024 // 1GB
}

fn default_max_file_duration_sec() -> u64 {
    600 // 10 minutes
}

fn default_sink_pipeline_order() -> u32 {
    3 // Sinks (Recorder/Monitor) are downstream
}

/// Monitor network configuration
#[derive(Debug, Clone, Deserialize)]
pub struct MonitorNetworkConfig {
    /// ZMQ address to subscribe to
    pub subscribe: String,

    /// ZMQ bind address for commands (e.g., "tcp://*:5590")
    #[serde(default)]
    pub command: Option<String>,

    /// HTTP server port for web UI
    #[serde(default = "default_http_port")]
    pub http_port: u16,

    /// Pipeline order for Start/Stop sequencing (default: 3)
    #[serde(default = "default_sink_pipeline_order")]
    pub pipeline_order: u32,
}

fn default_http_port() -> u16 {
    8081
}

// =============================================================================
// Settings Configuration
// =============================================================================

/// Where to load operational settings from
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SettingsSource {
    #[default]
    File,
    #[serde(rename = "mongodb")]
    MongoDB,
}

/// Settings configuration
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsConfig {
    /// Source of settings
    #[serde(default)]
    pub source: SettingsSource,

    /// File-based settings
    #[serde(default)]
    pub file: FileSettings,

    /// MongoDB connection settings (future)
    pub mongodb: Option<MongoDbSettings>,
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            source: SettingsSource::File,
            file: FileSettings::default(),
            mongodb: None,
        }
    }
}

impl SettingsConfig {
    /// Get the effective settings based on the configured source
    pub fn get_settings(&self) -> Result<Settings, ConfigError> {
        match self.source {
            SettingsSource::File => Ok(Settings::from(&self.file)),
            SettingsSource::MongoDB => Err(ConfigError::MongoDbNotSupported),
        }
    }
}

/// File-based operational settings
#[derive(Debug, Clone, Deserialize)]
pub struct FileSettings {
    /// Events per batch
    #[serde(default = "default_events_per_batch")]
    pub events_per_batch: u32,

    /// Batch interval in milliseconds
    #[serde(default = "default_batch_interval_ms")]
    pub batch_interval_ms: u64,

    /// Number of modules per digitizer
    #[serde(default = "default_num_modules")]
    pub num_modules: u32,

    /// Channels per module
    #[serde(default = "default_channels_per_module")]
    pub channels_per_module: u32,

    /// Enable waveform generation (emulator)
    #[serde(default)]
    pub enable_waveform: bool,

    /// Waveform probe bitmask (1=analog1, 2=analog2, 3=both analog, 63=all)
    #[serde(default = "default_waveform_probes")]
    pub waveform_probes: u8,

    /// Number of waveform samples
    #[serde(default = "default_waveform_samples")]
    pub waveform_samples: usize,
}

impl Default for FileSettings {
    fn default() -> Self {
        Self {
            events_per_batch: default_events_per_batch(),
            batch_interval_ms: default_batch_interval_ms(),
            num_modules: default_num_modules(),
            channels_per_module: default_channels_per_module(),
            enable_waveform: false,
            waveform_probes: default_waveform_probes(),
            waveform_samples: default_waveform_samples(),
        }
    }
}

fn default_events_per_batch() -> u32 {
    100
}
fn default_batch_interval_ms() -> u64 {
    100
}
fn default_num_modules() -> u32 {
    2
}
fn default_channels_per_module() -> u32 {
    16
}
fn default_waveform_probes() -> u8 {
    3 // Both analog probes
}
fn default_waveform_samples() -> usize {
    512
}

/// MongoDB connection settings (future)
#[derive(Debug, Clone, Deserialize)]
pub struct MongoDbSettings {
    /// MongoDB URI
    pub uri: String,

    /// Database name
    pub database: String,

    /// Collection name
    #[serde(default = "default_collection")]
    pub collection: String,
}

fn default_collection() -> String {
    "run_config".to_string()
}

/// Unified operational settings (loaded from file or MongoDB)
#[derive(Debug, Clone)]
pub struct Settings {
    pub events_per_batch: u32,
    pub batch_interval_ms: u64,
    pub num_modules: u32,
    pub channels_per_module: u32,
    pub enable_waveform: bool,
    pub waveform_probes: u8,
    pub waveform_samples: usize,
}

impl From<&FileSettings> for Settings {
    fn from(file: &FileSettings) -> Self {
        Self {
            events_per_batch: file.events_per_batch,
            batch_interval_ms: file.batch_interval_ms,
            num_modules: file.num_modules,
            channels_per_module: file.channels_per_module,
            enable_waveform: file.enable_waveform,
            waveform_probes: file.waveform_probes,
            waveform_samples: file.waveform_samples,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[network]
cluster_name = "test"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.network.cluster_name, "test");
        assert!(config.network.sources.is_empty());
    }

    #[test]
    fn parse_full_config() {
        let toml = r#"
[network]
cluster_name = "daq-cluster-1"

[[network.sources]]
id = 1
name = "digitizer-1"
bind = "tcp://*:5555"

[[network.sources]]
id = 2
name = "digitizer-2"
bind = "tcp://*:5556"

[network.merger]
subscribe = ["tcp://localhost:5555", "tcp://localhost:5556"]
publish = "tcp://*:5557"

[network.recorder]
subscribe = "tcp://localhost:5557"
output_dir = "/data/runs"

[network.monitor]
subscribe = "tcp://localhost:5557"
http_port = 9000

[settings]
source = "file"

[settings.file]
events_per_batch = 200
batch_interval_ms = 50
"#;
        let config = Config::from_toml(toml).unwrap();

        // Network
        assert_eq!(config.network.cluster_name, "daq-cluster-1");
        assert_eq!(config.network.sources.len(), 2);
        assert_eq!(config.network.sources[0].id, 1);
        assert_eq!(
            config.network.sources[1].bind,
            Some("tcp://*:5556".to_string())
        );

        // Merger
        let merger = config.network.merger.as_ref().unwrap();
        assert_eq!(merger.subscribe.len(), 2);
        assert_eq!(merger.publish, "tcp://*:5557");

        // Recorder
        let recorder = config.network.recorder.as_ref().unwrap();
        assert_eq!(recorder.output_dir, "/data/runs");

        // Monitor
        let monitor = config.network.monitor.as_ref().unwrap();
        assert_eq!(monitor.http_port, 9000);

        // Settings
        assert_eq!(config.settings.source, SettingsSource::File);
        let settings = config.settings.get_settings().unwrap();
        assert_eq!(settings.events_per_batch, 200);
        assert_eq!(settings.batch_interval_ms, 50);
    }

    #[test]
    fn default_settings() {
        let toml = r#"
[network]
cluster_name = "test"
"#;
        let config = Config::from_toml(toml).unwrap();
        let settings = config.settings.get_settings().unwrap();

        assert_eq!(settings.events_per_batch, 100);
        assert_eq!(settings.batch_interval_ms, 100);
    }

    #[test]
    fn mongodb_not_supported() {
        let toml = r#"
[network]
cluster_name = "test"

[settings]
source = "mongodb"

[settings.mongodb]
uri = "mongodb://localhost:27017"
database = "delila"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert!(config.settings.get_settings().is_err());
    }

    #[test]
    fn parse_digitizer_source() {
        let toml = r#"
[network]
cluster_name = "test"

[[network.sources]]
id = 0
name = "digitizer-0"
type = "psd2"
bind = "tcp://*:5555"
command = "tcp://*:5560"
digitizer_url = "dig2://172.18.4.56"
module_id = 1
time_step_ns = 4.0
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.network.sources.len(), 1);

        let source = &config.network.sources[0];
        assert!(source.is_digitizer());
        assert!(!source.is_emulator());
        assert_eq!(source.source_type, SourceType::Psd2);
        assert_eq!(source.digitizer_url, Some("dig2://172.18.4.56".to_string()));
        assert_eq!(source.module_id, Some(1));
        assert_eq!(source.time_step_ns, Some(4.0));
        assert_eq!(source.command_address(), "tcp://*:5560".to_string());
    }

    #[test]
    fn emulator_source_is_not_digitizer() {
        let toml = r#"
[network]
cluster_name = "test"

[[network.sources]]
id = 0
name = "emulator-0"
type = "emulator"
bind = "tcp://*:5555"
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        // type = "emulator" -> not a digitizer
        assert!(!source.is_digitizer());
        assert!(source.is_emulator());
        assert_eq!(source.source_type, SourceType::Emulator);

        // Command address uses default
        assert_eq!(source.command_address(), "tcp://*:5560".to_string());
    }

    #[test]
    fn emulator_is_default_type() {
        let toml = r#"
[network]
cluster_name = "test"

[[network.sources]]
id = 0
name = "source-0"
bind = "tcp://*:5555"
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        // Default type is emulator
        assert_eq!(source.source_type, SourceType::Emulator);
        assert!(source.is_emulator());
    }

    #[test]
    fn get_source_by_id() {
        let toml = r#"
[network]
cluster_name = "test"

[[network.sources]]
id = 0
name = "source-0"
bind = "tcp://*:5555"

[[network.sources]]
id = 2
name = "source-2"
type = "psd2"
bind = "tcp://*:5557"
digitizer_url = "dig2://192.168.1.100"
"#;
        let config = Config::from_toml(toml).unwrap();

        // Find source 0
        let s0 = config.get_source(0);
        assert!(s0.is_some());
        assert_eq!(s0.unwrap().name, "source-0");

        // Find source 2 (PSD2 digitizer)
        let s2 = config.get_source(2);
        assert!(s2.is_some());
        assert_eq!(s2.unwrap().name, "source-2");
        assert!(s2.unwrap().is_digitizer());
        assert_eq!(s2.unwrap().source_type, SourceType::Psd2);

        // Source 1 doesn't exist
        assert!(config.get_source(1).is_none());
    }

    #[test]
    fn parse_all_source_types() {
        let toml = r#"
[network]
[[network.sources]]
id = 0
name = "emu"
type = "emulator"
bind = "tcp://*:5550"

[[network.sources]]
id = 1
name = "psd1"
type = "PSD1"
bind = "tcp://*:5551"

[[network.sources]]
id = 2
name = "psd2"
type = "psd2"
bind = "tcp://*:5552"

[[network.sources]]
id = 3
name = "pha1"
type = "PHA1"
bind = "tcp://*:5553"

[[network.sources]]
id = 4
name = "zle"
type = "ZLE"
bind = "tcp://*:5554"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.network.sources.len(), 5);

        assert_eq!(config.network.sources[0].source_type, SourceType::Emulator);
        assert_eq!(config.network.sources[1].source_type, SourceType::Psd1);
        assert_eq!(config.network.sources[2].source_type, SourceType::Psd2);
        assert_eq!(config.network.sources[3].source_type, SourceType::Pha1);
        assert_eq!(config.network.sources[4].source_type, SourceType::Zle);
    }

    #[test]
    fn parse_source_with_config_file() {
        let toml = r#"
[network]
[[network.sources]]
id = 0
name = "digitizer-0"
type = "psd2"
bind = "tcp://*:5555"
config_file = "config/digitizers/digitizer_0.json"
digitizer_url = "dig2://172.18.4.56"
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        assert_eq!(source.source_type, SourceType::Psd2);
        assert_eq!(
            source.config_file,
            Some("config/digitizers/digitizer_0.json".to_string())
        );
        assert!(source.is_digitizer());
    }

    #[test]
    fn parse_master_slave_sources() {
        let toml = r#"
[network]
[[network.sources]]
id = 0
name = "master"
type = "psd2"
bind = "tcp://*:5555"
digitizer_url = "dig2://172.18.4.100"
is_master = true

[[network.sources]]
id = 1
name = "slave"
type = "psd2"
bind = "tcp://*:5556"
digitizer_url = "dig2://172.18.4.101"
is_master = false
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.network.sources.len(), 2);

        let master = &config.network.sources[0];
        assert!(master.is_master);
        assert!(master.is_master_digitizer());

        let slave = &config.network.sources[1];
        assert!(!slave.is_master);
        assert!(!slave.is_master_digitizer());
    }

    #[test]
    fn emulator_is_not_master_digitizer() {
        let toml = r#"
[network]
[[network.sources]]
id = 0
name = "emulator"
type = "emulator"
bind = "tcp://*:5555"
is_master = true
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        // is_master is true, but it's an emulator, not a digitizer
        assert!(source.is_master);
        assert!(!source.is_master_digitizer());
    }

    #[test]
    fn load_digitizer_config_no_file() {
        let toml = r#"
[network]
[[network.sources]]
id = 0
name = "emulator-0"
type = "emulator"
bind = "tcp://*:5555"
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        // No config_file set, should return None
        let result = source.load_digitizer_config();
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn load_digitizer_config_required_missing() {
        let toml = r#"
[network]
[[network.sources]]
id = 0
name = "digitizer-0"
type = "psd2"
bind = "tcp://*:5555"
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        // config_file not set but required
        let result = source.load_digitizer_config_required();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("config_file required"));
    }

    #[test]
    fn auto_allocate_data_port() {
        let toml = r#"
[network]
[[network.sources]]
id = 3
name = "source-3"
type = "psd2"
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        // bind is None → auto-allocate from port_base_data (6000) + id (3)
        assert!(source.bind.is_none());
        assert_eq!(source.data_address(6000), "tcp://*:6003");
        assert_eq!(source.command_address_with_base(6100), "tcp://*:6103");
    }

    #[test]
    fn explicit_bind_overrides_auto() {
        let toml = r#"
[network]
[[network.sources]]
id = 3
name = "source-3"
type = "psd2"
bind = "tcp://*:9999"
command = "tcp://*:8888"
"#;
        let config = Config::from_toml(toml).unwrap();
        let source = &config.network.sources[0];

        // Explicit bind/command override auto-allocation
        assert_eq!(source.data_address(6000), "tcp://*:9999");
        assert_eq!(source.command_address_with_base(6100), "tcp://*:8888");
    }

    #[test]
    fn custom_port_bases() {
        let toml = r#"
[network]
port_base_data = 7000
port_base_command = 7100

[[network.sources]]
id = 5
name = "source-5"
type = "psd2"
host = "192.168.1.10"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.network.port_base_data, 7000);
        assert_eq!(config.network.port_base_command, 7100);

        let source = &config.network.sources[0];
        assert_eq!(
            source.data_address(config.network.port_base_data),
            "tcp://*:7005"
        );
        assert_eq!(
            source.data_connect_address_with_base(config.network.port_base_data),
            "tcp://192.168.1.10:7005"
        );
        assert_eq!(
            source.command_connect_address_with_base(config.network.port_base_command),
            "tcp://192.168.1.10:7105"
        );
    }

    #[test]
    fn merger_auto_subscribe_with_auto_ports() {
        let toml = r#"
[network]
port_base_data = 7000

[[network.sources]]
id = 0
name = "src-0"
type = "psd2"
host = "192.168.1.10"

[[network.sources]]
id = 1
name = "src-1"
type = "psd1"
host = "192.168.1.20"

[network.merger]
publish = "tcp://*:5557"
"#;
        let config = Config::from_toml(toml).unwrap();
        let subs = config.resolved_merger_subscribe();

        // Auto-resolved from port_base_data (7000) + source id + host
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0], "tcp://192.168.1.10:7000");
        assert_eq!(subs[1], "tcp://192.168.1.20:7001");
    }

    #[test]
    fn default_port_bases() {
        let toml = r#"
[network]
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.network.port_base_data, 6000);
        assert_eq!(config.network.port_base_command, 6100);
    }
}

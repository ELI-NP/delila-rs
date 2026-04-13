//! Reader module for digitizer data acquisition
//!
//! This module provides:
//! - CAEN digitizer FFI bindings (caen)
//! - Data decoders (decoder)
//! - Reader integration with two-task architecture

pub mod caen;
#[cfg(feature = "x743")]
pub mod caen_legacy;
pub mod decoder;

// Re-exports
pub use crate::config::FirmwareType;
pub use caen::{CaenError, CaenHandle, EndpointHandle, OpenDppEvent};
pub use decoder::{
    AMaxConfig, AMaxDecoder, DataType, DecodeResult, EventData, Pha1Config, Pha1Decoder,
    Psd1Config, Psd1Decoder, Psd2Config, Psd2Decoder, Waveform,
};

use crate::common::{
    handle_command, run_command_task, CommandHandlerExt, ComponentSharedState, ComponentState,
    EventData as CommonEventData, EventDataBatch, Message, Waveform as CommonWaveform,
};
use futures::SinkExt;
use rand::Rng;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tmq::Context;
use tmq::{publish, AsZmqSocket};
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Reader error type
#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("CAEN error: {0}")]
    Caen(#[from] CaenError),

    #[error("ZMQ error: {0}")]
    Zmq(#[from] tmq::TmqError),

    #[error("MessagePack serialization error: {0}")]
    MsgPack(#[from] rmp_serde::encode::Error),

    #[error("Decode error: {0}")]
    Decode(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Channel send error")]
    ChannelSend,
}

/// Internal message type from ReadLoop to DecodeLoop
///
/// Supports both RAW data (requiring decoding) and pre-decoded events (from OpenDPP).
enum ReadLoopOutput {
    /// Raw data that needs decoding (PSD1/PSD2/PHA1)
    Raw(decoder::RawData),
    /// Already decoded event from OpenDPP (AMax)
    Decoded(decoder::EventData),
    /// Start signal from digitizer (reserved for future use)
    #[allow(dead_code)]
    Start,
    /// Stop signal — triggers EOS publication in DecodeLoop
    Stop,
}

/// Convert OpenDppEvent to decoder::EventData
///
/// AMax firmware uses OpenDPP endpoint which returns pre-decoded event data.
/// This converts the OpenDPP event structure to our common EventData format.
fn opendpp_to_event_data(event: &OpenDppEvent, module_id: u8) -> decoder::EventData {
    // AMax timestamp: 1 LSB = 8 ns
    // Fine timestamp adds sub-clock resolution (10-bit, scale by 1024)
    const TIME_STEP_NS: f64 = 8.0;
    const FINE_TIME_SCALE: f64 = 1024.0;

    let coarse_time_ns = (event.timestamp as f64) * TIME_STEP_NS;
    let fine_time_ns = (event.fine_timestamp as f64 / FINE_TIME_SCALE) * TIME_STEP_NS;
    let timestamp_ns = coarse_time_ns + fine_time_ns;

    // Combine flags (flags_a is 8 bits, flags_b is 12 bits)
    let flags = ((event.flags_a as u32) << 12) | (event.flags_b as u32);

    decoder::EventData {
        timestamp_ns,
        module: module_id,
        channel: event.channel,
        energy: event.energy,
        energy_short: event.psd, // PSD value stored in energy_short
        fine_time: event.fine_timestamp,
        flags,
        waveform: None, // AMax OpenDPP without waveform for now
    }
}

/// Enum-based decoder dispatch (KISS: PSD1/PSD2/PHA1/AMax, no trait object needed)
enum DecoderKind {
    Psd2(Psd2Decoder),
    Psd1(Psd1Decoder),
    Pha1(Pha1Decoder),
    AMax(AMaxDecoder),
}

impl DecoderKind {
    fn classify(&self, raw: &decoder::RawData) -> DataType {
        match self {
            Self::Psd2(d) => d.classify(raw),
            Self::Psd1(d) => d.classify(raw),
            Self::Pha1(d) => d.classify(raw),
            Self::AMax(d) => d.classify(raw),
        }
    }

    fn decode_into(&mut self, raw: &decoder::RawData, events: &mut Vec<decoder::EventData>) {
        match self {
            Self::Psd2(d) => d.decode_into(raw, events),
            Self::Psd1(d) => d.decode_into(raw, events),
            Self::Pha1(d) => d.decode_into(raw, events),
            Self::AMax(d) => {
                // AMax decoder returns AMaxEventData, extract base EventData
                let mut amax_events = Vec::new();
                d.decode_into(raw, &mut amax_events);
                events.extend(amax_events.into_iter().map(|e| e.base));
            }
        }
    }

    /// Reset decoder state for a new run (SW Fine TS rollover tracking)
    fn reset_for_new_run(&mut self) {
        match self {
            Self::Psd1(d) => d.reset_for_new_run(),
            Self::Pha1(d) => d.reset_for_new_run(),
            Self::Psd2(_) | Self::AMax(_) => {} // No run-level state to reset
        }
    }
}

/// Reader configuration
#[derive(Debug, Clone)]
pub struct ReaderConfig {
    /// Device URL (e.g., "dig2://172.18.4.56")
    pub url: String,
    /// ZMQ data publish address
    pub data_address: String,
    /// ZMQ command address (REP socket)
    pub command_address: String,
    /// Source ID for this reader
    pub source_id: u32,
    /// Firmware type (determines decoder)
    pub firmware: FirmwareType,
    /// Module ID for decoded events
    pub module_id: u8,
    /// Read timeout in milliseconds
    pub read_timeout_ms: i32,
    /// Buffer size for raw data reads
    pub buffer_size: usize,
    /// Heartbeat interval in milliseconds (0 = disabled)
    pub heartbeat_interval_ms: u64,
    /// Time step in nanoseconds (for timestamp calculation)
    pub time_step_ns: f64,
    /// Path to digitizer configuration JSON file (optional)
    pub config_file: Option<String>,
    /// Minimum ADC value filter. Events with energy < adc_min are discarded.
    /// 0 = no filtering (default).
    pub adc_min: u16,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            url: "dig2://localhost".to_string(),
            data_address: "tcp://*:5555".to_string(),
            command_address: "tcp://*:5556".to_string(),
            source_id: 0,
            firmware: FirmwareType::PSD2,
            module_id: 0,
            read_timeout_ms: 100,
            buffer_size: 64 * 1024 * 1024, // 64MB - CAEN FELib has no bounds check
            heartbeat_interval_ms: 1000,
            time_step_ns: 2.0, // 500 MHz ADC = 2ns per sample
            config_file: None,
            adc_min: 0,
        }
    }
}

impl ReaderConfig {
    /// Create ReaderConfig from Config and source ID
    ///
    /// Returns None if source_id is not found or source has no digitizer_url
    pub fn from_config(config: &crate::config::Config, source_id: u32) -> Option<Self> {
        let source = config.get_source(source_id)?;
        let url = source.digitizer_url.as_ref()?;

        let firmware = match source.source_type {
            crate::config::SourceType::Psd2 => FirmwareType::PSD2,
            crate::config::SourceType::Psd1 => FirmwareType::PSD1,
            crate::config::SourceType::Pha1 => FirmwareType::PHA1,
            crate::config::SourceType::AMax => FirmwareType::AMax,
            crate::config::SourceType::X743CI => FirmwareType::X743CI,
            crate::config::SourceType::X743Std => FirmwareType::X743Std,
            // Emulator/Zle sources shouldn't create a Reader — caller should handle
            _ => return None,
        };

        Some(Self {
            url: url.clone(),
            data_address: source.data_address(config.network.port_base_data),
            command_address: source.command_address_with_base(config.network.port_base_command),
            source_id,
            firmware,
            module_id: source.module_id.unwrap_or(source_id as u8),
            read_timeout_ms: 100,
            buffer_size: 64 * 1024 * 1024, // 64MB - CAEN FELib has no bounds check
            heartbeat_interval_ms: 1000,
            time_step_ns: source.time_step_ns.unwrap_or(2.0),
            config_file: source.config_file.clone(),
            adc_min: source.adc_min,
        })
    }
}

/// Metrics for monitoring
/// Maximum channels per digitizer (DT5725S = 32ch, DT5730B = 16ch)
pub const MAX_CHANNELS: usize = 32;

#[derive(Debug)]
pub struct ReaderMetrics {
    /// Total events decoded
    pub events_decoded: AtomicU64,
    /// Total bytes read from digitizer
    pub bytes_read: AtomicU64,
    /// Total batches published
    pub batches_published: AtomicU64,
    /// Current decode queue length (approximate)
    pub queue_length: AtomicU64,
    /// Cumulative trigger loss count (DIG1: flag-based estimate, DIG2: counter-based exact)
    pub trigger_loss_count: AtomicU64,
    /// Events with trigger_lost flag set (DIG1 only)
    pub trigger_lost_flag_events: AtomicU64,
    /// Events with n_lost_trigger flag set (DIG1 only)
    pub n_lost_trigger_flag_events: AtomicU64,
    /// Per-channel cumulative event counts (index = channel number)
    pub per_channel_counts: [AtomicU64; MAX_CHANNELS],
    /// Events filtered out by adc_min threshold
    pub filtered_events: AtomicU64,
}

impl Default for ReaderMetrics {
    fn default() -> Self {
        Self {
            events_decoded: AtomicU64::new(0),
            bytes_read: AtomicU64::new(0),
            batches_published: AtomicU64::new(0),
            queue_length: AtomicU64::new(0),
            trigger_loss_count: AtomicU64::new(0),
            trigger_lost_flag_events: AtomicU64::new(0),
            n_lost_trigger_flag_events: AtomicU64::new(0),
            per_channel_counts: std::array::from_fn(|_| AtomicU64::new(0)),
            filtered_events: AtomicU64::new(0),
        }
    }
}

/// Rate tracker for 1-second interval rate calculation
#[derive(Debug)]
struct RateTracker {
    prev_events: AtomicU64,
    prev_time: std::sync::Mutex<Option<Instant>>,
    current_rate: AtomicU64,
}

impl RateTracker {
    fn new() -> Self {
        Self {
            prev_events: AtomicU64::new(0),
            prev_time: std::sync::Mutex::new(None),
            current_rate: AtomicU64::new(0),
        }
    }

    fn update(&self, current_events: u64) {
        let now = Instant::now();
        let mut prev_time_guard = self.prev_time.lock().unwrap();

        if let Some(prev_time) = *prev_time_guard {
            let elapsed = now.duration_since(prev_time).as_secs_f64();
            if elapsed >= 1.0 {
                let prev_events = self.prev_events.load(Ordering::Relaxed);
                let delta = current_events.saturating_sub(prev_events);
                let rate = (delta as f64 / elapsed) as u64;
                self.current_rate.store(rate, Ordering::Relaxed);
                self.prev_events.store(current_events, Ordering::Relaxed);
                *prev_time_guard = Some(now);
            }
        } else {
            self.prev_events.store(current_events, Ordering::Relaxed);
            *prev_time_guard = Some(now);
        }
    }

    fn get_rate(&self) -> f64 {
        self.current_rate.load(Ordering::Relaxed) as f64
    }

    fn reset(&self) {
        self.prev_events.store(0, Ordering::Relaxed);
        self.current_rate.store(0, Ordering::Relaxed);
        *self.prev_time.lock().unwrap() = None;
    }
}

/// Request from command handler to read_loop.
/// Delegates hardware operations to the read_loop's existing CaenHandle
/// to avoid opening multiple FELib connections.
enum ReadLoopRequest {
    /// Detect: read device info from hardware
    Detect {
        response_tx: std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>,
    },
    /// Apply digitizer configuration to hardware
    ApplyConfig {
        config: Box<crate::config::digitizer::DigitizerConfig>,
        response_tx: std::sync::mpsc::SyncSender<Result<usize, String>>,
    },
    /// Apply only SetInRun parameters while running
    ApplyConfigRunning {
        config: Box<crate::config::digitizer::DigitizerConfig>,
        response_tx: std::sync::mpsc::SyncSender<Result<usize, String>>,
    },
}

/// Command handler extension for Reader
struct ReaderCommandExt {
    metrics: Arc<ReaderMetrics>,
    rate_tracker: Arc<RateTracker>,
    /// Channel to delegate hardware requests to the read_loop's existing CaenHandle
    request_tx: std::sync::mpsc::Sender<ReadLoopRequest>,
    /// Hardware-confirmed state (updated by ReadLoop after actual HW transitions).
    /// GetStatus reports the minimum of software state and this value so that
    /// the Operator doesn't proceed until hardware is truly ready.
    hw_state: Arc<std::sync::Mutex<ComponentState>>,
}

impl CommandHandlerExt for ReaderCommandExt {
    fn component_name(&self) -> &'static str {
        "Reader"
    }

    fn status_details(&self) -> Option<String> {
        let events = self.metrics.events_decoded.load(Ordering::Relaxed);
        let batches = self.metrics.batches_published.load(Ordering::Relaxed);
        let bytes = self.metrics.bytes_read.load(Ordering::Relaxed);
        Some(format!(
            "Events: {}, Batches: {}, Bytes: {}",
            events, batches, bytes
        ))
    }

    fn get_metrics(&self) -> Option<crate::common::ComponentMetrics> {
        let events = self.metrics.events_decoded.load(Ordering::Relaxed);
        let bytes = self.metrics.bytes_read.load(Ordering::Relaxed);
        let queue = self.metrics.queue_length.load(Ordering::Relaxed);
        let trigger_loss = self.metrics.trigger_loss_count.load(Ordering::Relaxed);
        self.rate_tracker.update(events);
        let loss_rate = if events > 0 {
            (trigger_loss as f64 / (events as f64 + trigger_loss as f64)) * 100.0
        } else {
            0.0
        };
        Some(crate::common::ComponentMetrics {
            events_processed: events,
            bytes_transferred: bytes,
            queue_size: queue as u32,
            queue_max: 0,
            event_rate: self.rate_tracker.get_rate(),
            data_rate: 0.0,
            trigger_loss_count: trigger_loss,
            trigger_loss_rate: loss_rate,
            channel_counts: Some(
                self.metrics
                    .per_channel_counts
                    .iter()
                    .map(|c| c.load(Ordering::Relaxed))
                    .collect(),
            ),
        })
    }

    fn effective_state(&self, software_state: ComponentState) -> ComponentState {
        let hw = *self.hw_state.lock().unwrap();
        // Report the lesser of software and hardware state so Operator waits
        // until hardware actually reaches the target state.
        if state_rank(hw) < state_rank(software_state) {
            hw
        } else {
            software_state
        }
    }

    fn on_start(&mut self, _run_number: u32) -> Result<(), String> {
        self.rate_tracker.reset();
        // Reset all metrics for new run
        self.metrics.events_decoded.store(0, Ordering::Relaxed);
        self.metrics.bytes_read.store(0, Ordering::Relaxed);
        self.metrics.batches_published.store(0, Ordering::Relaxed);
        self.metrics.trigger_loss_count.store(0, Ordering::Relaxed);
        self.metrics
            .trigger_lost_flag_events
            .store(0, Ordering::Relaxed);
        self.metrics
            .n_lost_trigger_flag_events
            .store(0, Ordering::Relaxed);
        self.metrics.filtered_events.store(0, Ordering::Relaxed);
        for ch in &self.metrics.per_channel_counts {
            ch.store(0, Ordering::Relaxed);
        }
        Ok(())
    }

    fn on_detect(&mut self) -> Result<serde_json::Value, String> {
        // Delegate to read_loop which owns the CaenHandle.
        // This avoids opening a second FELib connection that would
        // interfere with the existing one.
        let (resp_tx, resp_rx) = std::sync::mpsc::sync_channel(1);
        self.request_tx
            .send(ReadLoopRequest::Detect {
                response_tx: resp_tx,
            })
            .map_err(|_| "ReadLoop not running".to_string())?;
        resp_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "Detect timeout: ReadLoop did not respond".to_string())?
    }

    fn on_apply_digitizer_config(
        &mut self,
        config: &crate::config::digitizer::DigitizerConfig,
    ) -> Result<usize, String> {
        let (resp_tx, resp_rx) = std::sync::mpsc::sync_channel(1);
        self.request_tx
            .send(ReadLoopRequest::ApplyConfig {
                config: Box::new(config.clone()),
                response_tx: resp_tx,
            })
            .map_err(|_| "ReadLoop not running".to_string())?;
        // 10s timeout: USB digitizers (DT5730B) can be slow
        resp_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "ApplyConfig timeout: ReadLoop did not respond within 10s".to_string())?
    }

    fn on_apply_digitizer_config_running(
        &mut self,
        config: &crate::config::digitizer::DigitizerConfig,
    ) -> Result<usize, String> {
        let (resp_tx, resp_rx) = std::sync::mpsc::sync_channel(1);
        self.request_tx
            .send(ReadLoopRequest::ApplyConfigRunning {
                config: Box::new(config.clone()),
                response_tx: resp_tx,
            })
            .map_err(|_| "ReadLoop not running".to_string())?;
        resp_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| {
                "ApplyConfigRunning timeout: ReadLoop did not respond within 10s".to_string()
            })?
    }
}

/// Bundles CaenHandle + EndpointHandle + hardware state tracking.
///
/// When dropped, endpoint is dropped before handle (Rust struct field drop order),
/// ensuring the endpoint is released before the connection is closed.
struct DeviceConnection {
    handle: CaenHandle,
    endpoint: EndpointHandle,
    /// Whether digitizer config has been applied since connection
    hw_configured: bool,
    /// Whether digitizer has been armed
    hw_armed: bool,
    /// Whether acquisition is running
    hw_running: bool,
    /// Auto-configure from JSON file failed — block Arm until Operator sends valid config
    auto_config_failed: bool,
    /// Cached DevTree parameter metadata for validation (None if fetch failed)
    param_cache: Option<std::collections::HashMap<String, caen::handle::ParamInfo>>,
    /// Enabled channel indices (for DIG2 counter polling)
    enabled_channels: Vec<u8>,
}

/// Try to connect to a digitizer and configure the RAW endpoint.
/// Returns None on failure (non-fatal — ReadLoop stays alive).
fn try_connect_raw(url: &str, include_n_events: bool) -> Option<DeviceConnection> {
    match CaenHandle::open(url) {
        Ok(h) => match h.configure_endpoint(include_n_events) {
            Ok(ep) => {
                info!("Connected to digitizer (RAW endpoint)");
                // Build param cache from DevTree (best-effort)
                let param_cache = match h.build_param_cache() {
                    Ok(cache) => {
                        info!(params = cache.len(), "Parameter cache built");
                        Some(cache)
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to build param cache, validation disabled");
                        None
                    }
                };
                Some(DeviceConnection {
                    handle: h,
                    endpoint: ep,
                    hw_configured: false,
                    hw_armed: false,
                    hw_running: false,
                    auto_config_failed: false,
                    param_cache,
                    enabled_channels: Vec::new(),
                })
            }
            Err(e) => {
                error!(error = %e, "Connected but endpoint configuration failed");
                None // h drops here → CAEN_FELib_Close
            }
        },
        Err(e) => {
            warn!(error = %e, "Failed to connect to digitizer");
            None
        }
    }
}

/// Try to connect to a digitizer and configure the OpenDPP endpoint.
/// Returns None on failure (non-fatal — ReadLoop stays alive).
fn try_connect_opendpp(url: &str) -> Option<DeviceConnection> {
    match CaenHandle::open(url) {
        Ok(h) => match h.configure_opendpp_endpoint(false) {
            Ok(ep) => {
                info!("Connected to digitizer (OpenDPP endpoint)");
                // Build param cache from DevTree (best-effort)
                let param_cache = match h.build_param_cache() {
                    Ok(cache) => {
                        info!(params = cache.len(), "Parameter cache built");
                        Some(cache)
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to build param cache, validation disabled");
                        None
                    }
                };
                Some(DeviceConnection {
                    handle: h,
                    endpoint: ep,
                    hw_configured: false,
                    hw_armed: false,
                    hw_running: false,
                    auto_config_failed: false,
                    param_cache,
                    enabled_channels: Vec::new(),
                })
            }
            Err(e) => {
                error!(error = %e, "Connected but OpenDPP endpoint configuration failed");
                None
            }
        },
        Err(e) => {
            warn!(error = %e, "Failed to connect to digitizer");
            None
        }
    }
}

/// Extract enabled channel indices from a DigitizerConfig.
fn get_enabled_channels_from_config(config: &crate::config::digitizer::DigitizerConfig) -> Vec<u8> {
    let default_enabled = config
        .channel_defaults
        .enabled
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let mut enabled = Vec::new();
    for ch in 0..config.num_channels {
        let ch_enabled = config
            .channel_overrides
            .get(&ch)
            .and_then(|c| c.enabled.as_deref())
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(default_enabled);
        if ch_enabled {
            enabled.push(ch);
        }
    }
    enabled
}

/// 24-bit counter wraparound-aware difference (for DIG2 FPGA counters).
fn wrapping_diff_24bit(current: u64, prev: u64) -> u64 {
    if current >= prev {
        current - prev
    } else {
        current + 0x100_0000 - prev
    }
}

/// DIG2 trigger counter polling state (tracks across poll intervals for wraparound handling).
struct Dig2PollState {
    prev_trigger: Vec<u64>,
    prev_saved: Vec<u64>,
    accumulated_lost: u64,
    accumulated_trigger: u64,
    initialized: bool,
}

impl Dig2PollState {
    fn new() -> Self {
        Self {
            prev_trigger: Vec::new(),
            prev_saved: Vec::new(),
            accumulated_lost: 0,
            accumulated_trigger: 0,
            initialized: false,
        }
    }

    fn reset(&mut self) {
        self.prev_trigger.clear();
        self.prev_saved.clear();
        self.accumulated_lost = 0;
        self.accumulated_trigger = 0;
        self.initialized = false;
    }
}

/// Poll DIG2 trigger counters and update metrics.
/// Must only be called for DIG2 firmware during Running state.
fn poll_dig2_counters(
    conn: &DeviceConnection,
    poll: &mut Dig2PollState,
    metrics: &ReaderMetrics,
    last_warn: &mut Instant,
) {
    if conn.enabled_channels.is_empty() {
        return;
    }

    // Initialize prev vectors if needed
    if !poll.initialized {
        let n = conn.enabled_channels.len();
        poll.prev_trigger = vec![0; n];
        poll.prev_saved = vec![0; n];
        // Read initial baseline values
        for (i, &ch) in conn.enabled_channels.iter().enumerate() {
            // ChRealtimeMonitor must be read first to latch FPGA counters
            let _ = conn
                .handle
                .get_value(&format!("/ch/{}/par/ChRealtimeMonitor", ch));
            poll.prev_trigger[i] = conn
                .handle
                .get_value(&format!("/ch/{}/par/ChTriggerCnt", ch))
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            poll.prev_saved[i] = conn
                .handle
                .get_value(&format!("/ch/{}/par/ChSavedEventCnt", ch))
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
        }
        poll.initialized = true;
        return;
    }

    for (i, &ch) in conn.enabled_channels.iter().enumerate() {
        // ChRealtimeMonitor must be read first to latch FPGA counters
        let _ = conn
            .handle
            .get_value(&format!("/ch/{}/par/ChRealtimeMonitor", ch));
        let trigger = conn
            .handle
            .get_value(&format!("/ch/{}/par/ChTriggerCnt", ch))
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let saved = conn
            .handle
            .get_value(&format!("/ch/{}/par/ChSavedEventCnt", ch))
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        let delta_trigger = wrapping_diff_24bit(trigger, poll.prev_trigger[i]);
        let delta_saved = wrapping_diff_24bit(saved, poll.prev_saved[i]);
        poll.accumulated_trigger += delta_trigger;
        poll.accumulated_lost += delta_trigger.saturating_sub(delta_saved);

        poll.prev_trigger[i] = trigger;
        poll.prev_saved[i] = saved;
    }

    metrics
        .trigger_loss_count
        .store(poll.accumulated_lost, Ordering::Relaxed);

    if poll.accumulated_lost > 0 && last_warn.elapsed() >= Duration::from_secs(10) {
        let rate = if poll.accumulated_trigger > 0 {
            poll.accumulated_lost as f64 / poll.accumulated_trigger as f64 * 100.0
        } else {
            0.0
        };
        warn!(
            total_trigger = poll.accumulated_trigger,
            total_lost = poll.accumulated_lost,
            loss_rate_pct = format!("{:.2}", rate),
            "Trigger loss detected (DIG2 counters)"
        );
        *last_warn = Instant::now();
    }
}

/// Map ComponentState to a rank for ordering comparisons.
/// Transitional/Error states map to 0 (treated as Idle).
fn state_rank(s: ComponentState) -> u8 {
    match s {
        ComponentState::Idle => 0,
        ComponentState::Configured => 1,
        ComponentState::Armed => 2,
        ComponentState::Running => 3,
        _ => 0,
    }
}

/// Reconnection backoff parameters.
/// Exponential backoff (1s→2s→4s→8s→16s→max 30s) + random jitter (±500ms)
/// prevents Thundering Herd when multiple readers reconnect simultaneously
/// after an optical link failure.
const RECONNECT_INITIAL: Duration = Duration::from_millis(1000);
const RECONNECT_MAX: Duration = Duration::from_millis(30000);
const RECONNECT_JITTER_MS: u64 = 500;

/// Compute next reconnect cooldown with exponential backoff + jitter.
/// Returns the jittered cooldown and the next (doubled) base for the caller to store.
fn next_reconnect_cooldown(current_base: Duration) -> (Duration, Duration) {
    let jitter_ms = rand::thread_rng().gen_range(0..=RECONNECT_JITTER_MS * 2);
    let jittered = current_base
        .checked_add(Duration::from_millis(jitter_ms))
        .unwrap_or(RECONNECT_MAX)
        .min(RECONNECT_MAX + Duration::from_millis(RECONNECT_JITTER_MS));
    let next_base = (current_base * 2).min(RECONNECT_MAX);
    (jittered, next_base)
}

/// Send firmware-specific arm command to the digitizer.
///
/// For DIG1 (PSD1/PHA) with START_MODE_SW, the actual arm is deferred to start phase.
/// For DIG2 (PSD2), always sends armacquisition immediately.
fn send_arm_command(handle: &CaenHandle, firmware: FirmwareType) -> Result<(), caen::CaenError> {
    if firmware.is_dig1() {
        let startmode = handle.get_value("/par/startmode").unwrap_or_default();
        if startmode == "START_MODE_SW" {
            info!("START_MODE_SW detected - deferring arm to Start");
        } else {
            info!("Arming digitizer (DIG1, mode={})", startmode);
            handle.send_command("/cmd/armacquisition")?;
        }
    } else {
        info!("Arming digitizer (PSD2)");
        handle.send_command("/cmd/armacquisition")?;
    }
    Ok(())
}

/// Send firmware-specific start command to the digitizer.
///
/// For DIG2 (PSD2), sends swstartacquisition.
/// For DIG1 (PSD1/PHA) with START_MODE_SW, sends armacquisition (arm=start).
fn send_start_command(handle: &CaenHandle, firmware: FirmwareType) -> Result<(), caen::CaenError> {
    if firmware.is_dig1() {
        let startmode = handle.get_value("/par/startmode").unwrap_or_default();
        if startmode == "START_MODE_SW" {
            info!("Starting acquisition (DIG1, START_MODE_SW)");
            handle.send_command("/cmd/armacquisition")?;
        } else {
            info!("DIG1 acquisition already started on Arm");
        }
    } else {
        info!("Starting digitizer acquisition (PSD2)");
        handle.send_command("/cmd/swstartacquisition")?;
    }
    Ok(())
}

/// Reader for CAEN digitizer data acquisition
///
/// Uses two-task architecture:
/// - ReadLoop: Blocking reads from CAEN hardware (spawn_blocking)
/// - DecodeLoop: Async decoding and ZMQ publishing
pub struct Reader {
    config: ReaderConfig,
    data_socket: publish::Publish,
    shared_state: Arc<Mutex<ComponentSharedState>>,
    state_rx: watch::Receiver<ComponentState>,
    state_tx: watch::Sender<ComponentState>,
    metrics: Arc<ReaderMetrics>,
    rate_tracker: Arc<RateTracker>,
}

impl Reader {
    /// Create a new Reader with the given configuration
    pub async fn new(config: ReaderConfig) -> Result<Self, ReaderError> {
        let context = Context::new();
        let data_socket = publish(&context).bind(&config.data_address)?;
        // Never drop messages — buffer in memory instead (DAQ: no data loss)
        data_socket
            .get_socket()
            .set_sndhwm(0)
            .map_err(|e| ReaderError::Zmq(e.into()))?;

        info!(
            data_address = %config.data_address,
            command_address = %config.command_address,
            url = %config.url,
            "Reader bound to data address (SNDHWM=0)"
        );

        let (state_tx, state_rx) = watch::channel(ComponentState::Idle);

        Ok(Self {
            config,
            data_socket,
            shared_state: Arc::new(Mutex::new(ComponentSharedState::new())),
            state_rx,
            state_tx,
            metrics: Arc::new(ReaderMetrics::default()),
            rate_tracker: Arc::new(RateTracker::new()),
        })
    }

    /// Get current state
    pub fn state(&self) -> ComponentState {
        *self.state_rx.borrow()
    }

    /// Get metrics
    pub fn metrics(&self) -> &Arc<ReaderMetrics> {
        &self.metrics
    }

    /// Remap DIG1 (PSD1/PHA1) raw hardware flags to common flag constants.
    ///
    /// Raw decoder flags come from EXTRAS word bits[15:10] shifted to bits[5:0],
    /// plus pileup at bit[15] from the charge/energy word.
    fn remap_dig1_flags(raw: u32) -> u64 {
        use crate::common::flags::*;
        let mut out: u64 = 0;
        if raw & (1 << 15) != 0 {
            out |= FLAG_PILEUP;
        } // Pileup from charge word
        if raw & (1 << 5) != 0 {
            out |= FLAG_TRIGGER_LOST;
        } // EXTRAS bit[15]
        if raw & (1 << 4) != 0 {
            out |= FLAG_OVER_RANGE;
        } // EXTRAS bit[14]
        if raw & (1 << 3) != 0 {
            out |= FLAG_1024_TRIGGER;
        } // EXTRAS bit[13]
        if raw & (1 << 2) != 0 {
            out |= FLAG_N_LOST_TRIGGER;
        } // EXTRAS bit[12]
        out
    }

    /// Convert EventData to CommonEventData (consumes event, zero-copy for waveforms)
    fn convert_event(event: EventData, firmware: FirmwareType) -> CommonEventData {
        let flags = if firmware.is_dig1() {
            Self::remap_dig1_flags(event.flags)
        } else {
            event.flags as u64
        };

        if let Some(wf) = event.waveform {
            CommonEventData::with_waveform(
                event.module,
                event.channel,
                event.energy,
                event.energy_short,
                event.timestamp_ns,
                flags,
                CommonWaveform {
                    analog_probe1: wf.analog_probe1,   // move, not clone
                    analog_probe2: wf.analog_probe2,   // move
                    digital_probe1: wf.digital_probe1, // move
                    digital_probe2: wf.digital_probe2, // move
                    digital_probe3: wf.digital_probe3, // move
                    digital_probe4: wf.digital_probe4, // move
                    time_resolution: wf.time_resolution,
                    trigger_threshold: wf.trigger_threshold,
                    ns_per_sample: wf.ns_per_sample,
                },
            )
        } else {
            CommonEventData::new(
                event.module,
                event.channel,
                event.energy,
                event.energy_short,
                event.timestamp_ns,
                flags,
            )
        }
    }

    /// Publish a message via ZMQ
    async fn publish_message(&mut self, message: &Message) -> Result<(), ReaderError> {
        let bytes = message.to_msgpack()?;
        let msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
        self.data_socket.send(msg).await?;

        match message {
            Message::Data(batch) => {
                debug!(
                    seq = batch.sequence_number,
                    events = batch.len(),
                    "Published batch"
                );
                self.metrics
                    .batches_published
                    .fetch_add(1, Ordering::Relaxed);
            }
            Message::EndOfStream { source_id, .. } => {
                info!(source_id = source_id, "Published EOS");
            }
            Message::Heartbeat(hb) => {
                debug!(
                    source_id = hb.source_id,
                    counter = hb.counter,
                    "Published heartbeat"
                );
            }
        }

        Ok(())
    }

    /// Send EOS (End Of Stream) signal
    async fn send_eos(&mut self) -> Result<(), ReaderError> {
        let eos = Message::eos(self.config.source_id, 0);
        self.publish_message(&eos).await
    }

    /// ReadLoop task for RAW endpoint (PSD1/PSD2/PHA1) - runs in spawn_blocking
    ///
    /// Reads raw data from CAEN digitizer and sends to decode channel.
    /// Uses lazy connection: if the digitizer is not available at startup,
    /// the loop stays alive and retries connection on demand (Detect, Configure, etc.).
    #[allow(clippy::too_many_arguments)]
    fn read_loop_raw(
        config: ReaderConfig,
        tx: mpsc::Sender<ReadLoopOutput>,
        state_rx: watch::Receiver<ComponentState>,
        state_tx: watch::Sender<ComponentState>,
        metrics: Arc<ReaderMetrics>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
        request_rx: std::sync::mpsc::Receiver<ReadLoopRequest>,
        hw_state: Arc<std::sync::Mutex<ComponentState>>,
    ) -> Result<(), ReaderError> {
        info!(url = %config.url, "ReadLoop (RAW) starting");

        let include_n_events = config.firmware.includes_n_events();

        // Lazy connection: try initial connect (non-fatal)
        let mut connection = try_connect_raw(&config.url, include_n_events);
        let mut last_connect_attempt = Instant::now();
        let mut reconnect_backoff = RECONNECT_INITIAL;

        // Pre-allocate reusable read buffer.
        // CAEN FELib does NOT check buffer bounds — undersized buffers cause SIGBUS.
        let mut read_buffer: Vec<u8> = vec![0u8; config.buffer_size];
        info!(
            buffer_size = config.buffer_size,
            "ReadLoop buffer allocated"
        );

        // Track consecutive read errors for retry logic.
        // Optical link transients (e.g. A3818 RX timeout) are recoverable —
        // the digitizer keeps buffering data internally.
        let mut read_error_since: Option<Instant> = None;
        const READ_ERROR_TIMEOUT: Duration = Duration::from_secs(30);

        // DIG2 trigger counter polling state
        let mut dig2_poll = Dig2PollState::new();
        let mut last_dig2_poll = Instant::now();
        let mut last_dig2_warn = Instant::now();
        const DIG2_POLL_INTERVAL: Duration = Duration::from_secs(5);

        loop {
            // Check shutdown flag
            if shutdown.load(Ordering::Relaxed) {
                info!("ReadLoop received shutdown signal");
                break;
            }

            // --- Connection management: periodic retry with exponential backoff ---
            if connection.is_none() {
                let (cooldown, next_base) = next_reconnect_cooldown(reconnect_backoff);
                if last_connect_attempt.elapsed() > cooldown {
                    last_connect_attempt = Instant::now();
                    connection = try_connect_raw(&config.url, include_n_events);
                    if connection.is_some() {
                        info!("Reconnected successfully, resetting backoff");
                        reconnect_backoff = RECONNECT_INITIAL;
                    } else {
                        warn!(
                            backoff_ms = next_base.as_millis() as u64,
                            "Reconnect failed, increasing backoff"
                        );
                        reconnect_backoff = next_base;
                    }
                }
            }

            // Get target state from Operator
            let target_state = *state_rx.borrow();

            // --- Target state synchronization ---
            // Ensures hardware catches up to target state after (re)connection.
            if let Some(ref mut conn) = connection {
                let target_rank = state_rank(target_state);

                // Configure needed?
                if target_rank >= state_rank(ComponentState::Configured) && !conn.hw_configured {
                    // Reset digitizer to factory defaults first — ensures clean slate
                    // regardless of prior state (e.g. CoMPASS register changes)
                    match conn.handle.send_command("/cmd/reset") {
                        Ok(()) => info!("Digitizer reset to factory defaults"),
                        Err(e) => warn!(error = %e, "Digitizer reset failed (non-fatal)"),
                    }

                    // Re-configure endpoint after reset (/cmd/reset invalidates
                    // activeendpoint and data format — read_data returns DISABLED without this)
                    match conn.handle.configure_endpoint(include_n_events) {
                        Ok(ep) => {
                            conn.endpoint = ep;
                            info!("Endpoint reconfigured after reset");
                        }
                        Err(e) => error!(error = %e, "Failed to reconfigure endpoint after reset"),
                    }

                    if let Some(ref config_path) = config.config_file {
                        info!(path = %config_path, "Loading digitizer configuration");
                        match crate::config::digitizer::DigitizerConfig::load(config_path) {
                            Ok(dig_config) => match conn.handle.apply_config(&dig_config) {
                                Ok(count) => {
                                    info!(count, "Digitizer configuration applied");
                                }
                                Err(e) => {
                                    warn!(error = %e, "Auto-configure from JSON failed — \
                                        awaiting Operator ApplyDigitizerConfig");
                                    conn.auto_config_failed = true;
                                }
                            },
                            Err(e) => {
                                error!(error = %e, path = %config_path, "Failed to load config file");
                                // Mark as configured anyway — digitizer keeps its current settings
                            }
                        }
                    } else {
                        info!("No config_file specified, using current digitizer settings");
                    }

                    // ADC calibration (DIG1 only) — final Configure step, before
                    // marking hw_configured. Prevents Arm delay / S_IN race.
                    if config.firmware.is_dig1() {
                        match conn.handle.send_command("/cmd/calibrateadc") {
                            Ok(()) => info!("ADC calibration completed"),
                            Err(e) => warn!(error = %e, "ADC calibration failed (non-fatal)"),
                        }
                    }

                    conn.hw_configured = true;
                    *hw_state.lock().unwrap() = ComponentState::Configured;
                }

                // Arm needed?
                if target_rank >= state_rank(ComponentState::Armed) && !conn.hw_armed {
                    if conn.auto_config_failed {
                        warn!(
                            "Cannot arm: auto-configure from JSON failed and no valid \
                            config received from Operator. Run Detect or fix the JSON config."
                        );
                    } else {
                        if let Err(e) = send_arm_command(&conn.handle, config.firmware) {
                            error!(error = %e, "Failed to arm digitizer");
                        }
                        conn.hw_armed = true;
                        *hw_state.lock().unwrap() = ComponentState::Armed;
                    }
                }

                // Start needed?
                if target_rank >= state_rank(ComponentState::Running) && !conn.hw_running {
                    if let Err(e) = send_start_command(&conn.handle, config.firmware) {
                        error!(error = %e, "Failed to start acquisition");
                    }
                    conn.hw_running = true;
                    *hw_state.lock().unwrap() = ComponentState::Running;
                    // Reset DIG2 poll state for new run
                    dig2_poll.reset();
                }

                // Stop needed? (target dropped below Running)
                if target_rank < state_rank(ComponentState::Running) && conn.hw_running {
                    info!("Stopping digitizer acquisition");
                    let _ = conn.handle.send_command("/cmd/disarmacquisition");

                    // Drain remaining buffered data before clearing (with limits)
                    let mut drained = 0u64;
                    let drain_start = Instant::now();
                    const MAX_DRAIN_EVENTS: u64 = 1000;
                    const MAX_DRAIN_TIME: Duration = Duration::from_secs(1);
                    while let Ok(Some(raw)) = conn.endpoint.read_data(100, &mut read_buffer) {
                        drained += 1;
                        let decoder_raw = decoder::RawData::from(raw);
                        let _ = tx.try_send(ReadLoopOutput::Raw(decoder_raw));
                        if drained >= MAX_DRAIN_EVENTS || drain_start.elapsed() > MAX_DRAIN_TIME {
                            warn!(drained, "Drain limit reached, clearing remaining");
                            break;
                        }
                    }
                    if drained > 0 {
                        info!(drained, "Drained remaining data after stop");
                    }

                    // Send Stop signal with retry to guarantee EOS delivery
                    let stop_deadline = Instant::now() + Duration::from_secs(3);
                    let mut stop_signal = ReadLoopOutput::Stop;
                    loop {
                        match tx.try_send(stop_signal) {
                            Ok(()) => {
                                info!("Stop signal sent to decode pipeline");
                                break;
                            }
                            Err(mpsc::error::TrySendError::Full(returned)) => {
                                if Instant::now() > stop_deadline {
                                    error!("Failed to send Stop signal: channel full for 3s");
                                    break;
                                }
                                stop_signal = returned;
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                warn!("Decode channel closed, Stop signal not needed");
                                break;
                            }
                        }
                    }

                    let _ = conn.handle.send_command("/cmd/cleardata");
                    conn.hw_armed = false;
                    conn.hw_running = false;
                    read_error_since = None; // Clear stale error timer across runs
                    *hw_state.lock().unwrap() = ComponentState::Configured;
                }

                // Reset needed? (target is Idle, but we have armed/configured state)
                if target_state == ComponentState::Idle && (conn.hw_armed || conn.hw_configured) {
                    info!("Resetting digitizer");
                    let _ = conn.handle.send_command("/cmd/disarmacquisition");
                    let _ = conn.handle.send_command("/cmd/cleardata");
                    conn.hw_armed = false;
                    conn.hw_running = false;
                    conn.hw_configured = false;
                    conn.auto_config_failed = false;
                    read_error_since = None;
                    *hw_state.lock().unwrap() = ComponentState::Idle;
                }
            }

            // --- Handle requests from command handler (Detect / ApplyConfig) ---
            if let Ok(req) = request_rx.try_recv() {
                match req {
                    ReadLoopRequest::Detect { response_tx } => {
                        // Try to connect on-demand for Detect
                        if connection.is_none() {
                            connection = try_connect_raw(&config.url, include_n_events);
                            last_connect_attempt = Instant::now();
                        }
                        let result = match connection.as_ref() {
                            Some(conn) => conn
                                .handle
                                .get_device_info()
                                .map(|info| serde_json::to_value(&info).unwrap_or_default())
                                .map_err(|e| format!("Failed to read device info: {}", e)),
                            None => Err("Not connected to digitizer".to_string()),
                        };
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfig {
                        config: dig_config,
                        response_tx,
                    } => {
                        if connection.is_none() {
                            connection = try_connect_raw(&config.url, include_n_events);
                            last_connect_attempt = Instant::now();
                        }
                        let result = match connection.as_ref() {
                            Some(conn) => {
                                if let Some(ref cache) = conn.param_cache {
                                    conn.handle
                                        .apply_config_validated(&dig_config, cache)
                                        .map(|r| r.ok + r.adjusted)
                                        .map_err(|e| format!("Failed to apply config: {}", e))
                                } else {
                                    conn.handle
                                        .apply_config(&dig_config)
                                        .map_err(|e| format!("Failed to apply config: {}", e))
                                }
                            }
                            None => Err("Not connected to digitizer".to_string()),
                        };
                        if result.is_ok() {
                            if let Some(ref mut conn) = connection {
                                conn.auto_config_failed = false;
                                conn.enabled_channels =
                                    get_enabled_channels_from_config(&dig_config);
                            }
                        }
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfigRunning {
                        config: dig_config,
                        response_tx,
                    } => {
                        let result = match connection.as_ref() {
                            Some(conn) => {
                                if let Some(ref cache) = conn.param_cache {
                                    conn.handle
                                        .apply_config_running_validated(&dig_config, cache)
                                        .map(|r| r.ok + r.adjusted)
                                        .map_err(|e| {
                                            format!("Failed to apply SetInRun config: {}", e)
                                        })
                                } else {
                                    conn.handle.apply_config_running(&dig_config).map_err(|e| {
                                        format!("Failed to apply SetInRun config: {}", e)
                                    })
                                }
                            }
                            None => Err("Not connected to digitizer".to_string()),
                        };
                        let _ = response_tx.send(result);
                    }
                }
            }

            // --- Data reading (Running only) ---
            if target_state != ComponentState::Running {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }

            if let Some(ref conn) = connection {
                if !conn.hw_running {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }

                match conn
                    .endpoint
                    .read_data(config.read_timeout_ms, &mut read_buffer)
                {
                    Ok(Some(raw)) => {
                        if let Some(since) = read_error_since.take() {
                            info!(
                                elapsed_ms = since.elapsed().as_millis() as u64,
                                "Read recovered after transient error"
                            );
                        }
                        metrics
                            .bytes_read
                            .fetch_add(raw.size as u64, Ordering::Relaxed);

                        let decoder_raw = decoder::RawData::from(raw);
                        let output = ReadLoopOutput::Raw(decoder_raw);
                        metrics.queue_length.fetch_add(1, Ordering::Relaxed);

                        // Send to decode channel with back-pressure retry
                        let mut pending = output;
                        let mut channel_closed = false;
                        loop {
                            match tx.try_send(pending) {
                                Ok(()) => break,
                                Err(mpsc::error::TrySendError::Full(returned)) => {
                                    if shutdown.load(Ordering::Relaxed)
                                        || *state_rx.borrow() != ComponentState::Running
                                    {
                                        warn!("Dropping pending data during shutdown/stop");
                                        break;
                                    }
                                    pending = returned;
                                    std::thread::sleep(Duration::from_millis(1));
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    warn!("Decode channel closed, stopping read loop");
                                    channel_closed = true;
                                    break;
                                }
                            }
                        }
                        if channel_closed {
                            break;
                        }
                    }
                    Ok(None) => {
                        // Timeout - no data available, continue polling.
                        // Also clears error state: read_data call succeeded.
                        if let Some(since) = read_error_since.take() {
                            info!(
                                elapsed_ms = since.elapsed().as_millis() as u64,
                                "Read recovered (timeout) after transient error"
                            );
                        }
                    }
                    Err(e) => {
                        if e.code == caen::error::codes::STOP {
                            if shutdown.load(Ordering::Relaxed) {
                                info!("Received STOP signal during shutdown");
                                break;
                            }
                            info!("Received STOP signal from digitizer, waiting for state change");
                            continue;
                        }
                        if target_state == ComponentState::Running {
                            // Transient error during acquisition — retry instead of
                            // dropping connection. The digitizer continues buffering
                            // data internally; we just need to wait for the optical
                            // link / driver to recover.
                            if read_error_since.is_none() {
                                read_error_since = Some(Instant::now());
                                warn!(error = %e, code = e.code,
                                    "Read error during acquisition, will retry for {:?}",
                                    READ_ERROR_TIMEOUT);
                            }
                            if read_error_since.unwrap().elapsed() > READ_ERROR_TIMEOUT {
                                error!(
                                    timeout_secs = READ_ERROR_TIMEOUT.as_secs(),
                                    error = %e, code = e.code,
                                    "Read errors persisting, transitioning to Error"
                                );
                                let _ = state_tx.send(ComponentState::Error);
                                connection = None;
                                read_error_since = None;
                            } else {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                        } else {
                            // Not running — safe to reconnect
                            error!(error = %e, code = e.code, "Read error, dropping connection");
                            connection = None;
                        }
                    }
                }
            } else {
                // Running but no connection — wait for reconnect at loop top
                std::thread::sleep(Duration::from_millis(100));
            }

            // DIG2: Periodic trigger counter polling (separate borrow scope)
            if !config.firmware.is_dig1() && last_dig2_poll.elapsed() >= DIG2_POLL_INTERVAL {
                if let Some(ref conn) = connection {
                    if conn.hw_running {
                        poll_dig2_counters(conn, &mut dig2_poll, &metrics, &mut last_dig2_warn);
                    }
                }
                last_dig2_poll = Instant::now();
            }
        }

        // Cleanup
        if let Some(conn) = connection {
            if conn.hw_armed || conn.hw_running {
                let _ = conn.handle.send_command("/cmd/disarmacquisition");
            }
        }
        info!("ReadLoop (RAW) stopped");
        Ok(())
    }

    /// ReadLoop task for OpenDPP endpoint (AMax) - runs in spawn_blocking
    ///
    /// Reads pre-decoded event data from CAEN digitizer via OpenDPP endpoint.
    /// Each event is already decoded by the hardware, so no software decoding is needed.
    /// Used for AMax/DPP_OPEN firmware.
    /// Uses lazy connection: if the digitizer is not available at startup,
    /// the loop stays alive and retries connection on demand (Detect, Configure, etc.).
    #[allow(clippy::too_many_arguments)]
    fn read_loop_opendpp(
        config: ReaderConfig,
        tx: mpsc::Sender<ReadLoopOutput>,
        state_rx: watch::Receiver<ComponentState>,
        state_tx: watch::Sender<ComponentState>,
        metrics: Arc<ReaderMetrics>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
        request_rx: std::sync::mpsc::Receiver<ReadLoopRequest>,
        hw_state: Arc<std::sync::Mutex<ComponentState>>,
    ) -> Result<(), ReaderError> {
        info!(url = %config.url, "ReadLoop (OpenDPP) starting");

        // Lazy connection: try initial connect (non-fatal)
        let mut connection = try_connect_opendpp(&config.url);
        let mut last_connect_attempt = Instant::now();
        let mut reconnect_backoff = RECONNECT_INITIAL;

        // Buffer for user info words (FW caenlist max len = 1024)
        let mut user_info_buffer = [0u64; 1024];

        // Track consecutive read errors for retry logic (same as RAW loop)
        let mut read_error_since: Option<Instant> = None;
        const READ_ERROR_TIMEOUT: Duration = Duration::from_secs(30);

        loop {
            // Check shutdown flag
            if shutdown.load(Ordering::Relaxed) {
                info!("ReadLoop (OpenDPP) received shutdown signal");
                break;
            }

            // --- Connection management: periodic retry with exponential backoff ---
            if connection.is_none() {
                let (cooldown, next_base) = next_reconnect_cooldown(reconnect_backoff);
                if last_connect_attempt.elapsed() > cooldown {
                    last_connect_attempt = Instant::now();
                    connection = try_connect_opendpp(&config.url);
                    if connection.is_some() {
                        info!("Reconnected successfully, resetting backoff");
                        reconnect_backoff = RECONNECT_INITIAL;
                    } else {
                        warn!(
                            backoff_ms = next_base.as_millis() as u64,
                            "Reconnect failed, increasing backoff"
                        );
                        reconnect_backoff = next_base;
                    }
                }
            }

            // Get target state from Operator
            let target_state = *state_rx.borrow();

            // --- Target state synchronization ---
            // Ensures hardware catches up to target state after (re)connection.
            if let Some(ref mut conn) = connection {
                let target_rank = state_rank(target_state);

                // Configure needed?
                if target_rank >= state_rank(ComponentState::Configured) && !conn.hw_configured {
                    // Reset digitizer to factory defaults first — ensures clean slate
                    // regardless of prior state (e.g. CoMPASS register changes)
                    match conn.handle.send_command("/cmd/reset") {
                        Ok(()) => info!("Digitizer reset to factory defaults"),
                        Err(e) => warn!(error = %e, "Digitizer reset failed (non-fatal)"),
                    }

                    // Re-configure endpoint after reset (/cmd/reset invalidates
                    // activeendpoint and data format — read_data returns DISABLED without this)
                    match conn.handle.configure_opendpp_endpoint(false) {
                        Ok(ep) => {
                            conn.endpoint = ep;
                            info!("Endpoint reconfigured after reset");
                        }
                        Err(e) => error!(error = %e, "Failed to reconfigure endpoint after reset"),
                    }

                    if let Some(ref config_path) = config.config_file {
                        info!(path = %config_path, "Loading digitizer configuration");
                        match crate::config::digitizer::DigitizerConfig::load(config_path) {
                            Ok(dig_config) => match conn.handle.apply_config(&dig_config) {
                                Ok(count) => {
                                    info!(count, "Digitizer configuration applied");
                                }
                                Err(e) => {
                                    warn!(error = %e, "Auto-configure from JSON failed — \
                                        awaiting Operator ApplyDigitizerConfig");
                                    conn.auto_config_failed = true;
                                }
                            },
                            Err(e) => {
                                error!(error = %e, path = %config_path, "Failed to load config file");
                                // Mark as configured anyway — digitizer keeps its current settings
                            }
                        }
                    } else {
                        info!("No config_file specified, using current digitizer settings");
                    }

                    // ADC calibration (DIG1 only) — final Configure step, before
                    // marking hw_configured. Prevents Arm delay / S_IN race.
                    if config.firmware.is_dig1() {
                        match conn.handle.send_command("/cmd/calibrateadc") {
                            Ok(()) => info!("ADC calibration completed"),
                            Err(e) => warn!(error = %e, "ADC calibration failed (non-fatal)"),
                        }
                    }

                    conn.hw_configured = true;
                    *hw_state.lock().unwrap() = ComponentState::Configured;
                }

                // Arm needed?
                if target_rank >= state_rank(ComponentState::Armed) && !conn.hw_armed {
                    if conn.auto_config_failed {
                        warn!(
                            "Cannot arm: auto-configure from JSON failed and no valid \
                            config received from Operator. Run Detect or fix the JSON config."
                        );
                    } else {
                        if let Err(e) = send_arm_command(&conn.handle, config.firmware) {
                            error!(error = %e, "Failed to arm digitizer");
                        }
                        conn.hw_armed = true;
                        *hw_state.lock().unwrap() = ComponentState::Armed;
                    }
                }

                // Start needed?
                if target_rank >= state_rank(ComponentState::Running) && !conn.hw_running {
                    if let Err(e) = send_start_command(&conn.handle, config.firmware) {
                        error!(error = %e, "Failed to start acquisition");
                    }
                    conn.hw_running = true;
                    *hw_state.lock().unwrap() = ComponentState::Running;
                }

                // Stop needed? (target dropped below Running)
                if target_rank < state_rank(ComponentState::Running) && conn.hw_running {
                    info!("Stopping digitizer acquisition");
                    let _ = conn.handle.send_command("/cmd/disarmacquisition");
                    // Drain remaining buffered events before clearing
                    let mut drained = 0u64;
                    while let Ok(Some(evt)) =
                        conn.endpoint.read_opendpp_event(100, &mut user_info_buffer)
                    {
                        drained += 1;
                        let event_data = opendpp_to_event_data(&evt, config.module_id);
                        let _ = tx.try_send(ReadLoopOutput::Decoded(event_data));
                    }
                    if drained > 0 {
                        info!(drained, "Drained remaining events after stop");
                    }
                    let _ = tx.try_send(ReadLoopOutput::Stop);
                    let _ = conn.handle.send_command("/cmd/cleardata");
                    conn.hw_armed = false;
                    conn.hw_running = false;
                    read_error_since = None; // Clear stale error timer across runs
                    *hw_state.lock().unwrap() = ComponentState::Configured;
                }

                // Reset needed? (target is Idle, but we have armed/configured state)
                if target_state == ComponentState::Idle && (conn.hw_armed || conn.hw_configured) {
                    info!("Resetting digitizer");
                    let _ = conn.handle.send_command("/cmd/disarmacquisition");
                    let _ = conn.handle.send_command("/cmd/cleardata");
                    conn.hw_armed = false;
                    conn.hw_running = false;
                    conn.hw_configured = false;
                    conn.auto_config_failed = false;
                    read_error_since = None;
                    *hw_state.lock().unwrap() = ComponentState::Idle;
                }
            }

            // --- Handle requests from command handler (Detect / ApplyConfig) ---
            if let Ok(req) = request_rx.try_recv() {
                match req {
                    ReadLoopRequest::Detect { response_tx } => {
                        // Try to connect on-demand for Detect
                        if connection.is_none() {
                            connection = try_connect_opendpp(&config.url);
                            last_connect_attempt = Instant::now();
                        }
                        let result = match connection.as_ref() {
                            Some(conn) => conn
                                .handle
                                .get_device_info()
                                .map(|info| serde_json::to_value(&info).unwrap_or_default())
                                .map_err(|e| format!("Failed to read device info: {}", e)),
                            None => Err("Not connected to digitizer".to_string()),
                        };
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfig {
                        config: dig_config,
                        response_tx,
                    } => {
                        if connection.is_none() {
                            connection = try_connect_opendpp(&config.url);
                            last_connect_attempt = Instant::now();
                        }
                        let result = match connection.as_ref() {
                            Some(conn) => {
                                if let Some(ref cache) = conn.param_cache {
                                    conn.handle
                                        .apply_config_validated(&dig_config, cache)
                                        .map(|r| r.ok + r.adjusted)
                                        .map_err(|e| format!("Failed to apply config: {}", e))
                                } else {
                                    conn.handle
                                        .apply_config(&dig_config)
                                        .map_err(|e| format!("Failed to apply config: {}", e))
                                }
                            }
                            None => Err("Not connected to digitizer".to_string()),
                        };
                        if result.is_ok() {
                            if let Some(ref mut conn) = connection {
                                conn.auto_config_failed = false;
                                conn.enabled_channels =
                                    get_enabled_channels_from_config(&dig_config);
                            }
                        }
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfigRunning {
                        config: dig_config,
                        response_tx,
                    } => {
                        let result = match connection.as_ref() {
                            Some(conn) => {
                                if let Some(ref cache) = conn.param_cache {
                                    conn.handle
                                        .apply_config_running_validated(&dig_config, cache)
                                        .map(|r| r.ok + r.adjusted)
                                        .map_err(|e| {
                                            format!("Failed to apply SetInRun config: {}", e)
                                        })
                                } else {
                                    conn.handle.apply_config_running(&dig_config).map_err(|e| {
                                        format!("Failed to apply SetInRun config: {}", e)
                                    })
                                }
                            }
                            None => Err("Not connected to digitizer".to_string()),
                        };
                        let _ = response_tx.send(result);
                    }
                }
            }

            // --- Data reading (Running only) ---
            if target_state != ComponentState::Running {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }

            if let Some(ref conn) = connection {
                if !conn.hw_running {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }

                match conn
                    .endpoint
                    .read_opendpp_event(config.read_timeout_ms, &mut user_info_buffer)
                {
                    Ok(Some(event)) => {
                        if let Some(since) = read_error_since.take() {
                            info!(
                                elapsed_ms = since.elapsed().as_millis() as u64,
                                "Read recovered (OpenDPP) after transient error"
                            );
                        }
                        metrics
                            .bytes_read
                            .fetch_add(event.event_size as u64, Ordering::Relaxed);

                        // Convert OpenDPP event to EventData
                        let event_data = opendpp_to_event_data(&event, config.module_id);
                        let output = ReadLoopOutput::Decoded(event_data);

                        // Update queue length metric
                        metrics.queue_length.fetch_add(1, Ordering::Relaxed);

                        // Send to decode channel with back-pressure retry
                        let mut pending = output;
                        let mut channel_closed = false;
                        loop {
                            match tx.try_send(pending) {
                                Ok(()) => break,
                                Err(mpsc::error::TrySendError::Full(returned)) => {
                                    if shutdown.load(Ordering::Relaxed)
                                        || *state_rx.borrow() != ComponentState::Running
                                    {
                                        warn!("Dropping pending event during shutdown/stop");
                                        break;
                                    }
                                    pending = returned;
                                    std::thread::sleep(Duration::from_millis(1));
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    warn!("Decode channel closed, stopping read loop");
                                    channel_closed = true;
                                    break;
                                }
                            }
                        }
                        if channel_closed {
                            break;
                        }
                    }
                    Ok(None) => {
                        // Timeout - no data available, continue polling.
                        if let Some(since) = read_error_since.take() {
                            info!(
                                elapsed_ms = since.elapsed().as_millis() as u64,
                                "Read recovered (OpenDPP, timeout) after transient error"
                            );
                        }
                    }
                    Err(e) => {
                        if e.code == caen::error::codes::STOP {
                            if shutdown.load(Ordering::Relaxed) {
                                info!("Received STOP signal during shutdown");
                                break;
                            }
                            info!("Received STOP signal from digitizer, waiting for state change");
                            continue;
                        }
                        if target_state == ComponentState::Running {
                            // Transient error — retry (same logic as RAW loop)
                            if read_error_since.is_none() {
                                read_error_since = Some(Instant::now());
                                warn!(error = %e, code = e.code,
                                    "Read error during acquisition (OpenDPP), will retry for {:?}",
                                    READ_ERROR_TIMEOUT);
                            }
                            if read_error_since.unwrap().elapsed() > READ_ERROR_TIMEOUT {
                                error!(
                                    timeout_secs = READ_ERROR_TIMEOUT.as_secs(),
                                    error = %e, code = e.code,
                                    "Read errors persisting (OpenDPP), transitioning to Error"
                                );
                                let _ = state_tx.send(ComponentState::Error);
                                connection = None;
                                read_error_since = None;
                            } else {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                        } else {
                            // Not running — safe to reconnect
                            error!(error = %e, code = e.code, "Read error (OpenDPP), dropping connection");
                            connection = None;
                        }
                    }
                }
            } else {
                // Running but no connection — wait for reconnect at loop top
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        // Cleanup
        if let Some(conn) = connection {
            if conn.hw_armed || conn.hw_running {
                let _ = conn.handle.send_command("/cmd/disarmacquisition");
            }
        }
        info!("ReadLoop (OpenDPP) stopped");
        Ok(())
    }

    /// DecodeLoop task - decodes raw data and publishes via ZMQ
    async fn decode_loop(
        config: ReaderConfig,
        mut rx: mpsc::Receiver<ReadLoopOutput>,
        mut data_socket: publish::Publish,
        metrics: Arc<ReaderMetrics>,
        state_rx: watch::Receiver<ComponentState>,
        mut shutdown: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), ReaderError> {
        info!("DecodeLoop starting");

        let adc_min = config.adc_min;
        if adc_min > 0 {
            info!(
                adc_min,
                "ADC minimum filter enabled: events with energy < {} will be discarded", adc_min
            );
        }

        // Create decoder based on firmware type
        let mut decoder = match config.firmware {
            FirmwareType::PSD2 => {
                let psd2_config = Psd2Config {
                    time_step_ns: config.time_step_ns,
                    module_id: config.module_id,
                    dump_enabled: false,
                    num_channels: 32,
                };
                DecoderKind::Psd2(Psd2Decoder::new(psd2_config))
            }
            FirmwareType::PSD1 => {
                let psd1_config = Psd1Config {
                    time_step_ns: config.time_step_ns,
                    module_id: config.module_id,
                    dump_enabled: false,
                };
                DecoderKind::Psd1(Psd1Decoder::new(psd1_config))
            }
            FirmwareType::PHA1 => {
                let pha1_config = Pha1Config {
                    time_step_ns: config.time_step_ns,
                    module_id: config.module_id,
                    dump_enabled: false,
                };
                DecoderKind::Pha1(Pha1Decoder::new(pha1_config))
            }
            FirmwareType::AMax => {
                let amax_config = AMaxConfig {
                    module_id: config.module_id,
                    dump_enabled: false,
                    num_channels: 1, // AMax typically uses only ch0
                };
                DecoderKind::AMax(AMaxDecoder::new(amax_config))
            }
            FirmwareType::X743CI | FirmwareType::X743Std => {
                unreachable!("x743 uses its own read loop, not the FELib decode pipeline")
            }
        };

        let mut sequence_number: u64 = 0;
        let mut heartbeat_counter: u64 = 0;

        // Reusable Vec for decoded events (avoids allocation per-batch)
        let mut events_buffer: Vec<decoder::EventData> = Vec::with_capacity(1024);

        // Rate-limited trigger loss warning (DIG1)
        let mut last_trigger_loss_warn = Instant::now();
        let mut last_trigger_loss_logged: u64 = 0;

        // Heartbeat ticker
        let use_heartbeat = config.heartbeat_interval_ms > 0;
        let mut heartbeat_ticker =
            interval(Duration::from_millis(config.heartbeat_interval_ms.max(100)));

        loop {
            tokio::select! {
                biased;

                _ = shutdown.recv() => {
                    info!("DecodeLoop received shutdown signal");
                    break;
                }

                // Heartbeat (only when Running)
                _ = heartbeat_ticker.tick(), if use_heartbeat && *state_rx.borrow() == ComponentState::Running => {
                    let hb = Message::heartbeat(config.source_id, heartbeat_counter);
                    heartbeat_counter += 1;
                    match hb.to_msgpack() {
                        Ok(bytes) => {
                            let msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                            if let Err(e) = data_socket.send(msg).await {
                                warn!(error = %e, "Failed to send heartbeat");
                            } else {
                                debug!(counter = heartbeat_counter, "Published heartbeat");
                            }
                        }
                        Err(e) => warn!(error = %e, "Failed to serialize heartbeat"),
                    }
                }

                // Receive data from ReadLoop
                output = rx.recv() => {
                    match output {
                        Some(ReadLoopOutput::Raw(raw_data)) => {
                            // Update queue length metric
                            metrics.queue_length.fetch_sub(1, Ordering::Relaxed);

                            // Classify and decode
                            let data_type = decoder.classify(&raw_data);
                            match data_type {
                                DataType::Event => {
                                    // Decode events into reusable buffer
                                    let raw_size = raw_data.size;
                                    let raw_n_events = raw_data.n_events;
                                    decoder.decode_into(&raw_data, &mut events_buffer);

                                    if events_buffer.is_empty() {
                                        warn!(raw_size, raw_n_events, "Decoded 0 events from raw data");
                                        continue;
                                    }

                                    // Convert to EventDataBatch (draining events for zero-copy waveform move)
                                    let n_events = events_buffer.len();
                                    let mut batch = EventDataBatch::with_capacity(
                                        config.source_id,
                                        sequence_number,
                                        n_events,
                                    );

                                    for event in events_buffer.drain(..) {
                                        let common_event =
                                            Self::convert_event(event, config.firmware);
                                        // Count trigger loss flags (DIG1 only)
                                        if config.firmware.is_dig1() {
                                            if common_event.has_trigger_lost() {
                                                metrics
                                                    .trigger_lost_flag_events
                                                    .fetch_add(1, Ordering::Relaxed);
                                            }
                                            if (common_event.flags
                                                & crate::common::flags::FLAG_N_LOST_TRIGGER)
                                                != 0
                                            {
                                                metrics
                                                    .n_lost_trigger_flag_events
                                                    .fetch_add(1, Ordering::Relaxed);
                                                // Each N_LOST flag ≈ 1024 lost triggers
                                                metrics
                                                    .trigger_loss_count
                                                    .fetch_add(1024, Ordering::Relaxed);
                                            }
                                        }
                                        // ADC minimum filter
                                        if adc_min > 0 && common_event.energy < adc_min {
                                            metrics.filtered_events.fetch_add(1, Ordering::Relaxed);
                                            continue;
                                        }
                                        // Per-channel count (after filter)
                                        let ch = common_event.channel as usize;
                                        if ch < MAX_CHANNELS {
                                            metrics.per_channel_counts[ch].fetch_add(1, Ordering::Relaxed);
                                        }
                                        batch.push(common_event);
                                    }

                                    // Update metrics (n_events = pre-filter count)
                                    metrics.events_decoded.fetch_add(n_events as u64, Ordering::Relaxed);

                                    // Skip empty batches (all events filtered)
                                    if batch.is_empty() {
                                        continue;
                                    }
                                    let n_events = batch.len();

                                    // Rate-limited trigger loss warning (DIG1)
                                    if config.firmware.is_dig1() {
                                        let lost = metrics.trigger_loss_count.load(Ordering::Relaxed);
                                        if lost > last_trigger_loss_logged
                                            && last_trigger_loss_warn.elapsed() >= Duration::from_secs(10)
                                        {
                                            let flag_events = metrics.trigger_lost_flag_events.load(Ordering::Relaxed);
                                            let n_lost_events = metrics.n_lost_trigger_flag_events.load(Ordering::Relaxed);
                                            warn!(
                                                estimated_lost = lost,
                                                trigger_lost_flags = flag_events,
                                                n_lost_flags = n_lost_events,
                                                "Trigger loss detected (DIG1 EXTRAS flags)"
                                            );
                                            last_trigger_loss_warn = Instant::now();
                                            last_trigger_loss_logged = lost;
                                        }
                                    }

                                    // Publish
                                    let msg = Message::data(batch);
                                    match msg.to_msgpack() {
                                        Ok(bytes) => {
                                            let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                                            if let Err(e) = data_socket.send(zmq_msg).await {
                                                error!(error = %e, events = n_events, "Failed to send event batch via ZMQ");
                                            } else {
                                                sequence_number += 1;
                                                metrics.batches_published.fetch_add(1, Ordering::Relaxed);
                                                debug!(events = n_events, seq = sequence_number - 1, "Decoded and published batch");
                                            }
                                        }
                                        Err(e) => {
                                            error!(error = %e, events = n_events, "Failed to serialize event batch");
                                        }
                                    }
                                }
                                DataType::Start => {
                                    info!("Received START signal from digitizer");
                                    sequence_number = 0;
                                    heartbeat_counter = 0;
                                    decoder.reset_for_new_run();
                                    info!("Sequence number and decoder state reset on Start");
                                }
                                DataType::Stop => {
                                    info!("Received STOP signal from digitizer");
                                    let eos = Message::eos(config.source_id, 0);
                                    match eos.to_msgpack() {
                                        Ok(bytes) => {
                                            let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                                            if let Err(e) = data_socket.send(zmq_msg).await {
                                                error!(error = %e, "Failed to send EOS via ZMQ");
                                            } else {
                                                info!(source_id = config.source_id, "Published EOS");
                                            }
                                        }
                                        Err(e) => error!(error = %e, "Failed to serialize EOS"),
                                    }
                                }
                                DataType::Unknown => {
                                    warn!("Received unknown data type");
                                }
                            }
                        }

                        Some(ReadLoopOutput::Decoded(event_data)) => {
                            // Pre-decoded event from OpenDPP (AMax)
                            metrics.queue_length.fetch_sub(1, Ordering::Relaxed);

                            // Create single-event batch
                            let mut batch = EventDataBatch::with_capacity(
                                config.source_id,
                                sequence_number,
                                1,
                            );
                            let common_event = Self::convert_event(event_data, config.firmware);
                            let ch = common_event.channel as usize;
                            if ch < MAX_CHANNELS {
                                metrics.per_channel_counts[ch].fetch_add(1, Ordering::Relaxed);
                            }
                            batch.push(common_event);

                            // Update metrics
                            metrics.events_decoded.fetch_add(1, Ordering::Relaxed);

                            // Publish
                            let msg = Message::data(batch);
                            match msg.to_msgpack() {
                                Ok(bytes) => {
                                    let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                                    if let Err(e) = data_socket.send(zmq_msg).await {
                                        error!(error = %e, "Failed to send event via ZMQ");
                                    } else {
                                        sequence_number += 1;
                                        metrics.batches_published.fetch_add(1, Ordering::Relaxed);
                                        debug!(seq = sequence_number - 1, "Published OpenDPP event");
                                    }
                                }
                                Err(e) => {
                                    error!(error = %e, "Failed to serialize event");
                                }
                            }
                        }

                        Some(ReadLoopOutput::Start) => {
                            info!("Received START signal from ReadLoop");
                            sequence_number = 0;
                            heartbeat_counter = 0;
                            decoder.reset_for_new_run();
                            info!("Sequence number and decoder state reset on Start");
                        }

                        Some(ReadLoopOutput::Stop) => {
                            info!("Received STOP signal from ReadLoop");
                            let eos = Message::eos(config.source_id, 0);
                            match eos.to_msgpack() {
                                Ok(bytes) => {
                                    let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                                    if let Err(e) = data_socket.send(zmq_msg).await {
                                        error!(error = %e, "Failed to send EOS via ZMQ");
                                    } else {
                                        info!(source_id = config.source_id, "Published EOS");
                                    }
                                }
                                Err(e) => error!(error = %e, "Failed to serialize EOS"),
                            }
                        }

                        None => {
                            info!("Data channel closed, stopping decode loop");
                            break;
                        }
                    }

                    // Yield to tokio scheduler after processing each message.
                    // Without this, the decode loop can monopolize the tokio worker
                    // thread under high data rates (all futures resolve immediately),
                    // starving command_task and causing Stop command timeouts.
                    tokio::task::yield_now().await;
                }
            }
        }

        info!(
            total_batches = sequence_number,
            total_events = metrics.events_decoded.load(Ordering::Relaxed),
            "DecodeLoop stopped"
        );
        Ok(())
    }

    /// Run the reader with command control
    ///
    /// Spawns three tasks:
    /// - Command task: handles control commands
    /// - ReadLoop task: reads from CAEN hardware (blocking)
    /// - DecodeLoop task: decodes and publishes data (async)
    pub async fn run(
        mut self,
        mut shutdown: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), ReaderError> {
        info!(
            source_id = self.config.source_id,
            state = %self.state(),
            "Reader ready, waiting for commands"
        );

        // Create channels (using ReadLoopOutput to support both RAW and OpenDPP paths)
        let (data_tx, data_rx) = mpsc::channel::<ReadLoopOutput>(1000);

        // Shutdown flag for ReadLoop (it runs in spawn_blocking, can't use async channel)
        let read_shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let read_shutdown_clone = read_shutdown.clone();

        // Channel for delegating hardware requests (Detect/ApplyConfig) to the read_loop
        let (request_tx, request_rx) = std::sync::mpsc::channel::<ReadLoopRequest>();

        // Hardware-confirmed state: ReadLoop updates this after actual HW transitions.
        // GetStatus reports min(software_state, hw_state) so Operator waits until
        // hardware is truly ready before proceeding (e.g. Start after Arm).
        let hw_state = Arc::new(std::sync::Mutex::new(ComponentState::Idle));
        let hw_state_for_read = hw_state.clone();

        // Spawn command handler task using common infrastructure
        let command_address = self.config.command_address.clone();
        let shared_state = self.shared_state.clone();
        let state_tx = self.state_tx.clone();
        let shutdown_for_cmd = shutdown.resubscribe();
        let metrics_for_cmd = self.metrics.clone();
        let rate_tracker_for_cmd = self.rate_tracker.clone();

        let cmd_handle = tokio::spawn(async move {
            run_command_task(
                command_address,
                shared_state,
                state_tx,
                shutdown_for_cmd,
                move |state, tx, cmd| {
                    let mut ext = ReaderCommandExt {
                        metrics: metrics_for_cmd.clone(),
                        rate_tracker: rate_tracker_for_cmd.clone(),
                        request_tx: request_tx.clone(),
                        hw_state: hw_state.clone(),
                    };
                    handle_command(state, tx, cmd, Some(&mut ext))
                },
                "Reader",
            )
            .await;
        });

        // Spawn ReadLoop task (blocking)
        // Select read loop based on firmware type:
        // - AMax uses OpenDPP endpoint (pre-decoded events)
        // - Others use RAW endpoint (requires software decoding)
        let read_config = self.config.clone();
        let read_state_rx = self.state_rx.clone();
        let read_metrics = self.metrics.clone();
        let use_opendpp = self.config.firmware == FirmwareType::AMax;

        let read_state_tx = self.state_tx.clone();
        let read_handle = tokio::task::spawn_blocking(move || {
            if use_opendpp {
                info!("Using OpenDPP endpoint for AMax firmware");
                Self::read_loop_opendpp(
                    read_config,
                    data_tx,
                    read_state_rx,
                    read_state_tx,
                    read_metrics,
                    read_shutdown_clone,
                    request_rx,
                    hw_state_for_read,
                )
            } else {
                info!("Using RAW endpoint for firmware {:?}", read_config.firmware);
                Self::read_loop_raw(
                    read_config,
                    data_tx,
                    read_state_rx,
                    read_state_tx,
                    read_metrics,
                    read_shutdown_clone,
                    request_rx,
                    hw_state_for_read,
                )
            }
        });

        // Take ownership of data_socket for decode loop
        let data_socket = std::mem::replace(
            &mut self.data_socket,
            // Dummy socket - will not be used after this
            publish(&Context::new()).bind("tcp://127.0.0.1:0").unwrap(),
        );

        // Spawn DecodeLoop task
        let decode_config = self.config.clone();
        let decode_metrics = self.metrics.clone();
        let decode_state_rx = self.state_rx.clone();
        let shutdown_for_decode = shutdown.resubscribe();

        let decode_handle = tokio::spawn(async move {
            Self::decode_loop(
                decode_config,
                data_rx,
                data_socket,
                decode_metrics,
                decode_state_rx,
                shutdown_for_decode,
            )
            .await
        });

        // Wait for shutdown signal
        let _ = shutdown.recv().await;
        info!("Reader received shutdown signal");

        // Signal ReadLoop to stop
        read_shutdown.store(true, Ordering::Relaxed);

        // Wait for tasks to complete
        let _ = cmd_handle.await;
        match read_handle.await {
            Ok(Ok(())) => info!("ReadLoop completed normally"),
            Ok(Err(e)) => error!(error = %e, "ReadLoop exited with error"),
            Err(e) => error!(error = %e, "ReadLoop task panicked"),
        }
        match decode_handle.await {
            Ok(Ok(())) => info!("DecodeLoop completed normally"),
            Ok(Err(e)) => error!(error = %e, "DecodeLoop exited with error"),
            Err(e) => error!(error = %e, "DecodeLoop task panicked"),
        }

        // Send EOS if we were running
        if *self.state_rx.borrow() == ComponentState::Running {
            self.send_eos().await?;
        }

        info!(
            total_events = self.metrics.events_decoded.load(Ordering::Relaxed),
            total_bytes = self.metrics.bytes_read.load(Ordering::Relaxed),
            total_batches = self.metrics.batches_published.load(Ordering::Relaxed),
            "Reader stopped"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ReaderConfig::default();
        assert_eq!(config.source_id, 0);
        assert_eq!(config.firmware, FirmwareType::PSD2);
        assert_eq!(config.buffer_size, 64 * 1024 * 1024);
    }

    #[test]
    fn test_convert_event() {
        let event = EventData {
            timestamp_ns: 1234567.0,
            module: 1,
            channel: 5,
            energy: 1000,
            energy_short: 800,
            fine_time: 512,
            flags: 0x01,
            waveform: None,
        };

        let minimal = Reader::convert_event(event, FirmwareType::PSD2);
        // CommonEventData is packed, so we need to copy values before comparing
        let module = minimal.module;
        let channel = minimal.channel;
        let energy = { minimal.energy };
        let energy_short = { minimal.energy_short };
        let timestamp_ns = { minimal.timestamp_ns };
        let flags = { minimal.flags };

        assert_eq!(module, 1);
        assert_eq!(channel, 5);
        assert_eq!(energy, 1000);
        assert_eq!(energy_short, 800);
        assert_eq!(timestamp_ns, 1234567.0);
        assert_eq!(flags, 0x01);
        assert!(minimal.waveform.is_none());
    }

    #[test]
    fn test_from_config_psd2_maps_firmware() {
        let toml = r#"
            [[network.sources]]
            id = 0
            type = "psd2"
            bind = "tcp://*:5555"
            digitizer_url = "dig2://172.18.4.56"

            [network.merger]
            subscribe = ["tcp://localhost:5555"]
            publish = "tcp://*:5557"

            [network.recorder]
            subscribe = "tcp://localhost:5557"
        "#;
        let config = crate::config::Config::from_toml(toml).unwrap();
        let reader_config = ReaderConfig::from_config(&config, 0).unwrap();
        assert_eq!(reader_config.firmware, FirmwareType::PSD2);
    }

    #[test]
    fn test_from_config_psd1_maps_firmware() {
        let toml = r#"
            [[network.sources]]
            id = 0
            type = "psd1"
            bind = "tcp://*:5555"
            digitizer_url = "dig1://caen.internal/usb?link_num=0"

            [network.merger]
            subscribe = ["tcp://localhost:5555"]
            publish = "tcp://*:5557"

            [network.recorder]
            subscribe = "tcp://localhost:5557"
        "#;
        let config = crate::config::Config::from_toml(toml).unwrap();
        let reader_config = ReaderConfig::from_config(&config, 0).unwrap();
        assert_eq!(reader_config.firmware, FirmwareType::PSD1);
    }

    #[test]
    fn test_from_config_emulator_returns_none() {
        let toml = r#"
            [[network.sources]]
            id = 0
            type = "emulator"
            bind = "tcp://*:5555"
            digitizer_url = "dig2://172.18.4.56"

            [network.merger]
            subscribe = ["tcp://localhost:5555"]
            publish = "tcp://*:5557"

            [network.recorder]
            subscribe = "tcp://localhost:5557"
        "#;
        let config = crate::config::Config::from_toml(toml).unwrap();
        // Emulator sources should NOT create a ReaderConfig
        assert!(ReaderConfig::from_config(&config, 0).is_none());
    }

    #[test]
    fn test_convert_event_with_waveform() {
        let wf = Waveform {
            analog_probe1: vec![100, 200, -300],
            analog_probe2: vec![10, 20, -30],
            digital_probe1: vec![1, 0, 1],
            digital_probe2: vec![0, 1, 0],
            digital_probe3: vec![1, 1, 0],
            digital_probe4: vec![0, 0, 1],
            time_resolution: 2,
            trigger_threshold: 500,
            ns_per_sample: 2.0,
        };

        let event = EventData {
            timestamp_ns: 999.0,
            module: 0,
            channel: 3,
            energy: 2000,
            energy_short: 1500,
            fine_time: 100,
            flags: 0x00,
            waveform: Some(wf),
        };

        let converted = Reader::convert_event(event, FirmwareType::PSD2);
        assert!(converted.waveform.is_some(), "Waveform should be preserved");
        let cwf = converted.waveform.unwrap();
        assert_eq!(cwf.analog_probe1, vec![100, 200, -300]);
        assert_eq!(cwf.analog_probe2, vec![10, 20, -30]);
        assert_eq!(cwf.digital_probe1, vec![1, 0, 1]);
        assert_eq!(cwf.time_resolution, 2);
        assert_eq!(cwf.trigger_threshold, 500);
        assert_eq!(cwf.ns_per_sample, 2.0);
    }
}

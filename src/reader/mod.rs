//! Reader module for digitizer data acquisition
//!
//! This module provides:
//! - CAEN digitizer FFI bindings (caen)
//! - Data decoders (decoder)
//! - Reader integration with two-task architecture

pub mod caen;
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tmq::publish;
use tmq::Context;
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
    /// Stop signal from digitizer (reserved for future use)
    #[allow(dead_code)]
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
            // Emulator/Zle sources shouldn't create a Reader — caller should handle
            _ => return None,
        };

        Some(Self {
            url: url.clone(),
            data_address: source.bind.clone(),
            command_address: source.command_address(),
            source_id,
            firmware,
            module_id: source.module_id.unwrap_or(source_id as u8),
            read_timeout_ms: 100,
            buffer_size: 64 * 1024 * 1024, // 64MB - CAEN FELib has no bounds check
            heartbeat_interval_ms: 1000,
            time_step_ns: source.time_step_ns.unwrap_or(2.0),
            config_file: source.config_file.clone(),
        })
    }
}

/// Metrics for monitoring
#[derive(Debug, Default)]
pub struct ReaderMetrics {
    /// Total events decoded
    pub events_decoded: AtomicU64,
    /// Total bytes read from digitizer
    pub bytes_read: AtomicU64,
    /// Total batches published
    pub batches_published: AtomicU64,
    /// Current decode queue length (approximate)
    pub queue_length: AtomicU64,
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
        self.rate_tracker.update(events);
        Some(crate::common::ComponentMetrics {
            events_processed: events,
            bytes_transferred: bytes,
            queue_size: queue as u32,
            queue_max: 0,
            event_rate: self.rate_tracker.get_rate(),
            data_rate: 0.0,
        })
    }

    fn on_start(&mut self, _run_number: u32) -> Result<(), String> {
        self.rate_tracker.reset();
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

        info!(
            data_address = %config.data_address,
            command_address = %config.command_address,
            url = %config.url,
            "Reader bound to data address"
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

    /// Convert EventData to CommonEventData (consumes event, zero-copy for waveforms)
    fn convert_event(event: EventData) -> CommonEventData {
        if let Some(wf) = event.waveform {
            CommonEventData::with_waveform(
                event.module,
                event.channel,
                event.energy,
                event.energy_short,
                event.timestamp_ns,
                event.flags as u64,
                CommonWaveform {
                    analog_probe1: wf.analog_probe1,   // move, not clone
                    analog_probe2: wf.analog_probe2,   // move
                    digital_probe1: wf.digital_probe1, // move
                    digital_probe2: wf.digital_probe2, // move
                    digital_probe3: wf.digital_probe3, // move
                    digital_probe4: wf.digital_probe4, // move
                    time_resolution: wf.time_resolution,
                    trigger_threshold: wf.trigger_threshold,
                },
            )
        } else {
            CommonEventData::new(
                event.module,
                event.channel,
                event.energy,
                event.energy_short,
                event.timestamp_ns,
                event.flags as u64,
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
            Message::EndOfStream { source_id } => {
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
        let eos = Message::eos(self.config.source_id);
        self.publish_message(&eos).await
    }

    /// ReadLoop task for RAW endpoint (PSD1/PSD2/PHA1) - runs in spawn_blocking
    ///
    /// Reads raw data from CAEN digitizer and sends to decode channel.
    /// Respects state machine: only arms/starts digitizer when state transitions occur.
    fn read_loop_raw(
        config: ReaderConfig,
        tx: mpsc::Sender<ReadLoopOutput>,
        state_rx: watch::Receiver<ComponentState>,
        metrics: Arc<ReaderMetrics>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
        request_rx: std::sync::mpsc::Receiver<ReadLoopRequest>,
    ) -> Result<(), ReaderError> {
        info!(url = %config.url, "ReadLoop (RAW) starting, connecting to digitizer");

        // Open connection to digitizer
        let handle = CaenHandle::open(&config.url)?;
        info!("Connected to digitizer");

        // Configure endpoint for RAW data
        let include_n_events = config.firmware.includes_n_events();
        let endpoint = handle.configure_endpoint(include_n_events)?;
        info!("Endpoint configured");

        // Track digitizer hardware state
        let mut hw_armed = false;
        let mut hw_running = false;
        let mut dig1_needs_reconfig = false; // DIG1: set after reset, cleared after config applied
        let mut prev_state = ComponentState::Idle;

        // Cache for config pushed from Operator (used instead of local file on Arm)
        let mut cached_config: Option<crate::config::digitizer::DigitizerConfig> = None;

        // Pre-allocate reusable read buffer.
        // CAEN FELib does NOT check buffer bounds — undersized buffers cause SIGBUS.
        // Must be large enough for worst-case data (high rate + waveforms).
        let mut read_buffer: Vec<u8> = vec![0u8; config.buffer_size];
        info!(
            buffer_size = config.buffer_size,
            "ReadLoop buffer allocated"
        );

        loop {
            // Check shutdown flag
            if shutdown.load(Ordering::Relaxed) {
                info!("ReadLoop received shutdown signal");
                break;
            }

            // Get current state
            let current_state = *state_rx.borrow();

            // Handle state transitions
            if current_state != prev_state {
                info!(from = %prev_state, to = %current_state, "State transition");

                match (prev_state, current_state) {
                    // Configure digitizer when entering Configured state from Idle
                    (ComponentState::Idle, ComponentState::Configured) => {
                        // Apply configuration from JSON file if specified
                        if let Some(ref config_path) = config.config_file {
                            info!(path = %config_path, "Loading digitizer configuration");
                            match crate::config::digitizer::DigitizerConfig::load(config_path) {
                                Ok(dig_config) => {
                                    match handle.apply_config(&dig_config) {
                                        Ok(count) => {
                                            info!(count, "Digitizer configuration applied");
                                        }
                                        Err(e) => {
                                            error!(error = %e, "Failed to apply digitizer configuration");
                                            // Continue anyway - some parameters may have been applied
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!(error = %e, path = %config_path, "Failed to load digitizer configuration");
                                    // Continue without configuration
                                }
                            }
                        } else {
                            info!("No config_file specified, using current digitizer settings");
                        }
                    }

                    // Arm digitizer when entering Armed state
                    (_, ComponentState::Armed) => {
                        // DIG1: re-apply config if reset was performed (reset clears all params)
                        if dig1_needs_reconfig {
                            // Prefer cached config from Operator (network transparency)
                            // Fall back to local file if no cached config
                            if let Some(ref dig_config) = cached_config {
                                info!("DIG1: Re-applying cached config from Operator after reset");
                                match handle.apply_config(dig_config) {
                                    Ok(count) => {
                                        info!(count, "DIG1: Cached configuration re-applied");
                                    }
                                    Err(e) => {
                                        error!(error = %e, "DIG1: Failed to re-apply cached config");
                                    }
                                }
                            } else if let Some(ref config_path) = config.config_file {
                                info!(path = %config_path, "DIG1: Re-applying config from file after reset");
                                match crate::config::digitizer::DigitizerConfig::load(config_path) {
                                    Ok(dig_config) => match handle.apply_config(&dig_config) {
                                        Ok(count) => {
                                            info!(count, "DIG1: File configuration re-applied");
                                        }
                                        Err(e) => {
                                            error!(error = %e, "DIG1: Failed to re-apply config");
                                        }
                                    },
                                    Err(e) => {
                                        error!(error = %e, "DIG1: Failed to load config for re-apply");
                                    }
                                }
                            } else {
                                warn!("DIG1: No cached config and no config_file - settings may be default");
                            }
                            dig1_needs_reconfig = false;
                        }
                        if !hw_armed {
                            if let Err(e) = send_arm_command(&handle, config.firmware) {
                                error!(error = %e, "Failed to arm digitizer - continuing anyway");
                            }
                            hw_armed = true;
                        }
                    }

                    // Start acquisition when entering Running state
                    // Use (_, Running) to handle both Armed→Running and
                    // Configured→Running (when watch channel misses Armed state)
                    (_, ComponentState::Running) => {
                        if !hw_running {
                            // Arm if not yet armed (handles skipped Armed state)
                            if !hw_armed {
                                if let Err(e) = send_arm_command(&handle, config.firmware) {
                                    error!(error = %e, "Failed to arm digitizer - continuing anyway");
                                }
                                hw_armed = true;
                            }
                            if let Err(e) = send_start_command(&handle, config.firmware) {
                                error!(error = %e, "Failed to start acquisition - continuing anyway");
                            }
                            hw_running = true;
                        }
                    }

                    // Stop acquisition when leaving Running state
                    (ComponentState::Running, ComponentState::Configured) => {
                        if hw_running {
                            info!("Stopping digitizer acquisition");
                            if config.firmware.is_dig1() {
                                // DIG1 requires full reset to properly re-enable triggers on next run
                                info!("DIG1: Using reset (triggers require full reset after stop)");
                                let _ = handle.send_command("/cmd/reset");
                                dig1_needs_reconfig = true; // Config was cleared by reset
                            } else {
                                let _ = handle.send_command("/cmd/disarmacquisition");
                                // Clear data buffers to ensure clean state for next run
                                // (preserves register settings, unlike /cmd/reset)
                                info!("Clearing digitizer data buffers");
                                let _ = handle.send_command("/cmd/cleardata");
                            }
                            hw_armed = false;
                            hw_running = false;
                        }
                    }

                    // Reset: disarm if armed
                    (_, ComponentState::Idle) => {
                        if hw_armed || hw_running {
                            info!("Resetting digitizer");
                            let _ = handle.send_command("/cmd/disarmacquisition");
                            let _ = handle.send_command("/cmd/cleardata");
                            hw_armed = false;
                            hw_running = false;
                        }
                    }

                    _ => {}
                }

                prev_state = current_state;
            }

            // Handle requests from command handler (Detect / ApplyConfig)
            if let Ok(req) = request_rx.try_recv() {
                match req {
                    ReadLoopRequest::Detect { response_tx } => {
                        let result = handle
                            .get_device_info()
                            .map(|info| serde_json::to_value(&info).unwrap_or_default())
                            .map_err(|e| format!("Failed to read device info: {}", e));
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfig {
                        config: dig_config,
                        response_tx,
                    } => {
                        // Cache config for use on next Arm (DIG1 reset clears settings)
                        cached_config = Some((*dig_config).clone());
                        info!("Cached digitizer config from Operator");

                        let result = handle
                            .apply_config(&dig_config)
                            .map_err(|e| format!("Failed to apply config: {}", e));
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfigRunning {
                        config: dig_config,
                        response_tx,
                    } => {
                        let result = handle
                            .apply_config_running(&dig_config)
                            .map_err(|e| format!("Failed to apply SetInRun config: {}", e));
                        let _ = response_tx.send(result);
                    }
                }
            }

            // Only read data when Running
            if current_state != ComponentState::Running {
                // Not running, sleep briefly and check again
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }

            // Read data from digitizer (reusing pre-allocated buffer)
            match endpoint.read_data(config.read_timeout_ms, &mut read_buffer) {
                Ok(Some(raw)) => {
                    metrics
                        .bytes_read
                        .fetch_add(raw.size as u64, Ordering::Relaxed);

                    // Convert to decoder RawData and wrap in ReadLoopOutput
                    let decoder_raw = decoder::RawData::from(raw);
                    let output = ReadLoopOutput::Raw(decoder_raw);

                    // Update queue length metric (approximate)
                    metrics.queue_length.fetch_add(1, Ordering::Relaxed);

                    // Send to decode channel with back-pressure retry.
                    // NOTE: blocking_send() causes TLS fatal error on macOS,
                    // so we use try_send() + retry loop instead.
                    // Check shutdown/state on each retry to avoid hanging on Stop.
                    let mut pending = output;
                    let mut channel_closed = false;
                    loop {
                        match tx.try_send(pending) {
                            Ok(()) => break,
                            Err(mpsc::error::TrySendError::Full(returned)) => {
                                // Check if we should stop retrying
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
                    // Timeout - no data available, continue polling
                }
                Err(e) => {
                    // Check if it's a stop signal
                    if e.code == caen::error::codes::STOP {
                        info!("Received STOP signal from digitizer");
                        break;
                    }
                    error!(error = %e, code = e.code, "Read error");
                    // Continue on non-fatal errors
                }
            }
        }

        // Cleanup: stop acquisition if still running
        if hw_armed || hw_running {
            let _ = handle.send_command("/cmd/disarmacquisition");
        }
        info!("ReadLoop (RAW) stopped");
        Ok(())
    }

    /// ReadLoop task for OpenDPP endpoint (AMax) - runs in spawn_blocking
    ///
    /// Reads pre-decoded event data from CAEN digitizer via OpenDPP endpoint.
    /// Each event is already decoded by the hardware, so no software decoding is needed.
    /// Used for AMax/DPP_OPEN firmware.
    fn read_loop_opendpp(
        config: ReaderConfig,
        tx: mpsc::Sender<ReadLoopOutput>,
        state_rx: watch::Receiver<ComponentState>,
        metrics: Arc<ReaderMetrics>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
        request_rx: std::sync::mpsc::Receiver<ReadLoopRequest>,
    ) -> Result<(), ReaderError> {
        info!(url = %config.url, "ReadLoop (OpenDPP) starting, connecting to digitizer");

        // Open connection to digitizer
        let handle = CaenHandle::open(&config.url)?;
        info!("Connected to digitizer");

        // Configure OpenDPP endpoint (no waveform for now)
        let endpoint = handle.configure_opendpp_endpoint(false)?;
        info!("OpenDPP endpoint configured");

        // Track digitizer hardware state
        let mut hw_armed = false;
        let mut hw_running = false;
        let mut dig1_needs_reconfig = false; // DIG1: set after reset, cleared after config applied
        let mut prev_state = ComponentState::Idle;

        // Cache for config pushed from Operator (used instead of local file on Arm)
        let mut cached_config: Option<crate::config::digitizer::DigitizerConfig> = None;

        // Buffer for user info words
        let mut user_info_buffer = [0u64; 16];

        loop {
            // Check shutdown flag
            if shutdown.load(Ordering::Relaxed) {
                info!("ReadLoop (OpenDPP) received shutdown signal");
                break;
            }

            // Get current state
            let current_state = *state_rx.borrow();

            // Handle state transitions (same logic as read_loop_raw)
            if current_state != prev_state {
                info!(from = %prev_state, to = %current_state, "State transition");

                match (prev_state, current_state) {
                    // Configure digitizer when entering Configured state from Idle
                    (ComponentState::Idle, ComponentState::Configured) => {
                        // Apply configuration from JSON file if specified
                        if let Some(ref config_path) = config.config_file {
                            info!(path = %config_path, "Loading digitizer configuration");
                            match crate::config::digitizer::DigitizerConfig::load(config_path) {
                                Ok(dig_config) => match handle.apply_config(&dig_config) {
                                    Ok(count) => {
                                        info!(count, "Digitizer configuration applied");
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Failed to apply digitizer configuration");
                                    }
                                },
                                Err(e) => {
                                    error!(error = %e, path = %config_path, "Failed to load digitizer configuration");
                                }
                            }
                        } else {
                            info!("No config_file specified, using current digitizer settings");
                        }
                    }

                    // Arm digitizer when entering Armed state
                    (_, ComponentState::Armed) => {
                        // DIG1: re-apply config if reset was performed (reset clears all params)
                        if dig1_needs_reconfig {
                            // Prefer cached config from Operator (network transparency)
                            // Fall back to local file if no cached config
                            if let Some(ref dig_config) = cached_config {
                                info!("DIG1: Re-applying cached config from Operator after reset");
                                match handle.apply_config(dig_config) {
                                    Ok(count) => {
                                        info!(count, "DIG1: Cached configuration re-applied");
                                    }
                                    Err(e) => {
                                        error!(error = %e, "DIG1: Failed to re-apply cached config");
                                    }
                                }
                            } else if let Some(ref config_path) = config.config_file {
                                info!(path = %config_path, "DIG1: Re-applying config from file after reset");
                                match crate::config::digitizer::DigitizerConfig::load(config_path) {
                                    Ok(dig_config) => match handle.apply_config(&dig_config) {
                                        Ok(count) => {
                                            info!(count, "DIG1: File configuration re-applied");
                                        }
                                        Err(e) => {
                                            error!(error = %e, "DIG1: Failed to re-apply config");
                                        }
                                    },
                                    Err(e) => {
                                        error!(error = %e, "DIG1: Failed to load config for re-apply");
                                    }
                                }
                            } else {
                                warn!("DIG1: No cached config and no config_file - settings may be default");
                            }
                            dig1_needs_reconfig = false;
                        }
                        if !hw_armed {
                            if let Err(e) = send_arm_command(&handle, config.firmware) {
                                error!(error = %e, "Failed to arm digitizer - continuing anyway");
                            }
                            hw_armed = true;
                        }
                    }

                    // Start acquisition when entering Running state
                    (_, ComponentState::Running) => {
                        if !hw_running {
                            if !hw_armed {
                                if let Err(e) = send_arm_command(&handle, config.firmware) {
                                    error!(error = %e, "Failed to arm digitizer - continuing anyway");
                                }
                                hw_armed = true;
                            }
                            if let Err(e) = send_start_command(&handle, config.firmware) {
                                error!(error = %e, "Failed to start acquisition - continuing anyway");
                            }
                            hw_running = true;
                        }
                    }

                    // Stop acquisition when leaving Running state
                    (ComponentState::Running, ComponentState::Configured) => {
                        if hw_running {
                            info!("Stopping digitizer acquisition");
                            if config.firmware.is_dig1() {
                                // DIG1 requires full reset to properly re-enable triggers on next run
                                info!("DIG1: Using reset (triggers require full reset after stop)");
                                let _ = handle.send_command("/cmd/reset");
                                dig1_needs_reconfig = true; // Config was cleared by reset
                            } else {
                                let _ = handle.send_command("/cmd/disarmacquisition");
                                // Clear data buffers to ensure clean state for next run
                                // (preserves register settings, unlike /cmd/reset)
                                info!("Clearing digitizer data buffers");
                                let _ = handle.send_command("/cmd/cleardata");
                            }
                            hw_armed = false;
                            hw_running = false;
                        }
                    }

                    // Reset: disarm if armed
                    (_, ComponentState::Idle) => {
                        if hw_armed || hw_running {
                            info!("Resetting digitizer");
                            let _ = handle.send_command("/cmd/disarmacquisition");
                            let _ = handle.send_command("/cmd/cleardata");
                            hw_armed = false;
                            hw_running = false;
                        }
                    }

                    _ => {}
                }

                prev_state = current_state;
            }

            // Handle requests from command handler (Detect / ApplyConfig)
            if let Ok(req) = request_rx.try_recv() {
                match req {
                    ReadLoopRequest::Detect { response_tx } => {
                        let result = handle
                            .get_device_info()
                            .map(|info| serde_json::to_value(&info).unwrap_or_default())
                            .map_err(|e| format!("Failed to read device info: {}", e));
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfig {
                        config: dig_config,
                        response_tx,
                    } => {
                        // Cache config for use on next Arm (DIG1 reset clears settings)
                        cached_config = Some((*dig_config).clone());
                        info!("Cached digitizer config from Operator");

                        let result = handle
                            .apply_config(&dig_config)
                            .map_err(|e| format!("Failed to apply config: {}", e));
                        let _ = response_tx.send(result);
                    }
                    ReadLoopRequest::ApplyConfigRunning {
                        config: dig_config,
                        response_tx,
                    } => {
                        let result = handle
                            .apply_config_running(&dig_config)
                            .map_err(|e| format!("Failed to apply SetInRun config: {}", e));
                        let _ = response_tx.send(result);
                    }
                }
            }

            // Only read data when Running
            if current_state != ComponentState::Running {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }

            // Read event from OpenDPP endpoint
            match endpoint.read_opendpp_event(config.read_timeout_ms, &mut user_info_buffer) {
                Ok(Some(event)) => {
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
                    // Timeout - no data available, continue polling
                }
                Err(e) => {
                    // Check if it's a stop signal
                    if e.code == caen::error::codes::STOP {
                        info!("Received STOP signal from digitizer");
                        break;
                    }
                    error!(error = %e, code = e.code, "Read error (OpenDPP)");
                }
            }
        }

        // Cleanup: stop acquisition if still running
        if hw_armed || hw_running {
            let _ = handle.send_command("/cmd/disarmacquisition");
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
        };

        let mut sequence_number: u64 = 0;
        let mut heartbeat_counter: u64 = 0;

        // Reusable Vec for decoded events (avoids allocation per-batch)
        let mut events_buffer: Vec<decoder::EventData> = Vec::with_capacity(1024);

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
                                        batch.push(Self::convert_event(event));
                                    }

                                    // Update metrics
                                    metrics.events_decoded.fetch_add(n_events as u64, Ordering::Relaxed);

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
                                    info!("Sequence number reset to 0 on Start");
                                }
                                DataType::Stop => {
                                    info!("Received STOP signal from digitizer");
                                    let eos = Message::eos(config.source_id);
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
                            batch.push(Self::convert_event(event_data));

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
                            info!("Sequence number reset to 0 on Start");
                        }

                        Some(ReadLoopOutput::Stop) => {
                            info!("Received STOP signal from ReadLoop");
                            let eos = Message::eos(config.source_id);
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

        let read_handle = tokio::task::spawn_blocking(move || {
            if use_opendpp {
                info!("Using OpenDPP endpoint for AMax firmware");
                Self::read_loop_opendpp(
                    read_config,
                    data_tx,
                    read_state_rx,
                    read_metrics,
                    read_shutdown_clone,
                    request_rx,
                )
            } else {
                info!("Using RAW endpoint for firmware {:?}", read_config.firmware);
                Self::read_loop_raw(
                    read_config,
                    data_tx,
                    read_state_rx,
                    read_metrics,
                    read_shutdown_clone,
                    request_rx,
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

        let minimal = Reader::convert_event(event);
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

        let converted = Reader::convert_event(event);
        assert!(converted.waveform.is_some(), "Waveform should be preserved");
        let cwf = converted.waveform.unwrap();
        assert_eq!(cwf.analog_probe1, vec![100, 200, -300]);
        assert_eq!(cwf.analog_probe2, vec![10, 20, -30]);
        assert_eq!(cwf.digital_probe1, vec![1, 0, 1]);
        assert_eq!(cwf.time_resolution, 2);
        assert_eq!(cwf.trigger_threshold, 500);
    }
}

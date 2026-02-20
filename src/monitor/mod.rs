//! Monitor component - receives event data and provides histogram visualization
//!
//! Architecture (Lock-Free):
//! - Receiver task: ZMQ SUB → mpsc channel (non-blocking)
//! - Histogram task: mpsc channel → histogram update (owns state, no locks in hot path)
//! - Command task: REP socket for control commands
//! - HTTP server: REST API + static files for web UI (reads histogram via channel query)
//!
//! This module provides real-time monitoring of DAQ data with browser-based
//! histogram display.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tmq::{subscribe, AsZmqSocket, Context};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, info, warn};

use crate::common::{
    handle_command, run_command_task, ChannelRegistration, Command, CommandHandlerExt,
    CommandResponse, ComponentSharedState, ComponentState, EventData, EventDataBatch, Message,
    Waveform,
};

/// Monitor configuration
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    /// ZMQ connect address (e.g., "tcp://localhost:5557")
    pub subscribe_address: String,
    /// ZMQ bind address for commands (e.g., "tcp://*:5590")
    pub command_address: String,
    /// HTTP server port
    pub http_port: u16,
    /// Default histogram configuration (Energy 1D)
    pub histogram_config: HistogramConfig,
    /// PSD 1D histogram configuration
    pub psd_histogram_config: HistogramConfig,
    /// PSD 2D X axis (Energy) configuration
    pub psd2d_x_config: HistogramConfig,
    /// PSD 2D Y axis (PSD ratio) configuration
    pub psd2d_y_config: HistogramConfig,
    /// Internal channel capacity
    pub channel_capacity: usize,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            subscribe_address: "tcp://localhost:5557".to_string(),
            command_address: "tcp://*:5590".to_string(),
            http_port: 8081,
            histogram_config: HistogramConfig::default(),
            psd_histogram_config: HistogramConfig {
                num_bins: 200,
                min_value: -0.2,
                max_value: 1.2,
            },
            psd2d_x_config: HistogramConfig {
                num_bins: 512,
                min_value: 0.0,
                max_value: 65536.0,
            },
            psd2d_y_config: HistogramConfig {
                num_bins: 200,
                min_value: -0.2,
                max_value: 1.2,
            },
            channel_capacity: 1000,
        }
    }
}

/// Monitor errors
#[derive(Error, Debug)]
pub enum MonitorError {
    #[error("ZMQ error: {0}")]
    Zmq(#[from] tmq::TmqError),

    #[error("Deserialization error: {0}")]
    Deserialization(#[from] rmp_serde::decode::Error),

    #[error("HTTP server error: {0}")]
    Http(String),
}

/// Histogram configuration
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HistogramConfig {
    /// Number of bins
    pub num_bins: u32,
    /// Minimum value
    pub min_value: f32,
    /// Maximum value
    pub max_value: f32,
}

impl Default for HistogramConfig {
    fn default() -> Self {
        Self {
            num_bins: 65536,
            min_value: 0.0,
            max_value: 65536.0, // 1 bin per ADC channel (16-bit)
        }
    }
}

/// 1D Histogram for a single channel
#[derive(Debug, Clone, Serialize)]
pub struct Histogram1D {
    pub module_id: u32,
    pub channel_id: u32,
    pub config: HistogramConfig,
    pub bins: Vec<u64>,
    pub total_counts: u64,
    pub overflow: u64,
    pub underflow: u64,
}

impl Histogram1D {
    /// Create a new histogram with the given configuration
    pub fn new(module_id: u32, channel_id: u32, config: HistogramConfig) -> Self {
        let bins = vec![0u64; config.num_bins as usize];
        Self {
            module_id,
            channel_id,
            config,
            bins,
            total_counts: 0,
            overflow: 0,
            underflow: 0,
        }
    }

    /// Fill the histogram with a value
    pub fn fill(&mut self, value: f32) {
        self.total_counts += 1;

        if value < self.config.min_value {
            self.underflow += 1;
            return;
        }

        if value >= self.config.max_value {
            self.overflow += 1;
            return;
        }

        let range = self.config.max_value - self.config.min_value;
        let bin_width = range / self.config.num_bins as f32;
        let bin = ((value - self.config.min_value) / bin_width) as usize;

        if bin < self.bins.len() {
            self.bins[bin] += 1;
        } else {
            self.overflow += 1;
        }
    }

    /// Clear the histogram
    pub fn clear(&mut self) {
        self.bins.fill(0);
        self.total_counts = 0;
        self.overflow = 0;
        self.underflow = 0;
    }
}

/// 2D Histogram for Energy vs PSD scatter plots
#[derive(Debug, Clone, Serialize)]
pub struct Histogram2D {
    pub module_id: u32,
    pub channel_id: u32,
    pub x_config: HistogramConfig,
    pub y_config: HistogramConfig,
    /// Flat array: bins[y * x_bins + x]
    pub bins: Vec<u64>,
    pub total_counts: u64,
    pub overflow: u64,
}

impl Histogram2D {
    /// Create a new 2D histogram with the given X and Y configurations
    pub fn new(module_id: u32, channel_id: u32, x_config: HistogramConfig, y_config: HistogramConfig) -> Self {
        let total_bins = x_config.num_bins as usize * y_config.num_bins as usize;
        Self {
            module_id,
            channel_id,
            x_config,
            y_config,
            bins: vec![0u64; total_bins],
            total_counts: 0,
            overflow: 0,
        }
    }

    /// Fill the 2D histogram with (x, y) values
    pub fn fill(&mut self, x: f32, y: f32) {
        self.total_counts += 1;

        // X axis bounds check
        if x < self.x_config.min_value || x >= self.x_config.max_value {
            self.overflow += 1;
            return;
        }
        // Y axis bounds check
        if y < self.y_config.min_value || y >= self.y_config.max_value {
            self.overflow += 1;
            return;
        }

        let x_range = self.x_config.max_value - self.x_config.min_value;
        let x_bin_width = x_range / self.x_config.num_bins as f32;
        let x_bin = ((x - self.x_config.min_value) / x_bin_width) as usize;

        let y_range = self.y_config.max_value - self.y_config.min_value;
        let y_bin_width = y_range / self.y_config.num_bins as f32;
        let y_bin = ((y - self.y_config.min_value) / y_bin_width) as usize;

        let idx = y_bin * self.x_config.num_bins as usize + x_bin;
        if idx < self.bins.len() {
            self.bins[idx] += 1;
        } else {
            self.overflow += 1;
        }
    }

    /// Clear the 2D histogram
    pub fn clear(&mut self) {
        self.bins.fill(0);
        self.total_counts = 0;
        self.overflow = 0;
    }
}

/// Key for identifying a channel histogram
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChannelKey {
    pub module_id: u32,
    pub channel_id: u32,
}

impl ChannelKey {
    pub fn new(module_id: u32, channel_id: u32) -> Self {
        Self {
            module_id,
            channel_id,
        }
    }
}

/// Latest waveform data for a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestWaveform {
    pub module_id: u32,
    pub channel_id: u32,
    pub energy: u16,
    pub timestamp_ns: f64,
    pub waveform: Waveform,
}

/// Monitor state containing all histograms (owned by histogram task)
#[derive(Debug, Default)]
pub struct MonitorState {
    pub histograms: HashMap<ChannelKey, Histogram1D>,
    pub psd_histograms: HashMap<ChannelKey, Histogram1D>,
    pub psd2d_histograms: HashMap<ChannelKey, Histogram2D>,
    pub latest_waveforms: HashMap<ChannelKey, LatestWaveform>,
    pub total_events: u64,
    pub start_time: Option<Instant>,
    pub histogram_config: HistogramConfig,
    pub psd_histogram_config: HistogramConfig,
    pub psd2d_x_config: HistogramConfig,
    pub psd2d_y_config: HistogramConfig,
    /// Pre-registered channels from Operator (preserved across Clear, cleared on Reset)
    pub registered_channels: Vec<ChannelRegistration>,
    /// Channel display names lookup (built from registered_channels)
    pub channel_names: HashMap<ChannelKey, String>,
}

impl MonitorState {
    pub fn new(config: &MonitorConfig) -> Self {
        Self {
            histograms: HashMap::new(),
            psd_histograms: HashMap::new(),
            psd2d_histograms: HashMap::new(),
            latest_waveforms: HashMap::new(),
            total_events: 0,
            start_time: None,
            histogram_config: config.histogram_config,
            psd_histogram_config: config.psd_histogram_config,
            psd2d_x_config: config.psd2d_x_config,
            psd2d_y_config: config.psd2d_y_config,
            registered_channels: Vec::new(),
            channel_names: HashMap::new(),
        }
    }

    /// Process an event and update histograms (consumes event for zero-copy waveform move)
    pub fn process_event(&mut self, event: EventData) {
        self.total_events += 1;

        let key = ChannelKey::new(event.module as u32, event.channel as u32);

        // 1. Energy 1D histogram (existing)
        let histogram = self.histograms.entry(key).or_insert_with(|| {
            Histogram1D::new(
                event.module as u32,
                event.channel as u32,
                self.histogram_config,
            )
        });
        histogram.fill(event.energy as f32);

        // 2. PSD calculation (only when energy > 0 to avoid division by zero)
        if event.energy > 0 {
            let psd = (event.energy as f32 - event.energy_short as f32) / event.energy as f32;

            // 2a. 1D PSD histogram
            let psd_hist = self.psd_histograms.entry(key).or_insert_with(|| {
                Histogram1D::new(
                    event.module as u32,
                    event.channel as u32,
                    self.psd_histogram_config,
                )
            });
            psd_hist.fill(psd);

            // 2b. 2D Energy vs PSD histogram
            let psd2d = self.psd2d_histograms.entry(key).or_insert_with(|| {
                Histogram2D::new(
                    event.module as u32,
                    event.channel as u32,
                    self.psd2d_x_config,
                    self.psd2d_y_config,
                )
            });
            psd2d.fill(event.energy as f32, psd);
        }

        // 3. Store latest waveform if present (move, not clone)
        if let Some(wf) = event.waveform {
            self.latest_waveforms.insert(
                key,
                LatestWaveform {
                    module_id: event.module as u32,
                    channel_id: event.channel as u32,
                    energy: event.energy,
                    timestamp_ns: event.timestamp_ns,
                    waveform: wf,
                },
            );
        }
    }

    /// Process a batch of events (consumes batch for zero-copy)
    pub fn process_batch(&mut self, batch: EventDataBatch) {
        for event in batch.events {
            self.process_event(event);
        }
    }

    /// Clear histogram data and waveforms, preserving registered channels.
    /// Re-creates empty histograms for registered channels.
    pub fn clear(&mut self) {
        self.histograms.clear();
        self.psd_histograms.clear();
        self.psd2d_histograms.clear();
        self.latest_waveforms.clear();
        self.total_events = 0;
        // Re-create empty histograms for registered channels
        self.ensure_registered_histograms();
    }

    /// Full reset: clear everything including registered channels.
    pub fn reset(&mut self) {
        self.histograms.clear();
        self.psd_histograms.clear();
        self.psd2d_histograms.clear();
        self.latest_waveforms.clear();
        self.total_events = 0;
        self.registered_channels.clear();
        self.channel_names.clear();
    }

    /// Register channels and pre-create empty histograms.
    pub fn register_channels(&mut self, channels: Vec<ChannelRegistration>) {
        // Build channel_names lookup
        self.channel_names.clear();
        for ch in &channels {
            let key = ChannelKey::new(ch.module_id, ch.channel_id);
            self.channel_names.insert(key, ch.name.clone());
        }
        self.registered_channels = channels;
        // Pre-create empty histograms
        self.ensure_registered_histograms();
    }

    /// Ensure all registered channels have histogram entries (Energy, PSD 1D, PSD 2D).
    fn ensure_registered_histograms(&mut self) {
        for ch in &self.registered_channels {
            let key = ChannelKey::new(ch.module_id, ch.channel_id);
            self.histograms.entry(key).or_insert_with(|| {
                Histogram1D::new(ch.module_id, ch.channel_id, self.histogram_config)
            });
            self.psd_histograms.entry(key).or_insert_with(|| {
                Histogram1D::new(ch.module_id, ch.channel_id, self.psd_histogram_config)
            });
            self.psd2d_histograms.entry(key).or_insert_with(|| {
                Histogram2D::new(ch.module_id, ch.channel_id, self.psd2d_x_config, self.psd2d_y_config)
            });
        }
    }

    /// Create a snapshot for HTTP responses
    fn snapshot(&self) -> MonitorStateSnapshot {
        let elapsed_secs = self
            .start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        let event_rate = if elapsed_secs > 0.0 {
            self.total_events as f64 / elapsed_secs
        } else {
            0.0
        };

        MonitorStateSnapshot {
            total_events: self.total_events,
            elapsed_secs,
            event_rate,
            histograms: self.histograms.clone(),
        }
    }

    /// Create a lightweight summary for listing (no histogram bin data)
    fn list_summary(&self) -> HistogramListSummary {
        let elapsed_secs = self
            .start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        let event_rate = if elapsed_secs > 0.0 {
            self.total_events as f64 / elapsed_secs
        } else {
            0.0
        };

        let channels: Vec<ChannelSummaryData> = self
            .histograms
            .iter()
            .map(|(key, hist)| ChannelSummaryData {
                module_id: key.module_id,
                channel_id: key.channel_id,
                total_counts: hist.total_counts,
                name: self.channel_names.get(key).cloned(),
            })
            .collect();

        HistogramListSummary {
            total_events: self.total_events,
            elapsed_secs,
            event_rate,
            channels,
        }
    }
}

/// Snapshot of monitor state for HTTP responses
#[derive(Debug, Clone)]
struct MonitorStateSnapshot {
    total_events: u64,
    elapsed_secs: f64,
    event_rate: f64,
    histograms: HashMap<ChannelKey, Histogram1D>,
}

/// Atomic counters for hot-path statistics (lock-free)
struct AtomicStats {
    received_batches: AtomicU64,
    processed_batches: AtomicU64,
    dropped_batches: AtomicU64,
}

impl AtomicStats {
    fn new() -> Self {
        Self {
            received_batches: AtomicU64::new(0),
            processed_batches: AtomicU64::new(0),
            dropped_batches: AtomicU64::new(0),
        }
    }

    #[inline]
    fn record_received(&self) {
        self.received_batches.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_processed(&self) {
        self.processed_batches.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    #[allow(dead_code)] // Reserved for future bounded channel debugging
    fn record_drop(&self) {
        self.dropped_batches.fetch_add(1, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.received_batches.store(0, Ordering::Relaxed);
        self.processed_batches.store(0, Ordering::Relaxed);
        self.dropped_batches.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.received_batches.load(Ordering::Relaxed),
            self.processed_batches.load(Ordering::Relaxed),
            self.dropped_batches.load(Ordering::Relaxed),
        )
    }
}

/// Internal channel summary data (used between histogram_task and HTTP handler)
struct ChannelSummaryData {
    module_id: u32,
    channel_id: u32,
    total_counts: u64,
    name: Option<String>,
}

/// Summary data for histogram listing (lightweight, no bin data)
struct HistogramListSummary {
    total_events: u64,
    elapsed_secs: f64,
    event_rate: f64,
    channels: Vec<ChannelSummaryData>,
}

/// Data returned from ListWaveforms query
struct WaveformListData {
    channels: Vec<WaveformChannelData>,
}

struct WaveformChannelData {
    module_id: u32,
    channel_id: u32,
    name: Option<String>,
}

/// Message type for histogram task (commands from HTTP handlers and control)
enum HistogramMessage {
    /// Clear all histograms
    Clear,
    /// Get current state snapshot (expensive: clones all histogram bins)
    GetSnapshot(oneshot::Sender<MonitorStateSnapshot>),
    /// Get lightweight summary for listing (no bin data)
    GetListSummary(oneshot::Sender<HistogramListSummary>),
    /// Get specific histogram
    GetHistogram(ChannelKey, oneshot::Sender<Option<Histogram1D>>),
    /// Get PSD 1D histogram for a channel
    GetPsdHistogram(ChannelKey, oneshot::Sender<Option<Histogram1D>>),
    /// Get PSD 2D histogram for a channel
    GetPsd2dHistogram(ChannelKey, oneshot::Sender<Option<Histogram2D>>),
    /// Get latest waveform for a channel
    GetWaveform(ChannelKey, oneshot::Sender<Option<LatestWaveform>>),
    /// List all available waveforms (union of actual waveforms + registered channels)
    ListWaveforms(oneshot::Sender<WaveformListData>),
    /// Set start time
    SetStartTime,
    /// Register channels (pre-create empty histograms, store names)
    RegisterChannels(Vec<ChannelRegistration>),
    /// Full reset (clear everything including registered channels)
    Reset,
}

/// Shared state for HTTP handlers
#[derive(Clone)]
pub struct AppState {
    /// Channel to send requests to histogram task
    histogram_tx: mpsc::UnboundedSender<HistogramMessage>,
    /// Component state for status
    pub component_state: Arc<tokio::sync::Mutex<ComponentSharedState>>,
}

// =============================================================================
// HTTP API Handlers
// =============================================================================

/// Response for histogram list
#[derive(Serialize)]
struct HistogramListResponse {
    total_events: u64,
    elapsed_secs: f64,
    event_rate: f64,
    channels: Vec<ChannelSummary>,
}

#[derive(Serialize)]
struct ChannelSummary {
    module_id: u32,
    channel_id: u32,
    total_counts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// GET /api/histograms - List all histograms
async fn list_histograms(State(state): State<AppState>) -> Json<HistogramListResponse> {
    let (tx, rx) = oneshot::channel();
    // Use lightweight summary instead of full snapshot (avoids cloning histogram bins)
    let _ = state
        .histogram_tx
        .send(HistogramMessage::GetListSummary(tx));

    match rx.await {
        Ok(summary) => {
            let mut channels: Vec<ChannelSummary> = summary
                .channels
                .into_iter()
                .map(|ch| ChannelSummary {
                    module_id: ch.module_id,
                    channel_id: ch.channel_id,
                    total_counts: ch.total_counts,
                    name: ch.name,
                })
                .collect();

            // Sort by module_id, then channel_id
            channels.sort_by(|a, b| {
                a.module_id
                    .cmp(&b.module_id)
                    .then(a.channel_id.cmp(&b.channel_id))
            });

            Json(HistogramListResponse {
                total_events: summary.total_events,
                elapsed_secs: summary.elapsed_secs,
                event_rate: summary.event_rate,
                channels,
            })
        }
        Err(_) => Json(HistogramListResponse {
            total_events: 0,
            elapsed_secs: 0.0,
            event_rate: 0.0,
            channels: vec![],
        }),
    }
}

/// Query parameters for histogram endpoint
#[derive(Debug, Deserialize)]
struct HistogramQuery {
    /// Histogram type: "energy" (default) or "psd"
    #[serde(default = "default_histogram_type")]
    r#type: String,
}

fn default_histogram_type() -> String {
    "energy".to_string()
}

/// GET /api/histograms/:module/:channel?type=energy|psd - Get specific histogram
async fn get_histogram(
    State(state): State<AppState>,
    axum::extract::Path((module_id, channel_id)): axum::extract::Path<(u32, u32)>,
    axum::extract::Query(query): axum::extract::Query<HistogramQuery>,
) -> Result<Json<Histogram1D>, StatusCode> {
    let (tx, rx) = oneshot::channel();
    let key = ChannelKey::new(module_id, channel_id);

    let msg = match query.r#type.as_str() {
        "psd" => HistogramMessage::GetPsdHistogram(key, tx),
        _ => HistogramMessage::GetHistogram(key, tx),
    };
    let _ = state.histogram_tx.send(msg);

    match rx.await {
        Ok(Some(hist)) => Ok(Json(hist)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// GET /api/histograms2d/:module/:channel - Get 2D Energy vs PSD histogram
async fn get_histogram2d(
    State(state): State<AppState>,
    axum::extract::Path((module_id, channel_id)): axum::extract::Path<(u32, u32)>,
) -> Result<Json<Histogram2D>, StatusCode> {
    let (tx, rx) = oneshot::channel();
    let key = ChannelKey::new(module_id, channel_id);
    let _ = state
        .histogram_tx
        .send(HistogramMessage::GetPsd2dHistogram(key, tx));

    match rx.await {
        Ok(Some(hist)) => Ok(Json(hist)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// POST /api/histograms/clear - Clear all histograms
async fn clear_histograms(State(state): State<AppState>) -> StatusCode {
    let _ = state.histogram_tx.send(HistogramMessage::Clear);
    info!("Histograms cleared");
    StatusCode::OK
}

// =============================================================================
// Waveform API Endpoints
// =============================================================================

/// Response for listing available waveforms
#[derive(Serialize)]
struct WaveformListResponse {
    channels: Vec<WaveformChannelInfo>,
}

#[derive(Serialize)]
struct WaveformChannelInfo {
    module_id: u32,
    channel_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// GET /api/waveforms - List all available waveforms
async fn list_waveforms(State(state): State<AppState>) -> Json<WaveformListResponse> {
    let (tx, rx) = oneshot::channel();
    let _ = state.histogram_tx.send(HistogramMessage::ListWaveforms(tx));

    match rx.await {
        Ok(data) => {
            let mut channels: Vec<WaveformChannelInfo> = data
                .channels
                .into_iter()
                .map(|ch| WaveformChannelInfo {
                    module_id: ch.module_id,
                    channel_id: ch.channel_id,
                    name: ch.name,
                })
                .collect();
            // Sort by module_id, then channel_id
            channels.sort_by(|a, b| {
                a.module_id
                    .cmp(&b.module_id)
                    .then(a.channel_id.cmp(&b.channel_id))
            });
            Json(WaveformListResponse { channels })
        }
        Err(_) => Json(WaveformListResponse { channels: vec![] }),
    }
}

/// GET /api/waveforms/:module/:channel - Get specific waveform
async fn get_waveform(
    State(state): State<AppState>,
    axum::extract::Path((module_id, channel_id)): axum::extract::Path<(u32, u32)>,
) -> Result<Json<LatestWaveform>, StatusCode> {
    let (tx, rx) = oneshot::channel();
    let key = ChannelKey::new(module_id, channel_id);
    let _ = state
        .histogram_tx
        .send(HistogramMessage::GetWaveform(key, tx));

    match rx.await {
        Ok(Some(wf)) => Ok(Json(wf)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// GET /api/status - Get monitor status
#[derive(Serialize)]
struct StatusResponse {
    state: String,
    total_events: u64,
    num_channels: usize,
    elapsed_secs: f64,
    event_rate: f64,
}

async fn get_status(State(state): State<AppState>) -> Json<StatusResponse> {
    let component = state.component_state.lock().await;
    let component_state = component.state.to_string();
    drop(component);

    let (tx, rx) = oneshot::channel();
    let _ = state.histogram_tx.send(HistogramMessage::GetSnapshot(tx));

    match rx.await {
        Ok(snapshot) => Json(StatusResponse {
            state: component_state,
            total_events: snapshot.total_events,
            num_channels: snapshot.histograms.len(),
            elapsed_secs: snapshot.elapsed_secs,
            event_rate: snapshot.event_rate,
        }),
        Err(_) => Json(StatusResponse {
            state: component_state,
            total_events: 0,
            num_channels: 0,
            elapsed_secs: 0.0,
            event_rate: 0.0,
        }),
    }
}

/// GET / - Serve the web UI
async fn serve_ui() -> impl IntoResponse {
    Html(include_str!("monitor_ui.html"))
}

/// Create the Axum router
pub fn create_router(state: AppState) -> Router {
    // CORS layer for development (Angular dev server on different port)
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/", get(serve_ui))
        .route("/api/status", get(get_status))
        .route("/api/histograms", get(list_histograms))
        .route("/api/histograms/:module_id/:channel_id", get(get_histogram))
        .route(
            "/api/histograms/clear",
            axum::routing::post(clear_histograms),
        )
        .route("/api/histograms2d/:module_id/:channel_id", get(get_histogram2d))
        .route("/api/waveforms", get(list_waveforms))
        .route("/api/waveforms/:module_id/:channel_id", get(get_waveform))
        .layer(cors)
        .layer(CompressionLayer::new())
        .with_state(state)
}

// =============================================================================
// Monitor Component
// =============================================================================

/// Command handler extension for Monitor
struct MonitorCommandExt {
    histogram_tx: mpsc::UnboundedSender<HistogramMessage>,
    atomic_stats: Arc<AtomicStats>,
}

impl CommandHandlerExt for MonitorCommandExt {
    fn component_name(&self) -> &'static str {
        "Monitor"
    }

    fn on_start(&mut self, _run_number: u32) -> Result<(), String> {
        // Clear histograms and set start time when Running begins
        // This allows viewing histograms after Stop while starting fresh each run
        let _ = self.histogram_tx.send(HistogramMessage::Clear);
        let _ = self.histogram_tx.send(HistogramMessage::SetStartTime);
        Ok(())
    }

    fn on_reset(&mut self) -> Result<(), String> {
        // Full reset: clear everything including registered channels
        let _ = self.histogram_tx.send(HistogramMessage::Reset);
        Ok(())
    }

    fn status_details(&self) -> Option<String> {
        let (recv, proc, drop) = self.atomic_stats.snapshot();
        Some(format!(
            "Received: {}, Processed: {}, Dropped: {}",
            recv, proc, drop
        ))
    }

    fn get_metrics(&self) -> Option<crate::common::ComponentMetrics> {
        let (recv, proc, _drop) = self.atomic_stats.snapshot();
        Some(crate::common::ComponentMetrics {
            events_processed: proc,
            bytes_transferred: 0, // Monitor doesn't track bytes
            queue_size: (recv.saturating_sub(proc)) as u32,
            queue_max: 0,
            event_rate: 0.0, // Will be calculated in Phase 2
            data_rate: 0.0,
        })
    }
}

/// Monitor component
pub struct Monitor {
    config: MonitorConfig,
    shared_state: Arc<tokio::sync::Mutex<ComponentSharedState>>,
    atomic_stats: Arc<AtomicStats>,
    state_rx: watch::Receiver<ComponentState>,
    state_tx: watch::Sender<ComponentState>,
}

impl Monitor {
    /// Create a new monitor
    pub async fn new(config: MonitorConfig) -> Result<Self, MonitorError> {
        let (state_tx, state_rx) = watch::channel(ComponentState::Idle);

        info!(
            subscribe = %config.subscribe_address,
            command = %config.command_address,
            http_port = config.http_port,
            "Monitor created"
        );

        Ok(Self {
            config,
            shared_state: Arc::new(tokio::sync::Mutex::new(ComponentSharedState::new())),
            atomic_stats: Arc::new(AtomicStats::new()),
            state_rx,
            state_tx,
        })
    }

    /// Get current state
    pub fn state(&self) -> ComponentState {
        *self.state_rx.borrow()
    }

    /// Run the monitor
    pub async fn run(&mut self, mut shutdown: broadcast::Receiver<()>) -> Result<(), MonitorError> {
        // Create channels (unbounded - memory growth indicates bottleneck)
        let (hist_tx, hist_rx) = mpsc::unbounded_channel::<HistogramMessage>();
        let (data_tx, data_rx) = mpsc::unbounded_channel::<EventDataBatch>();

        // Create ZMQ SUB socket
        let context = Context::new();
        let socket = subscribe(&context)
            .connect(&self.config.subscribe_address)?
            .subscribe(b"")?;
        // Never drop messages — buffer in memory instead (DAQ: no data loss)
        socket
            .get_socket()
            .set_rcvhwm(0)
            .map_err(tmq::TmqError::from)?;

        info!(
            address = %self.config.subscribe_address,
            "Monitor connected to upstream (RCVHWM=0)"
        );

        // Start HTTP server
        let app_state = AppState {
            histogram_tx: hist_tx.clone(),
            component_state: self.shared_state.clone(),
        };
        let router = create_router(app_state);

        let addr = format!("0.0.0.0:{}", self.config.http_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| MonitorError::Http(e.to_string()))?;

        info!(address = %addr, "HTTP server started");

        let http_shutdown = shutdown.resubscribe();
        let http_handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = http_shutdown.resubscribe().recv().await;
                })
                .await
                .ok();
        });

        // Start command handler
        let command_address = self.config.command_address.clone();
        let shared_state = self.shared_state.clone();
        let state_tx = self.state_tx.clone();
        let shutdown_for_cmd = shutdown.resubscribe();
        let hist_tx_for_cmd = hist_tx.clone();
        let atomic_stats_for_cmd = self.atomic_stats.clone();

        let cmd_handle = tokio::spawn(async move {
            run_command_task(
                command_address,
                shared_state,
                state_tx,
                shutdown_for_cmd,
                move |state, tx, cmd| {
                    // Intercept RegisterChannels — forward to histogram task
                    if let Command::RegisterChannels(channels) = cmd {
                        let count = channels.len();
                        let _ = hist_tx_for_cmd
                            .clone()
                            .send(HistogramMessage::RegisterChannels(channels));
                        return CommandResponse::success(
                            state.state,
                            format!("Registered {} channels", count),
                        );
                    }
                    let mut ext = MonitorCommandExt {
                        histogram_tx: hist_tx_for_cmd.clone(),
                        atomic_stats: atomic_stats_for_cmd.clone(),
                    };
                    handle_command(state, tx, cmd, Some(&mut ext))
                },
                "Monitor",
            )
            .await;
        });

        // Spawn receiver task
        let shutdown_for_recv = shutdown.resubscribe();
        let atomic_stats_for_recv = self.atomic_stats.clone();
        let state_rx_for_recv = self.state_rx.clone();
        let recv_handle = tokio::spawn(async move {
            Self::receiver_task(
                socket,
                data_tx,
                shutdown_for_recv,
                atomic_stats_for_recv,
                state_rx_for_recv,
            )
            .await
        });

        // Spawn histogram task
        let monitor_config_for_hist = self.config.clone();
        let atomic_stats_for_hist = self.atomic_stats.clone();
        let hist_handle = tokio::spawn(async move {
            Self::histogram_task(hist_rx, data_rx, monitor_config_for_hist, atomic_stats_for_hist).await
        });

        info!(state = %self.state(), "Monitor ready, waiting for commands");

        // Wait for shutdown signal
        let _ = shutdown.recv().await;
        info!("Monitor received shutdown signal");

        // Wait for tasks to complete
        let _ = recv_handle.await;
        let _ = hist_handle.await;
        let _ = cmd_handle.await;
        let _ = http_handle.await;

        let (recv, proc, drop) = self.atomic_stats.snapshot();
        info!(
            received = recv,
            processed = proc,
            dropped = drop,
            "Monitor stopped"
        );

        Ok(())
    }

    /// Receiver task: ZMQ SUB → channel (non-blocking)
    ///
    /// IMPORTANT: Always drains ZMQ socket to prevent internal buffer growth.
    /// When not Running, data is discarded immediately.
    async fn receiver_task(
        mut socket: subscribe::Subscribe,
        tx: mpsc::UnboundedSender<EventDataBatch>,
        mut shutdown: broadcast::Receiver<()>,
        atomic_stats: Arc<AtomicStats>,
        mut state_rx: watch::Receiver<ComponentState>,
    ) {
        loop {
            let is_running = *state_rx.borrow() == ComponentState::Running;

            tokio::select! {
                biased;

                _ = shutdown.recv() => {
                    info!("Monitor receiver task shutting down");
                    break;
                }

                _ = state_rx.changed() => {
                    let current = *state_rx.borrow();
                    info!(state = %current, "Monitor receiver state changed");
                    continue;
                }

                // Always receive from ZMQ to drain the socket buffer
                // Data is only forwarded when Running, otherwise discarded
                msg = socket.next() => {
                    match msg {
                        Some(Ok(multipart)) => {
                            // Not running - discard data to prevent ZMQ buffer growth
                            if !is_running {
                                continue;
                            }

                            if let Some(data) = multipart.into_iter().next() {
                                match Message::from_msgpack(&data) {
                                    Ok(Message::Data(batch)) => {
                                        atomic_stats.record_received();
                                        debug!(
                                            seq = batch.sequence_number,
                                            events = batch.len(),
                                            source = batch.source_id,
                                            "Received batch"
                                        );

                                        // Non-blocking send to histogram task (unbounded)
                                        if tx.send(batch).is_err() {
                                            info!("Histogram channel closed, exiting");
                                            break;
                                        }
                                    }
                                    Ok(Message::EndOfStream { source_id }) => {
                                        let (recv, proc, _) = atomic_stats.snapshot();
                                        info!(
                                            source_id,
                                            received_batches = recv,
                                            processed_batches = proc,
                                            "Received EOS - data stream complete"
                                        );
                                    }
                                    Ok(Message::Heartbeat(hb)) => {
                                        debug!(source_id = hb.source_id, "Received heartbeat");
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Failed to deserialize message");
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            warn!(error = %e, "ZMQ receive error");
                        }
                        None => {
                            info!("Socket closed");
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Histogram task: owns MonitorState, processes batches and HTTP queries
    async fn histogram_task(
        mut cmd_rx: mpsc::UnboundedReceiver<HistogramMessage>,
        mut data_rx: mpsc::UnboundedReceiver<EventDataBatch>,
        monitor_config: MonitorConfig,
        atomic_stats: Arc<AtomicStats>,
    ) {
        let mut state = MonitorState::new(&monitor_config);

        loop {
            tokio::select! {
                biased;

                // Command messages have priority (for responsiveness)
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(HistogramMessage::Clear) => {
                            // Drain any stale data from the data channel first
                            let mut drained = 0u64;
                            while data_rx.try_recv().is_ok() {
                                drained += 1;
                            }
                            if drained > 0 {
                                info!(drained, "Drained stale batches from previous run");
                            }

                            state.clear();
                            state.start_time = None;
                            atomic_stats.reset();
                            info!("Histograms and stats cleared");
                        }
                        Some(HistogramMessage::GetSnapshot(tx)) => {
                            let _ = tx.send(state.snapshot());
                        }
                        Some(HistogramMessage::GetListSummary(tx)) => {
                            let _ = tx.send(state.list_summary());
                        }
                        Some(HistogramMessage::GetHistogram(key, tx)) => {
                            let _ = tx.send(state.histograms.get(&key).cloned());
                        }
                        Some(HistogramMessage::GetPsdHistogram(key, tx)) => {
                            let _ = tx.send(state.psd_histograms.get(&key).cloned());
                        }
                        Some(HistogramMessage::GetPsd2dHistogram(key, tx)) => {
                            let _ = tx.send(state.psd2d_histograms.get(&key).cloned());
                        }
                        Some(HistogramMessage::GetWaveform(key, tx)) => {
                            let _ = tx.send(state.latest_waveforms.get(&key).cloned());
                        }
                        Some(HistogramMessage::ListWaveforms(tx)) => {
                            // Union of actual waveform keys + registered channel keys
                            let mut seen = std::collections::HashSet::new();
                            let mut channels = Vec::new();
                            // Actual waveforms first
                            for key in state.latest_waveforms.keys() {
                                seen.insert(*key);
                                channels.push(WaveformChannelData {
                                    module_id: key.module_id,
                                    channel_id: key.channel_id,
                                    name: state.channel_names.get(key).cloned(),
                                });
                            }
                            // Then registered channels not already present
                            for ch in &state.registered_channels {
                                let key = ChannelKey::new(ch.module_id, ch.channel_id);
                                if seen.insert(key) {
                                    channels.push(WaveformChannelData {
                                        module_id: ch.module_id,
                                        channel_id: ch.channel_id,
                                        name: Some(ch.name.clone()),
                                    });
                                }
                            }
                            let _ = tx.send(WaveformListData { channels });
                        }
                        Some(HistogramMessage::SetStartTime) => {
                            state.start_time = Some(Instant::now());
                        }
                        Some(HistogramMessage::RegisterChannels(channels)) => {
                            let count = channels.len();
                            state.register_channels(channels);
                            info!(count, "Channels registered, empty histograms pre-created");
                        }
                        Some(HistogramMessage::Reset) => {
                            // Drain stale data
                            let mut drained = 0u64;
                            while data_rx.try_recv().is_ok() {
                                drained += 1;
                            }
                            if drained > 0 {
                                info!(drained, "Drained stale batches on reset");
                            }
                            state.reset();
                            state.start_time = None;
                            atomic_stats.reset();
                            info!("Full reset: histograms, waveforms, and registered channels cleared");
                        }
                        None => {
                            info!("Command channel closed");
                            break;
                        }
                    }
                }

                // Data batches
                batch = data_rx.recv() => {
                    match batch {
                        Some(batch) => {
                            state.process_batch(batch);
                            atomic_stats.record_processed();
                        }
                        None => {
                            info!("Data channel closed");
                            break;
                        }
                    }
                }
            }
        }

        info!(
            total_events = state.total_events,
            num_channels = state.histograms.len(),
            "Histogram task completed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_config_default() {
        let config = HistogramConfig::default();
        assert_eq!(config.num_bins, 65536);
        assert_eq!(config.min_value, 0.0);
        assert_eq!(config.max_value, 65536.0);
    }

    #[test]
    fn test_histogram_fill() {
        let config = HistogramConfig {
            num_bins: 100,
            min_value: 0.0,
            max_value: 100.0,
        };
        let mut hist = Histogram1D::new(0, 0, config);

        // Fill with values
        hist.fill(50.0); // bin 50
        hist.fill(0.0); // bin 0
        hist.fill(99.9); // bin 99

        assert_eq!(hist.total_counts, 3);
        assert_eq!(hist.bins[50], 1);
        assert_eq!(hist.bins[0], 1);
        assert_eq!(hist.bins[99], 1);
    }

    #[test]
    fn test_histogram_overflow_underflow() {
        let config = HistogramConfig {
            num_bins: 100,
            min_value: 0.0,
            max_value: 100.0,
        };
        let mut hist = Histogram1D::new(0, 0, config);

        hist.fill(-10.0); // underflow
        hist.fill(100.0); // overflow (>= max)
        hist.fill(150.0); // overflow

        assert_eq!(hist.total_counts, 3);
        assert_eq!(hist.underflow, 1);
        assert_eq!(hist.overflow, 2);
    }

    #[test]
    fn test_histogram_clear() {
        let config = HistogramConfig {
            num_bins: 100,
            min_value: 0.0,
            max_value: 100.0,
        };
        let mut hist = Histogram1D::new(0, 0, config);

        hist.fill(50.0);
        hist.fill(60.0);
        assert_eq!(hist.total_counts, 2);

        hist.clear();
        assert_eq!(hist.total_counts, 0);
        assert_eq!(hist.bins[50], 0);
        assert_eq!(hist.bins[60], 0);
    }

    #[test]
    fn test_monitor_state_process_event() {
        let config = MonitorConfig::default();
        let mut state = MonitorState::new(&config);

        let event = EventData {
            module: 0,
            channel: 5,
            energy: 1000,
            energy_short: 500,
            timestamp_ns: 0.0,
            flags: 0,
            waveform: None,
        };

        state.process_event(event);

        assert_eq!(state.total_events, 1);
        assert_eq!(state.histograms.len(), 1);

        let key = ChannelKey::new(0, 5);
        let hist = state.histograms.get(&key).unwrap();
        assert_eq!(hist.total_counts, 1);
    }

    #[test]
    fn test_psd_histogram_fill() {
        let config = MonitorConfig::default();
        let mut state = MonitorState::new(&config);

        // Event with energy=1000, energy_short=300 → PSD = (1000-300)/1000 = 0.7
        let event = EventData {
            module: 0,
            channel: 0,
            energy: 1000,
            energy_short: 300,
            timestamp_ns: 0.0,
            flags: 0,
            waveform: None,
        };
        state.process_event(event);

        let key = ChannelKey::new(0, 0);

        // 1D PSD histogram should have 1 entry
        let psd_hist = state.psd_histograms.get(&key).unwrap();
        assert_eq!(psd_hist.total_counts, 1);

        // 2D histogram should have 1 entry
        let psd2d = state.psd2d_histograms.get(&key).unwrap();
        assert_eq!(psd2d.total_counts, 1);
    }

    #[test]
    fn test_psd_skipped_for_zero_energy() {
        let config = MonitorConfig::default();
        let mut state = MonitorState::new(&config);

        // Event with energy=0 → PSD calculation skipped
        let event = EventData {
            module: 0,
            channel: 0,
            energy: 0,
            energy_short: 0,
            timestamp_ns: 0.0,
            flags: 0,
            waveform: None,
        };
        state.process_event(event);

        let key = ChannelKey::new(0, 0);

        // Energy histogram should still have the event
        assert_eq!(state.histograms.get(&key).unwrap().total_counts, 1);

        // PSD histograms should NOT have any entries (lazy created, so not present)
        assert!(state.psd_histograms.get(&key).is_none());
        assert!(state.psd2d_histograms.get(&key).is_none());
    }

    #[test]
    fn test_histogram2d_fill() {
        let x_config = HistogramConfig { num_bins: 10, min_value: 0.0, max_value: 100.0 };
        let y_config = HistogramConfig { num_bins: 5, min_value: 0.0, max_value: 1.0 };
        let mut hist = Histogram2D::new(0, 0, x_config, y_config);

        hist.fill(50.0, 0.5); // x_bin=5, y_bin=2 → idx = 2*10+5 = 25
        assert_eq!(hist.total_counts, 1);
        assert_eq!(hist.bins[25], 1);
        assert_eq!(hist.overflow, 0);

        // Overflow: x out of range
        hist.fill(100.0, 0.5);
        assert_eq!(hist.total_counts, 2);
        assert_eq!(hist.overflow, 1);

        // Overflow: y out of range
        hist.fill(50.0, 1.5);
        assert_eq!(hist.total_counts, 3);
        assert_eq!(hist.overflow, 2);
    }

    #[test]
    fn test_atomic_stats() {
        let stats = AtomicStats::new();
        stats.record_received();
        stats.record_received();
        stats.record_processed();
        stats.record_drop();

        let (recv, proc, drop) = stats.snapshot();
        assert_eq!(recv, 2);
        assert_eq!(proc, 1);
        assert_eq!(drop, 1);
    }
}

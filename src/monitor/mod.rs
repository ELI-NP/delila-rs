//! Monitor component - receives event data and provides histogram visualization
//!
//! Architecture (Lock-Free):
//! - Receive thread: plain OS thread, blocking ZMQ SUB → header parse →
//!   bounded mpsc of raw frames (`zmq::Message`, not yet deserialized)
//! - Histogram task: raw frame → msgpack decode → histogram update (owns
//!   state, no locks in hot path)
//! - Command task: REP socket for control commands
//! - HTTP server: REST API + static files for web UI (reads histogram via channel query)
//!
//! This module provides real-time monitoring of DAQ data with browser-based
//! histogram display.
//!
//! # Why the SUB socket lives on its own OS thread (TODO 84 §8, 2026-11-07)
//!
//! On pf-qaq at 2.8-5 MHz the processed counter froze for 8-16 s every
//! ~25 s with one thread at 100 %. The receiver used to be a tokio task
//! polling a `tmq` SUB socket. Three things combined:
//!
//! 1. `tmq` answers `next()` straight from `ZMQ_EVENTS` while data is
//!    queued, without touching tokio's `AsyncFd` or the coop budget, and the
//!    other `select!` branches give their budget back when `Pending`. The
//!    receiver therefore never yielded as long as ZMQ had a backlog.
//! 2. The consumer's wake-up from `try_send` was issued on the receiver's
//!    worker thread, so tokio put it in that worker's LIFO slot, which no
//!    other worker may steal, and notified nobody.
//! 3. The receiver's worker was usually the one holding the I/O+time driver
//!    when the data arrived, and nobody else turned the driver afterwards:
//!    timers, the HTTP listener and the REP socket went dead with it.
//!
//! A dedicated thread with a blocking `zmq::Socket` sidesteps all three: it
//! never runs on a worker, wakes the consumer through tokio's inject queue,
//! and the ZMQ file descriptor never enters tokio's driver. The thread only
//! parses the header (nanoseconds), so it drains the SUB pipe at wire speed
//! and the HWM=0 buffer on our side stays empty; memory is bounded by
//! `MonitorConfig::channel_capacity` raw frames. Deserialization (~9 ms per
//! 33k-event batch) happens in the histogram task, which yields after every
//! batch so HTTP and commands keep their turn. When the consumer is behind,
//! whole frames are dropped before deserialization; drops are counted in
//! `dropped_batches` and logged (display path only, CLAUDE.md exception).

mod axis;
pub mod energy_calib;
pub use axis::AxisSource;
use energy_calib::{ActiveCalib, CalibResponse};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tmq::AsZmqSocket;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, info, warn};

use crate::common::{
    handle_command, run_command_task, sub_no_hwm, ChannelRegistration, Command, CommandHandlerExt,
    CommandResponse, ComponentSharedState, ComponentState, EventData, EventDataBatch, Message,
    MessageHeader, Waveform,
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
    /// 1D `UserInfo[i]` histogram configuration (shared across the four slots).
    /// Default = 16384 bins on `[0, 16384)` (1 bin per ADC count, matches the
    /// 14-bit AMax user-info field). The view-tab slider rebins this down
    /// client-side via the rebin factor.
    pub userinfo_histogram_config: HistogramConfig,
    /// Per-axis range/bin overrides for 2D plots. Any axis not present here
    /// falls back to `AxisSource::default_axis()`. Used to honor the legacy
    /// `monitor.psd2d_x_bins` / `monitor.psd2d_y_bins` TOML knobs (see
    /// `bin/monitor.rs`) and to let operators tune ranges without
    /// recompiling.
    pub histogram2d_overrides: HashMap<AxisSource, HistogramConfig>,
    /// Capacity of the raw-frame channel between the receive thread and the
    /// histogram task, in batches (one Merger batch ≈ 770 KB at high rate).
    /// This is the only buffer on the Monitor side: the SUB pipe is drained
    /// at wire speed. When it is full, further batches are dropped before
    /// deserialization (counted). Keep it small: under sustained overload
    /// the channel is permanently full, and its depth divided by the decode
    /// rate (~90 batches/s per thread) is the lag of the display.
    /// Default 64 ≈ 50 MB and ≈ 0.7 s of lag.
    pub channel_capacity: usize,
    /// Table of keV coefficients for the display (TODO 83). None = ADC only.
    pub energy_calib_file: Option<String>,
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
            userinfo_histogram_config: HistogramConfig {
                num_bins: 16384,
                min_value: 0.0,
                max_value: 16384.0,
            },
            histogram2d_overrides: HashMap::new(),
            channel_capacity: 64,
            energy_calib_file: None,
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

    #[error("Receive thread error: {0}")]
    Thread(String),
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
    pub fn new(
        module_id: u32,
        channel_id: u32,
        x_config: HistogramConfig,
        y_config: HistogramConfig,
    ) -> Self {
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
        // TODO 58 L3: clamp — f32 rounding can push a value just below max to
        // bin == num_bins, which previously spilled into column 0 of the NEXT
        // y-row via the flat index (silent mis-binning, not overflow).
        let x_bin = (((x - self.x_config.min_value) / x_bin_width) as usize)
            .min(self.x_config.num_bins as usize - 1);

        let y_range = self.y_config.max_value - self.y_config.min_value;
        let y_bin_width = y_range / self.y_config.num_bins as f32;
        let y_bin = (((y - self.y_config.min_value) / y_bin_width) as usize)
            .min(self.y_config.num_bins as usize - 1);

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

/// One on-demand 2D histogram entry. `last_accessed` drives the TTL evictor
/// (see `evict_stale_plots`).
#[derive(Debug)]
pub struct PlotEntry {
    pub hist: Histogram2D,
    pub last_accessed: Instant,
}

/// Monitor state containing all histograms (owned by histogram task)
#[derive(Debug, Default)]
pub struct MonitorState {
    pub histograms: HashMap<ChannelKey, Histogram1D>,
    pub psd_histograms: HashMap<ChannelKey, Histogram1D>,
    /// 1D histograms for `UserInfo[0..=3]` (and any other `AxisSource` we care
    /// to expose later). Pre-created on register for the registered channels
    /// and refilled on every event whose `extract()` returns Some — same
    /// pattern as `psd_histograms`, just keyed by `AxisSource` so the four
    /// AMax slots share storage. Always-on (no TTL): a couple of channels ×
    /// four slots stays well under a megabyte.
    pub userinfo_histograms: HashMap<(ChannelKey, AxisSource), Histogram1D>,
    /// On-demand 2D histograms keyed by `(channel, x_axis, y_axis)`. Created
    /// lazily on first REST `GET /api/histograms2d/...?x=&y=` and evicted by
    /// `evict_stale_plots` after the TTL expires.
    pub histograms2d: HashMap<(ChannelKey, AxisSource, AxisSource), PlotEntry>,
    pub latest_waveforms: HashMap<ChannelKey, LatestWaveform>,
    pub total_events: u64,
    pub start_time: Option<Instant>,
    pub histogram_config: HistogramConfig,
    pub psd_histogram_config: HistogramConfig,
    /// 1D `UserInfo[i]` histogram config (mirrors `MonitorConfig`).
    pub userinfo_histogram_config: HistogramConfig,
    /// Per-axis overrides for 2D plot ranges (see `MonitorConfig`).
    pub histogram2d_overrides: HashMap<AxisSource, HistogramConfig>,
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
            userinfo_histograms: HashMap::new(),
            histograms2d: HashMap::new(),
            latest_waveforms: HashMap::new(),
            total_events: 0,
            start_time: None,
            histogram_config: config.histogram_config,
            psd_histogram_config: config.psd_histogram_config,
            userinfo_histogram_config: config.userinfo_histogram_config,
            histogram2d_overrides: config.histogram2d_overrides.clone(),
            registered_channels: Vec::new(),
            channel_names: HashMap::new(),
        }
    }

    /// Resolve the histogram axis configuration for a single `AxisSource`,
    /// preferring an explicit override and falling back to
    /// `AxisSource::default_axis()`.
    fn axis_config(&self, axis: AxisSource) -> HistogramConfig {
        if let Some(cfg) = self.histogram2d_overrides.get(&axis) {
            return *cfg;
        }
        let (min, max, bins) = axis.default_axis();
        HistogramConfig {
            num_bins: bins,
            min_value: min,
            max_value: max,
        }
    }

    /// Look up (or create) the 2D plot for `(key, x, y)` and return an
    /// immutable reference to the histogram. Updates `last_accessed` so the
    /// plot survives the TTL evictor on the next sweep.
    pub fn ensure_plot(&mut self, key: ChannelKey, x: AxisSource, y: AxisSource) -> &Histogram2D {
        let plot_key = (key, x, y);
        if !self.histograms2d.contains_key(&plot_key) {
            let x_cfg = self.axis_config(x);
            let y_cfg = self.axis_config(y);
            let hist = Histogram2D::new(key.module_id, key.channel_id, x_cfg, y_cfg);
            self.histograms2d.insert(
                plot_key,
                PlotEntry {
                    hist,
                    last_accessed: Instant::now(),
                },
            );
        }
        let entry = self.histograms2d.get_mut(&plot_key).expect("just inserted");
        entry.last_accessed = Instant::now();
        &entry.hist
    }

    /// Drop 2D plots whose `last_accessed` is older than `ttl`. Returns the
    /// number of evicted entries (for the caller's logging).
    pub fn evict_stale_plots(&mut self, ttl: std::time::Duration) -> usize {
        let now = Instant::now();
        let before = self.histograms2d.len();
        self.histograms2d
            .retain(|_, entry| now.duration_since(entry.last_accessed) <= ttl);
        before - self.histograms2d.len()
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

        // 2. PSD 1D histogram (only when energy > 0 to avoid division by zero)
        if event.energy > 0 {
            let psd = (event.energy as f32 - event.energy_short as f32) / event.energy as f32;
            let psd_hist = self.psd_histograms.entry(key).or_insert_with(|| {
                Histogram1D::new(
                    event.module as u32,
                    event.channel as u32,
                    self.psd_histogram_config,
                )
            });
            psd_hist.fill(psd);
        }

        // 3. UserInfo 1D fills (AMax-style): same pre-created pattern as PSD.
        // Walk the per-channel slots; AxisSource::extract handles the cast.
        for axis in [
            AxisSource::UserInfo0,
            AxisSource::UserInfo1,
            AxisSource::UserInfo2,
            AxisSource::UserInfo3,
        ] {
            if let Some(hist) = self.userinfo_histograms.get_mut(&(key, axis)) {
                if let Some(v) = axis.extract(&event) {
                    hist.fill(v as f32);
                }
            }
        }

        // 4. 2D fills: walk the on-demand plot map and fill any plot for this
        // channel whose axes both extract a defined value. With at most a
        // handful of live plots per channel this stays O(N) where N is small.
        for ((plot_key, x_src, y_src), entry) in self.histograms2d.iter_mut() {
            if plot_key.module_id != key.module_id || plot_key.channel_id != key.channel_id {
                continue;
            }
            if let (Some(x), Some(y)) = (x_src.extract(&event), y_src.extract(&event)) {
                entry.hist.fill(x as f32, y as f32);
            }
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
        self.userinfo_histograms.clear();
        self.histograms2d.clear();
        self.latest_waveforms.clear();
        self.total_events = 0;
        // Re-create empty histograms for registered channels
        self.ensure_registered_histograms();
    }

    /// Full reset: clear everything including registered channels.
    pub fn reset(&mut self) {
        self.histograms.clear();
        self.psd_histograms.clear();
        self.userinfo_histograms.clear();
        self.histograms2d.clear();
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

    /// Ensure all registered channels have 1D histogram entries
    /// (Energy + PSD + UserInfo[0..=3]). 2D histograms are created lazily on
    /// first request — see `ensure_plot`.
    fn ensure_registered_histograms(&mut self) {
        for ch in &self.registered_channels {
            let key = ChannelKey::new(ch.module_id, ch.channel_id);
            self.histograms.entry(key).or_insert_with(|| {
                Histogram1D::new(ch.module_id, ch.channel_id, self.histogram_config)
            });
            self.psd_histograms.entry(key).or_insert_with(|| {
                Histogram1D::new(ch.module_id, ch.channel_id, self.psd_histogram_config)
            });
            for axis in [
                AxisSource::UserInfo0,
                AxisSource::UserInfo1,
                AxisSource::UserInfo2,
                AxisSource::UserInfo3,
            ] {
                // 1D UserInfo uses its own dedicated config (native ADC
                // resolution, 16384 bins by default), independent of the 2D
                // override map. Frontend rebins live via the slider.
                let cfg = self.userinfo_histogram_config;
                self.userinfo_histograms
                    .entry((key, axis))
                    .or_insert_with(|| Histogram1D::new(ch.module_id, ch.channel_id, cfg));
            }
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
    processed_events: AtomicU64,
    dropped_batches: AtomicU64,
}

impl AtomicStats {
    fn new() -> Self {
        Self {
            received_batches: AtomicU64::new(0),
            processed_batches: AtomicU64::new(0),
            processed_events: AtomicU64::new(0),
            dropped_batches: AtomicU64::new(0),
        }
    }

    #[inline]
    fn record_received(&self) {
        self.received_batches.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_processed(&self, event_count: u64) {
        self.processed_batches.fetch_add(1, Ordering::Relaxed);
        self.processed_events
            .fetch_add(event_count, Ordering::Relaxed);
    }

    #[inline]
    fn record_drop(&self) {
        self.dropped_batches.fetch_add(1, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.received_batches.store(0, Ordering::Relaxed);
        self.processed_batches.store(0, Ordering::Relaxed);
        self.processed_events.store(0, Ordering::Relaxed);
        self.dropped_batches.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.received_batches.load(Ordering::Relaxed),
            self.processed_events.load(Ordering::Relaxed),
            self.dropped_batches.load(Ordering::Relaxed),
        )
    }

    /// Batches received but neither processed nor dropped yet: the raw-frame
    /// channel occupancy as seen through the counters.
    fn queued_batches(&self) -> u64 {
        let received = self.received_batches.load(Ordering::Relaxed);
        let done = self.processed_batches.load(Ordering::Relaxed)
            + self.dropped_batches.load(Ordering::Relaxed);
        received.saturating_sub(done)
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
    /// Get a 1D `UserInfo[i]` histogram for a channel (i = 0..=3, encoded
    /// as the matching `AxisSource::UserInfo*` variant).
    GetUserInfoHistogram(ChannelKey, AxisSource, oneshot::Sender<Option<Histogram1D>>),
    /// Get a 2D histogram for `(channel, x_axis, y_axis)`. Creates the plot
    /// on-demand if it doesn't exist yet (returns the empty histogram so the
    /// frontend has something to render until events start arriving).
    Get2dHistogram(
        ChannelKey,
        AxisSource,
        AxisSource,
        oneshot::Sender<Histogram2D>,
    ),
    /// Run the TTL evictor (called periodically by a background task).
    EvictStalePlots(std::time::Duration),
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
    /// keV coefficients for the UI. Never reaches the histogram task (TODO 83).
    energy_calib: Arc<parking_lot::RwLock<ActiveCalib>>,
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
    /// Histogram type: "energy" (default), "psd", or "user_info0".."user_info3"
    /// (the AMax-style 63-bit user-info slots).
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
        "user_info0" => HistogramMessage::GetUserInfoHistogram(key, AxisSource::UserInfo0, tx),
        "user_info1" => HistogramMessage::GetUserInfoHistogram(key, AxisSource::UserInfo1, tx),
        "user_info2" => HistogramMessage::GetUserInfoHistogram(key, AxisSource::UserInfo2, tx),
        "user_info3" => HistogramMessage::GetUserInfoHistogram(key, AxisSource::UserInfo3, tx),
        _ => HistogramMessage::GetHistogram(key, tx),
    };
    let _ = state.histogram_tx.send(msg);

    match rx.await {
        Ok(Some(hist)) => Ok(Json(hist)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Query parameters for the 2D histogram endpoint.
///
/// Preferred form is `?x=<axis>&y=<axis>` where each axis is an
/// `AxisSource` snake_case literal. `?type=psd2d|amax2d` is kept as a
/// backward-compatible alias for the two Phase 1 fixed plots.
#[derive(Debug, Deserialize)]
struct Histogram2dQuery {
    x: Option<AxisSource>,
    y: Option<AxisSource>,
    /// Legacy alias — `psd2d` → (Energy, Psd), `amax2d` → (Energy, UserInfo0).
    /// Frontends should migrate to `?x=&y=`; we keep this so existing
    /// links/scripts don't break overnight.
    r#type: Option<String>,
}

/// Resolve `(x, y)` from the query, applying the legacy `?type=` alias.
/// Returns 400 BadRequest if the caller gave us neither axis pair nor a
/// recognised legacy alias.
fn resolve_axes(query: &Histogram2dQuery) -> Result<(AxisSource, AxisSource), StatusCode> {
    if let (Some(x), Some(y)) = (query.x, query.y) {
        return Ok((x, y));
    }
    match query.r#type.as_deref() {
        Some("psd2d") | None => Ok((AxisSource::Energy, AxisSource::Psd)),
        Some("amax2d") => Ok((AxisSource::Energy, AxisSource::UserInfo0)),
        Some(_) => Err(StatusCode::BAD_REQUEST),
    }
}

/// GET /api/histograms2d/:module/:channel?x=<axis>&y=<axis> — fetch a 2D
/// histogram for a channel. Plots are created on demand on first request and
/// kept alive while polled (TTL eviction in the background).
async fn get_histogram2d(
    State(state): State<AppState>,
    axum::extract::Path((module_id, channel_id)): axum::extract::Path<(u32, u32)>,
    axum::extract::Query(query): axum::extract::Query<Histogram2dQuery>,
) -> Result<Json<Histogram2D>, StatusCode> {
    let (x_axis, y_axis) = resolve_axes(&query)?;
    let (tx, rx) = oneshot::channel();
    let key = ChannelKey::new(module_id, channel_id);

    let _ = state
        .histogram_tx
        .send(HistogramMessage::Get2dHistogram(key, x_axis, y_axis, tx));

    match rx.await {
        Ok(hist) => Ok(Json(hist)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// POST /api/histograms/clear - Clear all histograms
async fn clear_histograms(State(state): State<AppState>) -> StatusCode {
    let _ = state.histogram_tx.send(HistogramMessage::Clear);
    info!("Histograms cleared");
    StatusCode::OK
}

/// GET /api/calibration - The keV coefficients in force and how they were read
async fn get_calibration(State(state): State<AppState>) -> Json<CalibResponse> {
    Json(state.energy_calib.read().response())
}

/// POST /api/calibration/reload - Read the file now, without starting a run.
///
/// A file that cannot be used is a state to show, not an HTTP error: the reply says
/// `stale` or `error` with the line numbers. 404 only if no file is configured.
async fn reload_calibration(
    State(state): State<AppState>,
) -> Result<Json<CalibResponse>, StatusCode> {
    let mut calib = state.energy_calib.write();
    if !calib.is_configured() {
        return Err(StatusCode::NOT_FOUND);
    }
    calib.reload();
    Ok(Json(calib.response()))
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
        .route(
            "/api/histograms2d/:module_id/:channel_id",
            get(get_histogram2d),
        )
        .route("/api/calibration", get(get_calibration))
        .route(
            "/api/calibration/reload",
            axum::routing::post(reload_calibration),
        )
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
    energy_calib: Arc<parking_lot::RwLock<ActiveCalib>>,
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
        self.atomic_stats.reset();
        // A bad file shows in GET /api/calibration; the run must start regardless.
        self.energy_calib.write().reload();
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
        let (_recv, proc, _drop) = self.atomic_stats.snapshot();
        Some(crate::common::ComponentMetrics {
            events_processed: proc,
            bytes_transferred: 0, // Monitor doesn't track bytes
            queue_size: self.atomic_stats.queued_batches() as u32,
            queue_max: 0,
            queue_bytes: 0,
            queue_bytes_peak: 0,
            backlog_level: 0,
            write_failed: false,
            unwritten_events: 0,
            disk_free_bytes: None,
            disk_level: 0,
            pll: None,
            event_rate: 0.0, // Will be calculated in Phase 2
            data_rate: 0.0,
            trigger_loss_count: 0,
            trigger_loss_rate: 0.0,
            channel_counts: None,
        })
    }
}

/// `RCVTIMEO` of the blocking SUB socket: how long the receive thread can go
/// without re-checking the stop flag and the component state.
const RECV_POLL_MS: i32 = 100;

/// Minimum spacing of the "batches skipped" warning from the receive thread.
const DROP_LOG_INTERVAL: Duration = Duration::from_secs(5);

/// Lets the blocking `zmq::Socket` go through the shared HWM=0 helper
/// (`sub_no_hwm`), which is written against tmq's `AsZmqSocket`.
struct RawSocket<'a>(&'a zmq::Socket);

impl AsZmqSocket for RawSocket<'_> {
    fn get_socket(&self) -> &zmq::Socket {
        self.0
    }
}

/// Monitor component
pub struct Monitor {
    config: MonitorConfig,
    shared_state: Arc<tokio::sync::Mutex<ComponentSharedState>>,
    atomic_stats: Arc<AtomicStats>,
    state_rx: watch::Receiver<ComponentState>,
    state_tx: watch::Sender<ComponentState>,
    energy_calib: Arc<parking_lot::RwLock<ActiveCalib>>,
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

        // A bad file does not stop the Monitor: it only changes what is on screen.
        let mut energy_calib = ActiveCalib::new(config.energy_calib_file.clone());
        if !energy_calib.is_configured() {
            info!("No energy calibration file: histograms are shown in ADC");
        }
        energy_calib.reload();

        Ok(Self {
            config,
            shared_state: Arc::new(tokio::sync::Mutex::new(ComponentSharedState::new())),
            atomic_stats: Arc::new(AtomicStats::new()),
            state_rx,
            state_tx,
            energy_calib: Arc::new(parking_lot::RwLock::new(energy_calib)),
        })
    }

    /// Get current state
    pub fn state(&self) -> ComponentState {
        *self.state_rx.borrow()
    }

    /// Run the monitor
    pub async fn run(&mut self, mut shutdown: broadcast::Receiver<()>) -> Result<(), MonitorError> {
        // Create channels
        let (hist_tx, hist_rx) = mpsc::unbounded_channel::<HistogramMessage>();
        // Bounded raw-frame channel: Monitor is display-only, frames are
        // dropped (counted) when full. This bounds the Monitor's memory.
        let data_channel_capacity = self.config.channel_capacity;
        let (data_tx, data_rx) = mpsc::channel::<zmq::Message>(data_channel_capacity);

        // Blocking ZMQ SUB socket, owned by the receive thread (see module
        // docs for why this is not a tmq socket on the runtime).
        let zmq_context = zmq::Context::new();
        let socket = zmq_context.socket(zmq::SUB).map_err(tmq::TmqError::from)?;
        socket
            .connect(&self.config.subscribe_address)
            .map_err(tmq::TmqError::from)?;
        socket.set_subscribe(b"").map_err(tmq::TmqError::from)?;
        // Never drop messages in ZMQ — buffer in memory instead (CLAUDE.md HWM=0)
        sub_no_hwm(&RawSocket(&socket)).map_err(tmq::TmqError::from)?;
        // Periodic timeout so the thread can notice shutdown and state changes.
        socket
            .set_rcvtimeo(RECV_POLL_MS)
            .map_err(tmq::TmqError::from)?;

        info!(
            address = %self.config.subscribe_address,
            "Monitor connected to upstream (RCVHWM=0)"
        );

        // Start HTTP server
        let app_state = AppState {
            histogram_tx: hist_tx.clone(),
            component_state: self.shared_state.clone(),
            energy_calib: self.energy_calib.clone(),
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
        let energy_calib_for_cmd = self.energy_calib.clone();

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
                        energy_calib: energy_calib_for_cmd.clone(),
                    };
                    handle_command(state, tx, cmd, Some(&mut ext))
                },
                "Monitor",
            )
            .await;
        });

        // Spawn the receive thread (plain OS thread, outside the runtime)
        let recv_stop = Arc::new(AtomicBool::new(false));
        let stop_for_recv = recv_stop.clone();
        let atomic_stats_for_recv = self.atomic_stats.clone();
        let state_rx_for_recv = self.state_rx.clone();
        let recv_thread = std::thread::Builder::new()
            .name("monitor-recv".to_string())
            .spawn(move || {
                Self::receive_thread(
                    socket,
                    data_tx,
                    stop_for_recv,
                    atomic_stats_for_recv,
                    state_rx_for_recv,
                )
            })
            .map_err(|e| MonitorError::Thread(e.to_string()))?;

        // Spawn histogram task
        let monitor_config_for_hist = self.config.clone();
        let atomic_stats_for_hist = self.atomic_stats.clone();
        let hist_handle = tokio::spawn(async move {
            Self::histogram_task(
                hist_rx,
                data_rx,
                monitor_config_for_hist,
                atomic_stats_for_hist,
            )
            .await
        });

        // Spawn TTL evictor: every 30s, drop on-demand 2D plots that haven't
        // been polled for >60s. Without this, idle browser tabs would keep
        // every (X, Y) pair alive forever.
        const PLOT_TTL: std::time::Duration = std::time::Duration::from_secs(60);
        const EVICT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
        let evictor_tx = hist_tx.clone();
        let mut evictor_shutdown = shutdown.resubscribe();
        let evict_handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(EVICT_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if evictor_tx
                            .send(HistogramMessage::EvictStalePlots(PLOT_TTL))
                            .is_err()
                        {
                            // histogram_task gone; nothing to do.
                            break;
                        }
                    }
                    _ = evictor_shutdown.recv() => break,
                }
            }
        });

        info!(state = %self.state(), "Monitor ready, waiting for commands");

        // Wait for shutdown signal
        let _ = shutdown.recv().await;
        info!("Monitor received shutdown signal");

        // Stop the receive thread first (it drops data_tx, which ends the
        // histogram task's data loop), then wait for the tasks.
        recv_stop.store(true, Ordering::Relaxed);
        let joined = tokio::task::spawn_blocking(move || recv_thread.join()).await;
        if !matches!(joined, Ok(Ok(()))) {
            warn!("Monitor receive thread did not shut down cleanly");
        }
        let _ = hist_handle.await;
        let _ = evict_handle.await;
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

    /// Receive thread: blocking ZMQ SUB → bounded channel of raw frames.
    ///
    /// Runs on its own OS thread, never on a tokio worker (module docs).
    /// Always drains the socket so the HWM=0 pipe cannot grow:
    /// - not Running: frames are discarded;
    /// - Running, channel has room: the raw frame is handed over untouched
    ///   (deserialization is the histogram task's job);
    /// - Running, channel full: the frame is dropped before deserialization,
    ///   counted in `dropped_batches` and reported in a rate-limited warning.
    ///
    /// `RCVTIMEO` bounds every `recv` so the loop re-checks `stop` and the
    /// component state at least every `RECV_POLL_MS`.
    fn receive_thread(
        socket: zmq::Socket,
        tx: mpsc::Sender<zmq::Message>,
        stop: Arc<AtomicBool>,
        atomic_stats: Arc<AtomicStats>,
        mut state_rx: watch::Receiver<ComponentState>,
    ) {
        let mut is_running = *state_rx.borrow() == ComponentState::Running;
        let mut dropped_since_log = 0u64;
        let mut last_drop_log = Instant::now();
        info!("Monitor receive thread started");

        while !stop.load(Ordering::Relaxed) {
            if state_rx.has_changed().unwrap_or(false) {
                let current = *state_rx.borrow_and_update();
                is_running = current == ComponentState::Running;
                info!(state = %current, "Monitor receiver state changed");
            }

            let frame = match socket.recv_msg(0) {
                Ok(frame) => frame,
                // RCVTIMEO expired or interrupted: loop to re-check the flags
                Err(zmq::Error::EAGAIN) | Err(zmq::Error::EINTR) => continue,
                Err(e) => {
                    warn!(error = %e, "ZMQ receive error");
                    continue;
                }
            };
            // Only the first part of a multipart carries the message
            // (same as before); swallow any extra parts.
            while frame.get_more() {
                if socket.recv_msg(0).is_err() {
                    break;
                }
            }

            // Not running: discard to keep the ZMQ pipe empty
            if !is_running {
                continue;
            }

            // Lightweight header parse (no allocation, no deserialization)
            match MessageHeader::parse(&frame) {
                Some(MessageHeader::Data { .. }) => {
                    atomic_stats.record_received();
                    match tx.try_send(frame) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            // Consumer is behind: drop the whole frame before
                            // deserializing it. Display only; counted.
                            atomic_stats.record_drop();
                            dropped_since_log += 1;
                            if last_drop_log.elapsed() >= DROP_LOG_INTERVAL {
                                let (recv, _proc, dropped) = atomic_stats.snapshot();
                                warn!(
                                    dropped_recently = dropped_since_log,
                                    dropped_total = dropped,
                                    received_total = recv,
                                    "Monitor behind the data rate: batches skipped before \
                                     deserialization (display only, recording unaffected)"
                                );
                                dropped_since_log = 0;
                                last_drop_log = Instant::now();
                            }
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            info!("Histogram channel closed, exiting");
                            break;
                        }
                    }
                }
                Some(MessageHeader::EndOfStream { source_id }) => {
                    let (recv, proc, dropped) = atomic_stats.snapshot();
                    info!(
                        source_id,
                        received_batches = recv,
                        processed_events = proc,
                        dropped_batches = dropped,
                        "Received EOS - data stream complete"
                    );
                }
                Some(MessageHeader::Heartbeat { source_id }) => {
                    debug!(source_id, "Received heartbeat");
                }
                None => {
                    warn!("Failed to parse message header");
                }
            }
        }

        if dropped_since_log > 0 {
            let (recv, _proc, dropped) = atomic_stats.snapshot();
            warn!(
                dropped_recently = dropped_since_log,
                dropped_total = dropped,
                received_total = recv,
                "Monitor skipped batches before stopping (display only)"
            );
        }
        info!("Monitor receive thread stopped");
    }

    /// Histogram task: owns MonitorState, decodes raw frames into batches,
    /// fills histograms and answers HTTP queries.
    async fn histogram_task(
        mut cmd_rx: mpsc::UnboundedReceiver<HistogramMessage>,
        mut data_rx: mpsc::Receiver<zmq::Message>,
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
                        Some(HistogramMessage::GetUserInfoHistogram(key, axis, tx)) => {
                            let _ = tx.send(state.userinfo_histograms.get(&(key, axis)).cloned());
                        }
                        Some(HistogramMessage::Get2dHistogram(key, x, y, tx)) => {
                            // ensure_plot creates the entry on demand and bumps
                            // last_accessed; the response is always a (possibly
                            // empty) histogram so the frontend has something to
                            // render.
                            let hist = state.ensure_plot(key, x, y).clone();
                            let _ = tx.send(hist);
                        }
                        Some(HistogramMessage::EvictStalePlots(ttl)) => {
                            let evicted = state.evict_stale_plots(ttl);
                            if evicted > 0 {
                                debug!(evicted, "evicted stale 2D plot(s)");
                            }
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

                // Raw frames from the receive thread: decode here, on the
                // consumer side, so the receive thread stays at wire speed.
                frame = data_rx.recv() => {
                    match frame {
                        Some(frame) => {
                            match Message::from_msgpack(&frame) {
                                Ok(Message::Data(batch)) => {
                                    let event_count = batch.events.len() as u64;
                                    state.process_batch(batch);
                                    atomic_stats.record_processed(event_count);
                                }
                                Ok(_) => {
                                    warn!("Header said Data but payload is not a batch");
                                }
                                Err(e) => {
                                    warn!(error = %e, "Failed to deserialize batch");
                                }
                            }
                            drop(frame);
                            // ~10 ms of CPU per batch: hand the worker back so
                            // HTTP handlers and commands are never queued
                            // behind a run of batches.
                            tokio::task::yield_now().await;
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

    fn calib_state(path: Option<String>) -> AppState {
        let (histogram_tx, _rx) = mpsc::unbounded_channel();
        AppState {
            histogram_tx,
            component_state: Arc::new(tokio::sync::Mutex::new(ComponentSharedState::new())),
            energy_calib: Arc::new(parking_lot::RwLock::new(ActiveCalib::new(path))),
        }
    }

    #[tokio::test]
    async fn test_calibration_reload_without_a_file_is_not_found() {
        let state = calib_state(None);
        assert_eq!(
            reload_calibration(State(state.clone())).await.err(),
            Some(StatusCode::NOT_FOUND)
        );
        let Json(response) = get_calibration(State(state)).await;
        assert!(!response.configured);
    }

    #[tokio::test]
    async fn test_calibration_reload_reports_a_bad_file_as_a_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("calib.txt");
        std::fs::write(&path, "0 1 0.5 -10\n").expect("write");
        let state = calib_state(Some(path.to_string_lossy().into_owned()));

        let Ok(Json(response)) = reload_calibration(State(state.clone())).await else {
            panic!("expected the table");
        };
        assert_eq!(response.status, energy_calib::CalibStatus::Ok);
        assert_eq!(response.entries.len(), 1);

        // A broken file is a 200 that says stale, with the old table still there.
        std::fs::write(&path, "0 1 -1 0\n").expect("write");
        let Ok(Json(response)) = reload_calibration(State(state.clone())).await else {
            panic!("expected a state, not an HTTP error");
        };
        assert_eq!(response.status, energy_calib::CalibStatus::Stale);
        assert_eq!(response.entries.len(), 1);
        assert!(!response.errors.is_empty());

        let Json(response) = get_calibration(State(state)).await;
        assert_eq!(response.status, energy_calib::CalibStatus::Stale);
    }

    #[test]
    fn test_run_start_rereads_the_calibration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("calib.txt");
        std::fs::write(&path, "0 1 0.5 -10\n").expect("write");
        let (histogram_tx, _rx) = mpsc::unbounded_channel();
        let energy_calib = Arc::new(parking_lot::RwLock::new(ActiveCalib::new(Some(
            path.to_string_lossy().into_owned(),
        ))));
        let mut ext = MonitorCommandExt {
            histogram_tx,
            atomic_stats: Arc::new(AtomicStats::new()),
            energy_calib: energy_calib.clone(),
        };

        ext.on_start(1).expect("start");
        assert_eq!(energy_calib.read().response().entries[0].kev_per_adc, 0.5);

        std::fs::write(&path, "0 1 0.75 -10\n").expect("write");
        ext.on_start(2).expect("start");
        assert_eq!(energy_calib.read().response().entries[0].kev_per_adc, 0.75);

        // A bad file must not stop the run.
        std::fs::write(&path, "broken\n").expect("write");
        ext.on_start(3).expect("a bad table must not stop the run");
        assert_eq!(energy_calib.read().response().entries[0].kev_per_adc, 0.75);
    }

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
            user_info: [0; 4],
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
        let key = ChannelKey::new(0, 0);

        // Pre-register the 2D plot so the frontend's "polling" is simulated —
        // 2D histograms are now on-demand and only the actively-requested
        // (X, Y) pairs get filled.
        state.ensure_plot(key, AxisSource::Energy, AxisSource::Psd);

        // Event with energy=1000, energy_short=300 → PSD = (1000-300)/1000 = 0.7
        let event = EventData {
            module: 0,
            channel: 0,
            energy: 1000,
            energy_short: 300,
            timestamp_ns: 0.0,
            flags: 0,
            user_info: [0; 4],
            waveform: None,
        };
        state.process_event(event);

        // 1D PSD histogram should have 1 entry
        let psd_hist = state.psd_histograms.get(&key).unwrap();
        assert_eq!(psd_hist.total_counts, 1);

        // 2D histogram for (Energy, Psd) should have 1 entry
        let entry = state
            .histograms2d
            .get(&(key, AxisSource::Energy, AxisSource::Psd))
            .expect("2D plot was pre-registered");
        assert_eq!(entry.hist.total_counts, 1);
    }

    #[test]
    fn test_psd_skipped_for_zero_energy() {
        let config = MonitorConfig::default();
        let mut state = MonitorState::new(&config);
        let key = ChannelKey::new(0, 0);

        // Pre-register the (Energy, Psd) plot — the test asserts that a
        // zero-energy event leaves both the 1D PSD and the 2D plot empty
        // (Psd extraction returns None when energy == 0).
        state.ensure_plot(key, AxisSource::Energy, AxisSource::Psd);

        let event = EventData {
            module: 0,
            channel: 0,
            energy: 0,
            energy_short: 0,
            timestamp_ns: 0.0,
            flags: 0,
            user_info: [0; 4],
            waveform: None,
        };
        state.process_event(event);

        // Energy histogram should still have the event
        assert_eq!(state.histograms.get(&key).unwrap().total_counts, 1);

        // 1D PSD histogram is lazy-created on first non-zero energy event, so
        // it shouldn't exist.
        assert!(!state.psd_histograms.contains_key(&key));

        // The (Energy, Psd) 2D plot exists (we pre-registered it) but its
        // total_counts is 0 because Psd.extract returns None for energy == 0.
        let entry = state
            .histograms2d
            .get(&(key, AxisSource::Energy, AxisSource::Psd))
            .expect("2D plot was pre-registered");
        assert_eq!(entry.hist.total_counts, 0);
    }

    #[test]
    fn test_ensure_plot_lazy_creation_and_axis_orthogonality() {
        // Two different (X, Y) pairs for the same channel each get their own
        // independent plot — the axis pair is part of the storage key.
        let config = MonitorConfig::default();
        let mut state = MonitorState::new(&config);
        let key = ChannelKey::new(0, 0);

        state.ensure_plot(key, AxisSource::Energy, AxisSource::Psd);
        state.ensure_plot(key, AxisSource::Energy, AxisSource::UserInfo0);
        assert_eq!(state.histograms2d.len(), 2);

        // Event with energy=1000, energy_short=400 → psd=0.6, user_info[0]=42
        let event = EventData {
            module: 0,
            channel: 0,
            energy: 1000,
            energy_short: 400,
            timestamp_ns: 0.0,
            flags: 0,
            user_info: [42, 0, 0, 0],
            waveform: None,
        };
        state.process_event(event);

        // Both plots filled — extract on different axes.
        for axes in [
            (AxisSource::Energy, AxisSource::Psd),
            (AxisSource::Energy, AxisSource::UserInfo0),
        ] {
            let entry = state.histograms2d.get(&(key, axes.0, axes.1)).unwrap();
            assert_eq!(entry.hist.total_counts, 1, "{:?}", axes);
        }
    }

    #[test]
    fn test_userinfo_histograms_filled_on_event() {
        let config = MonitorConfig::default();
        let mut state = MonitorState::new(&config);
        let key = ChannelKey::new(0, 0);

        // Pre-register so the four UserInfo slots get pre-created.
        state.register_channels(vec![ChannelRegistration {
            module_id: 0,
            channel_id: 0,
            name: "ch0".into(),
        }]);
        for axis in [
            AxisSource::UserInfo0,
            AxisSource::UserInfo1,
            AxisSource::UserInfo2,
            AxisSource::UserInfo3,
        ] {
            assert!(state.userinfo_histograms.contains_key(&(key, axis)));
        }

        let event = EventData {
            module: 0,
            channel: 0,
            energy: 100,
            energy_short: 80,
            timestamp_ns: 0.0,
            flags: 0,
            user_info: [42, 17, 9, 3],
            waveform: None,
        };
        state.process_event(event);

        let h0 = state
            .userinfo_histograms
            .get(&(key, AxisSource::UserInfo0))
            .unwrap();
        let h1 = state
            .userinfo_histograms
            .get(&(key, AxisSource::UserInfo1))
            .unwrap();
        assert_eq!(h0.total_counts, 1);
        assert_eq!(h1.total_counts, 1);
        // 1D UserInfo defaults to native (1 bin per ADC count, 16384 bins on
        // [0, 16384)); slot 0 (=42) lands in bin 42.
        let cfg = config.userinfo_histogram_config;
        let bin = ((42.0 - cfg.min_value) / (cfg.max_value - cfg.min_value) * cfg.num_bins as f32)
            as usize;
        assert_eq!(h0.bins[bin], 1);
    }

    #[test]
    fn test_evict_stale_plots() {
        use std::time::Duration;

        let config = MonitorConfig::default();
        let mut state = MonitorState::new(&config);
        let key = ChannelKey::new(0, 0);

        state.ensure_plot(key, AxisSource::Energy, AxisSource::Psd);
        assert_eq!(state.histograms2d.len(), 1);

        // TTL of 1h: nothing evicted (entry is fresh).
        assert_eq!(state.evict_stale_plots(Duration::from_secs(3600)), 0);
        assert_eq!(state.histograms2d.len(), 1);

        // Force the entry's last_accessed into the past so the TTL kicks in.
        let plot_key = (key, AxisSource::Energy, AxisSource::Psd);
        state.histograms2d.get_mut(&plot_key).unwrap().last_accessed =
            Instant::now() - Duration::from_secs(120);

        // TTL of 60s: stale entry gets evicted.
        assert_eq!(state.evict_stale_plots(Duration::from_secs(60)), 1);
        assert!(state.histograms2d.is_empty());
    }

    #[test]
    fn test_histogram2d_fill() {
        let x_config = HistogramConfig {
            num_bins: 10,
            min_value: 0.0,
            max_value: 100.0,
        };
        let y_config = HistogramConfig {
            num_bins: 5,
            min_value: 0.0,
            max_value: 1.0,
        };
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
        stats.record_processed(10);
        stats.record_drop();

        let (recv, events, drop) = stats.snapshot();
        assert_eq!(recv, 2);
        assert_eq!(events, 10);
        assert_eq!(drop, 1);
    }
}

/// Overload reproduction (TODO 84 §8): the Merger PUB at 2.8-5 MHz made the
/// histogram consumer stop for 8-16 s while the receiver stayed at 100 %.
/// This drives the whole Monitor (commands over its REP socket, data over a
/// real PUB) and watches the processed counter from outside.
#[cfg(test)]
mod overload_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use crate::common::RunConfig;

    const EVENTS_PER_BATCH: usize = 33_000;
    /// 300 batches/s ≈ 10 M events/s: well above what one thread can decode.
    const BATCHES_PER_SEC: u64 = 300;
    const OBSERVE: Duration = Duration::from_secs(4);
    const SAMPLE: Duration = Duration::from_millis(100);

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr")
            .port()
    }

    /// One batch shaped like the Merger's output at high rate: many events,
    /// no waveforms, a handful of channels on two modules.
    fn batch_bytes(n: usize) -> Vec<u8> {
        let mut batch = EventDataBatch::with_capacity(0, 1, n);
        for i in 0..n {
            let mut ev = EventData::zeroed();
            ev.module = (i % 2) as u8;
            ev.channel = (i % 16) as u8;
            ev.energy = (i % 60_000) as u16;
            ev.energy_short = ev.energy / 2;
            ev.timestamp_ns = i as f64 * 100.0;
            batch.push(ev);
        }
        Message::Data(batch).to_msgpack().expect("msgpack")
    }

    fn send_command(ctx: &zmq::Context, addr: &str, cmd: &Command) -> CommandResponse {
        let req = ctx.socket(zmq::REQ).expect("REQ socket");
        req.set_rcvtimeo(10_000).expect("rcvtimeo");
        req.set_linger(0).expect("linger");
        req.connect(addr).expect("connect");
        req.send(&cmd.to_json().expect("json")[..], 0)
            .expect("send command");
        let reply = req.recv_bytes(0).expect("command reply");
        CommandResponse::from_json(&reply).expect("parse reply")
    }

    /// Longest run of samples in which `processed` did not move while
    /// `received` did, in seconds.
    fn longest_stall(samples: &[(f64, u64, u64)]) -> f64 {
        let mut longest = 0.0f64;
        let mut stall_start: Option<f64> = None;
        for w in samples.windows(2) {
            let (t0, recv0, proc0) = w[0];
            let (t1, recv1, proc1) = w[1];
            let flowing = recv1 > recv0;
            if flowing && proc1 == proc0 {
                let start = *stall_start.get_or_insert(t0);
                longest = longest.max(t1 - start);
            } else {
                stall_start = None;
            }
        }
        longest
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_consumer_keeps_progressing_under_overload() {
        let zctx = zmq::Context::new();
        let publisher = zctx.socket(zmq::PUB).expect("PUB socket");
        publisher.set_sndhwm(0).expect("sndhwm");
        publisher.set_linger(0).expect("linger");
        publisher.bind("tcp://127.0.0.1:*").expect("bind PUB");
        let sub_addr = publisher
            .get_last_endpoint()
            .expect("endpoint")
            .expect("utf8 endpoint");

        let cmd_addr = format!("tcp://127.0.0.1:{}", free_port());
        let http_port = free_port();
        let config = MonitorConfig {
            subscribe_address: sub_addr,
            command_address: cmd_addr.clone(),
            http_port,
            ..MonitorConfig::default()
        };
        let mut monitor = Monitor::new(config).await.expect("monitor");
        let stats = monitor.atomic_stats.clone();
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let run_handle = tokio::spawn(async move { monitor.run(shutdown_rx).await });

        // Idle → Configured → Armed → Running, like the Operator does.
        let ctx_for_cmd = zctx.clone();
        let addr_for_cmd = cmd_addr.clone();
        tokio::task::spawn_blocking(move || {
            for cmd in [
                Command::Configure(RunConfig {
                    run_number: 1,
                    comment: String::new(),
                    exp_name: "overload_test".to_string(),
                }),
                Command::Arm,
                Command::Start { run_number: 1 },
            ] {
                let reply = send_command(&ctx_for_cmd, &addr_for_cmd, &cmd);
                assert!(reply.success, "{cmd} failed: {}", reply.message);
            }
        })
        .await
        .expect("command thread");

        // Paced blaster: a pre-serialized batch, no per-send allocation.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_pub = stop.clone();
        let bytes = batch_bytes(EVENTS_PER_BATCH);
        let blaster = std::thread::spawn(move || {
            let period = Duration::from_micros(1_000_000 / BATCHES_PER_SEC);
            let mut next = Instant::now();
            let mut sent = 0u64;
            while !stop_for_pub.load(Ordering::Relaxed) {
                publisher.send(&bytes[..], 0).expect("send batch");
                sent += 1;
                next += period;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
            sent
        });

        // Sample (t, received, processed) while data flows.
        let t0 = Instant::now();
        let mut samples = Vec::new();
        let mut max_status_latency = Duration::ZERO;
        let client = reqwest::Client::new();
        let status_url = format!("http://127.0.0.1:{http_port}/api/status");
        let mut ticks = 0u32;
        while t0.elapsed() < OBSERVE {
            tokio::time::sleep(SAMPLE).await;
            let (recv, proc, _drop) = stats.snapshot();
            samples.push((t0.elapsed().as_secs_f64(), recv, proc));
            ticks += 1;
            if ticks.is_multiple_of(5) {
                let started = Instant::now();
                let _ =
                    tokio::time::timeout(Duration::from_secs(2), client.get(&status_url).send())
                        .await;
                max_status_latency = max_status_latency.max(started.elapsed());
            }
        }

        stop.store(true, Ordering::Relaxed);
        let sent = blaster.join().expect("blaster");
        let (received, processed, dropped) = stats.snapshot();
        let stall = longest_stall(&samples);
        eprintln!(
            "sent={sent} received={received} processed_events={processed} dropped={dropped} \
             longest_stall={stall:.2}s max_status_latency={max_status_latency:?} \
             samples={} ticks={ticks}",
            samples.len()
        );

        let _ = shutdown_tx.send(());
        if tokio::time::timeout(Duration::from_secs(10), run_handle)
            .await
            .is_err()
        {
            eprintln!("Monitor::run did not return within 10 s after shutdown");
        }

        assert!(received > 0, "no batches received");
        assert!(processed > 0, "consumer never processed a batch");
        // The runtime's timers must keep firing too (the old receiver took
        // the I/O+time driver down with it: one 100 ms sleep lasted 33 s).
        assert!(
            samples.len() >= 20,
            "sampler got only {} samples in {OBSERVE:?}: runtime timers were starved",
            samples.len()
        );
        assert!(
            stall < 1.0,
            "histogram consumer stalled for {stall:.2} s while data was flowing"
        );
        assert!(
            max_status_latency < Duration::from_secs(1),
            "GET /api/status took {max_status_latency:?}"
        );
    }

    /// The receive thread alone, on an inproc pair: discard while not
    /// Running, hand over raw frames while Running, count drops when the
    /// channel is full, ignore EOS/heartbeat for the counters.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_receive_thread_discards_counts_and_drops() {
        let zctx = zmq::Context::new();
        let publisher = zctx.socket(zmq::PUB).expect("PUB");
        publisher.bind("inproc://monitor-recv-test").expect("bind");
        let sub = zctx.socket(zmq::SUB).expect("SUB");
        sub.connect("inproc://monitor-recv-test").expect("connect");
        sub.set_subscribe(b"").expect("subscribe");
        sub.set_rcvtimeo(20).expect("rcvtimeo");

        let (tx, mut rx) = mpsc::channel::<zmq::Message>(2);
        let stats = Arc::new(AtomicStats::new());
        let (state_tx, state_rx) = watch::channel(ComponentState::Idle);
        let stop = Arc::new(AtomicBool::new(false));
        let stats_for_thread = stats.clone();
        let stop_for_thread = stop.clone();
        let thread = std::thread::spawn(move || {
            Monitor::receive_thread(sub, tx, stop_for_thread, stats_for_thread, state_rx)
        });
        // Let the inproc subscription propagate before the first send.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let data = batch_bytes(10);
        // Idle: data is discarded without touching the counters.
        publisher.send(&data[..], 0).expect("send idle");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(stats.snapshot(), (0, 0, 0));
        assert!(rx.try_recv().is_err());

        // Running: 3 batches into a channel of 2, plus EOS and heartbeat.
        state_tx.send(ComponentState::Running).expect("state");
        tokio::time::sleep(Duration::from_millis(150)).await;
        for _ in 0..3 {
            publisher.send(&data[..], 0).expect("send data");
        }
        let eos = Message::EndOfStream {
            source_id: 7,
            run_number: 1,
        }
        .to_msgpack()
        .expect("eos");
        publisher.send(&eos[..], 0).expect("send eos");
        let heartbeat = Message::Heartbeat(crate::common::Heartbeat::new(7, 1))
            .to_msgpack()
            .expect("heartbeat");
        publisher.send(&heartbeat[..], 0).expect("send heartbeat");

        let deadline = Instant::now() + Duration::from_secs(5);
        while stats.snapshot().0 < 3 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (received, processed, dropped) = stats.snapshot();
        assert_eq!(received, 3, "every Data frame is counted as received");
        assert_eq!(processed, 0, "the receive thread never processes");
        assert_eq!(dropped, 1, "the third frame did not fit and was dropped");
        let first = rx.recv().await.expect("first frame");
        assert!(matches!(
            MessageHeader::parse(&first),
            Some(MessageHeader::Data { .. })
        ));
        assert!(rx.recv().await.is_some(), "second frame");

        stop.store(true, Ordering::Relaxed);
        thread.join().expect("receive thread");
        assert!(rx.recv().await.is_none(), "data_tx dropped with the thread");
    }

    /// The histogram task decodes raw frames itself, survives a frame that
    /// is not a batch, and still answers queries in between.
    #[tokio::test]
    async fn test_histogram_task_decodes_raw_frames() {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (data_tx, data_rx) = mpsc::channel::<zmq::Message>(8);
        let stats = Arc::new(AtomicStats::new());
        let handle = tokio::spawn(Monitor::histogram_task(
            cmd_rx,
            data_rx,
            MonitorConfig::default(),
            stats.clone(),
        ));

        data_tx
            .send(zmq::Message::from(&b"not a batch"[..]))
            .await
            .expect("send garbage");
        let batch = batch_bytes(10);
        data_tx
            .send(zmq::Message::from(&batch[..]))
            .await
            .expect("send batch");

        let deadline = Instant::now() + Duration::from_secs(5);
        while stats.snapshot().1 < 10 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(stats.snapshot().1, 10, "10 events processed");

        let (tx, rx) = oneshot::channel();
        cmd_tx
            .send(HistogramMessage::GetListSummary(tx))
            .expect("send query");
        let summary = rx.await.expect("summary");
        assert_eq!(summary.total_events, 10);

        drop(cmd_tx);
        drop(data_tx);
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("histogram task ends when its channels close")
            .expect("join");
    }
}

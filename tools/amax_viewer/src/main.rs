//! AMax Firmware Development Tool
//!
//! Real-time parameter adjustment and histogram viewer for AMax (user_info[0]) vs Energy.
//! Register definitions are loaded from register_defs.json (JSON-driven UI).

use clap::Parser;
use delila_rs::reader::CaenHandle;
use eframe::egui;
use egui_plot::{Line, Plot, PlotImage, PlotPoint, PlotPoints};
use oxyroot::{RootFile, WriterTree};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(about = "AMax Viewer - Firmware Development Tool")]
struct Args {
    /// Register definitions JSON file (e.g. registers/register_20260310.json)
    register_defs: Option<PathBuf>,

    /// Start in Test Pulse mode (use digitizer internal test pulse)
    #[arg(short = 't', long)]
    test_pulse: bool,

    /// Reset all register parameters to defaults (useful after firmware change)
    #[arg(long)]
    reset_params: bool,
}

// Default register definitions embedded at compile time.
// User can override by editing dirs::config_dir()/amax_viewer/register_defs.json
const DEFAULT_REGISTER_DEFS: &str = include_str!("../register_defs.json");

/// Register definition — loaded from register_defs.json
#[derive(Clone, Serialize, Deserialize)]
struct RegisterDef {
    /// UI section heading ("Core", "AMax", etc.)
    section: String,
    /// Register name — used as HashMap key and display label
    name: String,
    /// Word address (byte address = address * 4)
    address: u32,
    min: u32,
    max: u32,
    default: u32,
    /// Read-only status registers (skip writing, display as label)
    #[serde(default)]
    readonly: bool,
}

/// Event data for ROOT file output — all OpenDPP fields
#[derive(Default)]
struct EventBuffer {
    channel: Vec<i32>,
    energy: Vec<i32>,
    timestamp: Vec<i64>,
    fine_timestamp: Vec<i32>,
    flags_a: Vec<i32>,
    flags_b: Vec<i32>,
    psd: Vec<i32>,
    user_info_0: Vec<i64>,
    user_info_1: Vec<i64>,
    user_info_2: Vec<i64>,
    user_info_3: Vec<i64>,
    waveform: Vec<Vec<i32>>,
    waveform_size: Vec<i32>,
}

const MAX_BUFFER_BYTES: usize = 10 * 1_073_741_824; // 10 GB

impl EventBuffer {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        channel: u8,
        energy: u16,
        timestamp: u64,
        fine_timestamp: u16,
        flags_a: u16,
        flags_b: u16,
        psd: u16,
        user_info: &[u64],
        waveform: Option<&[u16]>,
    ) {
        self.channel.push(channel as i32);
        self.energy.push(energy as i32);
        self.timestamp.push(timestamp as i64);
        self.fine_timestamp.push(fine_timestamp as i32);
        self.flags_a.push(flags_a as i32);
        self.flags_b.push(flags_b as i32);
        self.psd.push(psd as i32);
        self.user_info_0
            .push(*user_info.first().unwrap_or(&0) as i64);
        self.user_info_1
            .push(*user_info.get(1).unwrap_or(&0) as i64);
        self.user_info_2
            .push(*user_info.get(2).unwrap_or(&0) as i64);
        self.user_info_3
            .push(*user_info.get(3).unwrap_or(&0) as i64);
        match waveform {
            Some(wf) => {
                self.waveform_size.push(wf.len() as i32);
                self.waveform.push(wf.iter().map(|&v| v as i32).collect());
            }
            None => {
                self.waveform_size.push(0);
                self.waveform.push(Vec::new());
            }
        }
    }

    fn len(&self) -> usize {
        self.energy.len()
    }

    fn clear(&mut self) {
        self.channel.clear();
        self.energy.clear();
        self.timestamp.clear();
        self.fine_timestamp.clear();
        self.flags_a.clear();
        self.flags_b.clear();
        self.psd.clear();
        self.user_info_0.clear();
        self.user_info_1.clear();
        self.user_info_2.clear();
        self.user_info_3.clear();
        self.waveform.clear();
        self.waveform_size.clear();
    }

    /// Estimated memory usage in bytes
    fn estimated_memory_bytes(&self) -> usize {
        let n = self.len();
        // Scalar fields: i32×7 + i64×5 = 68 bytes per event
        let scalar = n * 68;
        // Waveform: Vec overhead (24 bytes) + data (4 bytes per sample)
        let wf: usize = self.waveform.iter().map(|w| 24 + w.len() * 4).sum();
        scalar + wf
    }

    /// Write all events to ROOT file
    fn write_root(&self, path: &str) -> Result<usize, Box<dyn std::error::Error>> {
        if self.energy.is_empty() {
            return Ok(0);
        }

        let mut file = RootFile::create(path)?;
        let mut tree = WriterTree::new("events");

        tree.new_branch("channel", self.channel.clone().into_iter());
        tree.new_branch("energy", self.energy.clone().into_iter());
        tree.new_branch("timestamp", self.timestamp.clone().into_iter());
        tree.new_branch("fine_timestamp", self.fine_timestamp.clone().into_iter());
        tree.new_branch("flags_a", self.flags_a.clone().into_iter());
        tree.new_branch("flags_b", self.flags_b.clone().into_iter());
        tree.new_branch("psd", self.psd.clone().into_iter());
        tree.new_branch("user_info_0", self.user_info_0.clone().into_iter());
        tree.new_branch("user_info_1", self.user_info_1.clone().into_iter());
        tree.new_branch("user_info_2", self.user_info_2.clone().into_iter());
        tree.new_branch("user_info_3", self.user_info_3.clone().into_iter());
        tree.new_branch("waveform", self.waveform.clone().into_iter());
        tree.new_branch("waveform_size", self.waveform_size.clone().into_iter());

        tree.write(&mut file)?;
        file.close()?;

        Ok(self.energy.len())
    }
}

/// Deserialize selected_channel with backward compatibility (old Option<u8> null → 0)
fn deserialize_channel<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<u8>::deserialize(deserializer)?;
    Ok(opt.unwrap_or(0))
}

/// Application settings (persisted to settings.json)
#[derive(Clone, Serialize, Deserialize)]
struct AppSettings {
    #[serde(default = "AppSettings::default_url")]
    url: String,
    #[serde(default = "AppSettings::default_output_path")]
    output_path: String,
    /// Register values keyed by register name
    #[serde(default)]
    param_values: HashMap<String, u32>,
    /// Selected channel for display (0-31)
    #[serde(default, deserialize_with = "deserialize_channel")]
    selected_channel: u8,
    /// Hash of register_defs.json content — used to detect firmware changes
    #[serde(default)]
    register_defs_hash: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            url: Self::default_url(),
            output_path: Self::default_output_path(),
            param_values: HashMap::new(),
            selected_channel: 0,
            register_defs_hash: None,
        }
    }
}

impl AppSettings {
    fn default_url() -> String {
        "dig2://172.18.4.56".to_string()
    }

    fn default_output_path() -> String {
        "amax_data.root".to_string()
    }

    fn config_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|mut p| {
            p.push("amax_viewer");
            p
        })
    }

    fn settings_path() -> Option<PathBuf> {
        Self::config_dir().map(|mut p| {
            p.push("settings.json");
            p
        })
    }

    fn register_defs_path() -> Option<PathBuf> {
        Self::config_dir().map(|mut p| {
            p.push("register_defs.json");
            p
        })
    }

    fn load() -> Self {
        Self::settings_path()
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Some(path) = Self::settings_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(&path, json);
            }
        }
    }
}

/// Compute a hash of register definitions content for firmware change detection.
fn compute_defs_hash(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Load register definitions.
/// Priority: CLI argument > user config file (~/.config) > embedded default.
/// Returns (parsed defs, raw JSON content for hashing).
fn load_register_defs(cli_path: Option<&PathBuf>) -> (Vec<RegisterDef>, String) {
    // 1. CLI argument (highest priority)
    if let Some(path) = cli_path {
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Vec<RegisterDef>>(&content) {
                Ok(defs) => {
                    eprintln!(
                        "Loaded {} register defs from {}",
                        defs.len(),
                        path.display()
                    );
                    return (defs, content);
                }
                Err(e) => eprintln!("Failed to parse {}: {}", path.display(), e),
            },
            Err(e) => eprintln!("Failed to read {}: {}", path.display(), e),
        }
    }

    // 2. User config file
    if let Some(path) = AppSettings::register_defs_path() {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                // Check if embedded default has been updated (new binary build)
                let file_hash = compute_defs_hash(&content);
                let embedded_hash = compute_defs_hash(DEFAULT_REGISTER_DEFS);
                if file_hash != embedded_hash {
                    eprintln!(
                        "Updating config register_defs.json (embedded default changed)"
                    );
                    let _ = std::fs::write(&path, DEFAULT_REGISTER_DEFS);
                    if let Ok(defs) =
                        serde_json::from_str::<Vec<RegisterDef>>(DEFAULT_REGISTER_DEFS)
                    {
                        return (defs, DEFAULT_REGISTER_DEFS.to_string());
                    }
                } else if let Ok(defs) = serde_json::from_str::<Vec<RegisterDef>>(&content) {
                    return (defs, content);
                }
            }
        } else {
            // First run: copy embedded default to user config dir
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, DEFAULT_REGISTER_DEFS);
        }
    }

    // 3. Embedded default
    let defs = serde_json::from_str(DEFAULT_REGISTER_DEFS).unwrap_or_default();
    (defs, DEFAULT_REGISTER_DEFS.to_string())
}

/// Build param_values map from register defs (fill missing keys with defaults)
fn init_param_values(defs: &[RegisterDef], saved: &HashMap<String, u32>) -> HashMap<String, u32> {
    let mut values = HashMap::new();
    for reg in defs {
        let v = saved.get(&reg.name).copied().unwrap_or(reg.default);
        values.insert(reg.name.clone(), v);
    }
    values
}

/// Data source for 2D histogram axes
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlotSource {
    Energy,
    UserInfo0,
    UserInfo1,
    UserInfo2,
    UserInfo3,
}

impl PlotSource {
    const ALL: [PlotSource; 5] = [
        PlotSource::Energy,
        PlotSource::UserInfo0,
        PlotSource::UserInfo1,
        PlotSource::UserInfo2,
        PlotSource::UserInfo3,
    ];

    fn label(&self) -> &'static str {
        match self {
            PlotSource::Energy => "Energy",
            PlotSource::UserInfo0 => "user_info[0]",
            PlotSource::UserInfo1 => "user_info[1]",
            PlotSource::UserInfo2 => "user_info[2]",
            PlotSource::UserInfo3 => "user_info[3]",
        }
    }

    fn default_max(&self) -> f64 {
        match self {
            PlotSource::Energy => 65536.0,
            _ => 16384.0,
        }
    }

    fn extract(&self, energy: u16, user_info: &[u64]) -> f64 {
        match self {
            PlotSource::Energy => energy as f64,
            PlotSource::UserInfo0 => *user_info.first().unwrap_or(&0) as f64,
            PlotSource::UserInfo1 => *user_info.get(1).unwrap_or(&0) as f64,
            PlotSource::UserInfo2 => *user_info.get(2).unwrap_or(&0) as f64,
            PlotSource::UserInfo3 => *user_info.get(3).unwrap_or(&0) as f64,
        }
    }
}

/// 2D Histogram data
#[derive(Clone)]
struct Histogram2D {
    bins: Vec<Vec<u32>>,
    x_bins: usize,
    y_bins: usize,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    total_events: u64,
}

impl Histogram2D {
    fn new(x_bins: usize, y_bins: usize, x_max: f64, y_max: f64) -> Self {
        Self {
            bins: vec![vec![0u32; y_bins]; x_bins],
            x_bins,
            y_bins,
            x_min: 0.0,
            x_max,
            y_min: 0.0,
            y_max,
            total_events: 0,
        }
    }

    fn fill(&mut self, x: f64, y: f64) {
        if x >= self.x_min && x < self.x_max && y >= self.y_min && y < self.y_max {
            let x_bin =
                ((x - self.x_min) / (self.x_max - self.x_min) * self.x_bins as f64) as usize;
            let y_bin =
                ((y - self.y_min) / (self.y_max - self.y_min) * self.y_bins as f64) as usize;

            if x_bin < self.x_bins && y_bin < self.y_bins {
                self.bins[x_bin][y_bin] = self.bins[x_bin][y_bin].saturating_add(1);
                self.total_events += 1;
            }
        }
    }

    fn clear(&mut self) {
        for row in &mut self.bins {
            for bin in row {
                *bin = 0;
            }
        }
        self.total_events = 0;
    }

    fn resize(&mut self, x_bins: usize, y_bins: usize) {
        if x_bins != self.x_bins || y_bins != self.y_bins {
            self.x_bins = x_bins;
            self.y_bins = y_bins;
            self.bins = vec![vec![0u32; y_bins]; x_bins];
            self.total_events = 0;
        }
    }

    fn max_count(&self) -> u32 {
        self.bins
            .iter()
            .flat_map(|row| row.iter())
            .cloned()
            .max()
            .unwrap_or(1)
    }

    fn to_texture(&self) -> egui::ColorImage {
        let max = self.max_count().max(1) as f32;
        let mut pixels = Vec::with_capacity(self.x_bins * self.y_bins);

        for y_bin in (0..self.y_bins).rev() {
            for x_bin in 0..self.x_bins {
                let count = self.bins[x_bin][y_bin] as f32;
                let intensity = (count / max).sqrt();
                pixels.push(colormap(intensity));
            }
        }

        egui::ColorImage {
            size: [self.x_bins, self.y_bins],
            pixels,
        }
    }
}

/// Per-channel data storage (histogram + waveform + rate)
struct ChannelData {
    histogram: Histogram2D,
    waveform_buffer: Vec<u16>,
    waveform_len: usize,
    latest_waveform_energy: u16,
    histogram_dirty: bool,
    last_waveform_update: Instant,
    events_since_last_tick: u64,
    event_rate: f64,
}

impl ChannelData {
    fn new(x_bins: usize, y_bins: usize, x_max: f64, y_max: f64) -> Self {
        Self {
            histogram: Histogram2D::new(x_bins, y_bins, x_max, y_max),
            waveform_buffer: vec![0u16; 8192],
            waveform_len: 0,
            latest_waveform_energy: 0,
            histogram_dirty: true,
            last_waveform_update: Instant::now(),
            events_since_last_tick: 0,
            event_rate: 0.0,
        }
    }
}

fn colormap(t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.25 {
        let s = t / 0.25;
        egui::Color32::from_rgb(0, 0, (s * 255.0) as u8)
    } else if t < 0.5 {
        let s = (t - 0.25) / 0.25;
        egui::Color32::from_rgb(0, (s * 255.0) as u8, 255)
    } else if t < 0.75 {
        let s = (t - 0.5) / 0.25;
        egui::Color32::from_rgb((s * 255.0) as u8, 255, (255.0 * (1.0 - s)) as u8)
    } else {
        let s = (t - 0.75) / 0.25;
        egui::Color32::from_rgb(255, 255, (s * 255.0) as u8)
    }
}

/// Test pulse parameters (all SetInRun=true, can be changed during acquisition)
#[derive(Clone)]
struct TestPulseParams {
    period_ns: u32,  // TestPulsePeriod [ns]
    width_ns: u32,   // TestPulseWidth [ns]
    low_level: u32,  // TestPulseLowLevel [ADC count]
    high_level: u32, // TestPulseHighLevel [ADC count]
}

impl Default for TestPulseParams {
    fn default() -> Self {
        Self {
            period_ns: 1_000_000, // 1ms = 1kHz
            width_ns: 10,         // 10 ns
            low_level: 1000,
            high_level: 3000,
        }
    }
}

impl TestPulseParams {
    fn frequency_hz(&self) -> f64 {
        if self.period_ns == 0 {
            0.0
        } else {
            1.0e9 / self.period_ns as f64
        }
    }
}

/// Shared state between GUI and acquisition thread
struct SharedState {
    /// Per-channel histogram, waveform, and rate data
    channels: Vec<ChannelData>,
    param_values: HashMap<String, u32>,
    running: bool,
    /// Global event rate (all channels)
    event_rate: f64,
    connected: bool,
    status_message: String,
    event_buffer: EventBuffer,
    recording: bool,
    recorded_count: usize,
    /// Test pulse params — GUI writes, acq thread reads and applies via SetInRun
    test_pulse_params: TestPulseParams,
    /// Set to true by GUI when test_pulse_params changed; cleared by acq thread after applying
    test_pulse_params_dirty: bool,
    /// Runtime test pulse toggle state
    test_pulse_active: bool,
    /// Set to true by GUI when test_pulse_active toggled; cleared by acq thread after applying
    test_pulse_toggle_requested: bool,
    /// X-axis data source for 2D histogram
    x_source: PlotSource,
    /// Y-axis data source for 2D histogram
    y_source: PlotSource,
}

struct AmaxViewerApp {
    url: String,
    shared: Arc<Mutex<SharedState>>,
    shutdown: Arc<AtomicBool>,
    acq_thread: Option<thread::JoinHandle<()>>,
    texture: Option<egui::TextureHandle>,
    output_path: String,
    register_defs: Vec<RegisterDef>,
    test_pulse: bool,
    was_recording: bool,
    /// Selected channel for display (GUI-only state, not shared with acq thread)
    selected_channel: u8,
    /// Track previous channel to detect changes for texture regeneration
    prev_selected_channel: u8,
    /// Force-write all registers on next acquisition start (set on FW change or --reset-params)
    force_write_params: bool,
    /// Hash of current register_defs content (saved to settings on exit)
    register_defs_hash: String,
}

impl AmaxViewerApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        test_pulse: bool,
        register_defs_path: Option<PathBuf>,
        reset_params: bool,
    ) -> Self {
        let mut settings = AppSettings::load();
        let (register_defs, defs_content) = load_register_defs(register_defs_path.as_ref());
        let current_hash = compute_defs_hash(&defs_content);

        let fw_changed = settings.register_defs_hash.as_ref() != Some(&current_hash);
        let force_reset = reset_params || fw_changed;

        if force_reset {
            if fw_changed {
                eprintln!(
                    "Register definitions changed (firmware update detected). Parameters reset to defaults."
                );
            }
            if reset_params {
                eprintln!("--reset-params: Parameters reset to defaults.");
            }
            settings.param_values.clear();
        }

        let param_values = init_param_values(&register_defs, &settings.param_values);

        let x_source = PlotSource::Energy;
        let y_source = PlotSource::UserInfo0;
        let channels: Vec<ChannelData> = (0..32)
            .map(|_| {
                ChannelData::new(512, 512, x_source.default_max(), y_source.default_max())
            })
            .collect();

        let shared = Arc::new(Mutex::new(SharedState {
            channels,
            param_values,
            running: false,
            event_rate: 0.0,
            connected: false,
            status_message: if force_reset {
                "Parameters reset to defaults (firmware change detected)".to_string()
            } else if test_pulse {
                "Test Pulse mode - Not connected".to_string()
            } else {
                "Not connected".to_string()
            },
            event_buffer: EventBuffer::default(),
            recording: false,
            recorded_count: 0,
            test_pulse_params: TestPulseParams::default(),
            test_pulse_params_dirty: false,
            test_pulse_active: test_pulse,
            test_pulse_toggle_requested: false,
            x_source,
            y_source,
        }));

        let selected_channel = settings.selected_channel.min(31);

        Self {
            url: settings.url,
            shared,
            shutdown: Arc::new(AtomicBool::new(false)),
            acq_thread: None,
            texture: None,
            output_path: settings.output_path,
            register_defs,
            test_pulse,
            was_recording: false,
            selected_channel,
            prev_selected_channel: selected_channel,
            force_write_params: force_reset,
            register_defs_hash: current_hash,
        }
    }

    fn start_acquisition(&mut self) {
        if self.acq_thread.is_some() {
            return;
        }

        self.shutdown.store(false, Ordering::Relaxed);
        let shared = self.shared.clone();
        let shutdown = self.shutdown.clone();
        let url = self.url.clone();
        let register_defs = self.register_defs.clone();
        let test_pulse = self.test_pulse;
        let force_write = self.force_write_params;
        // Clear flag so subsequent reconnects don't force-write again
        self.force_write_params = false;

        self.acq_thread = Some(thread::spawn(move || {
            acquisition_thread(url, shared, shutdown, register_defs, test_pulse, force_write);
        }));
    }

    fn stop_acquisition(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.acq_thread.take() {
            let _ = handle.join();
        }
    }
}

impl eframe::App for AmaxViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(100));

        let mut start_clicked = false;
        let mut stop_clicked = false;
        let thread_active = self.acq_thread.is_some();

        // Bounds-clamp selected_channel to prevent panic on channels[] indexing
        // (could be out of range from corrupted settings file)
        self.selected_channel = self.selected_channel.min(31);

        egui::SidePanel::left("params_panel")
            .min_width(250.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut state = self.shared.lock().unwrap();

                    if state.test_pulse_active {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("TEST PULSE MODE")
                                .color(egui::Color32::from_rgb(255, 100, 0))
                                .heading()
                                .strong(),
                        );
                        ui.add_space(4.0);
                    }

                    ui.heading("Connection");
                    ui.text_edit_singleline(&mut self.url);

                    ui.horizontal(|ui| {
                        if state.running {
                            if ui.button("Stop").clicked() {
                                stop_clicked = true;
                            }
                        } else if ui.button("Start").clicked() {
                            start_clicked = true;
                        }

                        if ui.button("Clear").clicked() {
                            let ch = self.selected_channel as usize;
                            state.channels[ch].histogram.clear();
                            state.channels[ch].histogram_dirty = true;
                        }
                    });

                    // Test Pulse toggle (runtime)
                    {
                        let prev = state.test_pulse_active;
                        ui.checkbox(&mut state.test_pulse_active, "Test Pulse");
                        if state.test_pulse_active != prev {
                            state.test_pulse_toggle_requested = true;
                        }
                    }

                    ui.label(format!("Status: {}", state.status_message));
                    let ch = self.selected_channel as usize;
                    ui.label(format!(
                        "Events: {} (Ch {})",
                        state.channels[ch].histogram.total_events, self.selected_channel
                    ));
                    ui.label(format!(
                        "Rate: {:.1} Hz (Ch: {:.1} Hz)",
                        state.event_rate, state.channels[ch].event_rate
                    ));

                    // Channel selector
                    egui::ComboBox::from_label("Channel")
                        .selected_text(format!("Ch {}", self.selected_channel))
                        .show_ui(ui, |ui| {
                            for ch in 0..32u8 {
                                ui.selectable_value(
                                    &mut self.selected_channel,
                                    ch,
                                    format!("Ch {}", ch),
                                );
                            }
                        });

                    // Test Pulse parameters (shown only when active)
                    if state.test_pulse_active {
                        ui.separator();
                        egui::CollapsingHeader::new("Test Pulse Settings")
                            .default_open(true)
                            .show(ui, |ui| {
                                let mut changed = false;

                                ui.horizontal(|ui| {
                                    ui.label("Period:");
                                    if ui
                                        .add(
                                            egui::DragValue::new(
                                                &mut state.test_pulse_params.period_ns,
                                            )
                                            .range(1000..=1_000_000_000u32)
                                            .suffix(" ns"),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                    ui.label(format!(
                                        "({:.1} Hz)",
                                        state.test_pulse_params.frequency_hz()
                                    ));
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Width:");
                                    if ui
                                        .add(
                                            egui::DragValue::new(
                                                &mut state.test_pulse_params.width_ns,
                                            )
                                            .range(8..=1_000_000_000u32)
                                            .suffix(" ns"),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Low Level:");
                                    if ui
                                        .add(
                                            egui::DragValue::new(
                                                &mut state.test_pulse_params.low_level,
                                            )
                                            .range(0..=65535u32)
                                            .suffix(" ADC"),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });

                                ui.horizontal(|ui| {
                                    ui.label("High Level:");
                                    if ui
                                        .add(
                                            egui::DragValue::new(
                                                &mut state.test_pulse_params.high_level,
                                            )
                                            .range(0..=65535u32)
                                            .suffix(" ADC"),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });

                                if changed {
                                    state.test_pulse_params_dirty = true;
                                }

                                if state.running {
                                    ui.label(
                                        egui::RichText::new("Changes apply immediately (SetInRun)")
                                            .small()
                                            .weak(),
                                    );
                                }
                            });
                    }

                    ui.separator();
                    ui.heading("ROOT Output");

                    ui.horizontal(|ui| {
                        ui.label("File:");
                        ui.text_edit_singleline(&mut self.output_path);
                    });

                    ui.horizontal(|ui| {
                        ui.checkbox(&mut state.recording, "Record");
                        let mem_mb =
                            state.event_buffer.estimated_memory_bytes() as f64 / (1024.0 * 1024.0);
                        ui.label(format!(
                            "({} events, {:.1} MB)",
                            state.recorded_count, mem_mb
                        ));
                    });

                    // Auto-save: when DAQ stops with data, or when user unchecks Record
                    let is_recording = state.recording && state.running;
                    if self.was_recording && !is_recording && state.recorded_count > 0 {
                        match state.event_buffer.write_root(&self.output_path) {
                            Ok(n) => {
                                state.status_message =
                                    format!("Saved {} events to {}", n, self.output_path);
                                eprintln!("Auto-saved {} events to {}", n, self.output_path);
                                state.event_buffer.clear();
                                state.recorded_count = 0;
                            }
                            Err(e) => {
                                state.status_message = format!("Save failed: {}", e);
                                eprintln!("Auto-save failed: {}", e);
                            }
                        }
                    }
                    self.was_recording = is_recording;

                    // ---- Parameters (collapsible) ----
                    egui::CollapsingHeader::new("Parameters")
                        .default_open(true)
                        .show(ui, |ui| {
                            let mut current_section = String::new();
                            for reg in &self.register_defs {
                                if reg.section != current_section {
                                    ui.separator();
                                    ui.strong(&reg.section);
                                    current_section = reg.section.clone();
                                }
                                let value = state
                                    .param_values
                                    .entry(reg.name.clone())
                                    .or_insert(reg.default);
                                ui.horizontal(|ui| {
                                    ui.label(format!("{}:", reg.name));
                                    if reg.readonly {
                                        ui.label(format!("{}", *value));
                                    } else {
                                        ui.add(egui::DragValue::new(value).range(reg.min..=reg.max));
                                    }
                                });
                            }

                            ui.add_space(10.0);
                            if thread_active {
                                if ui.button("Restart to Apply").clicked() {
                                    stop_clicked = true;
                                    start_clicked = true;
                                }
                                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    stop_clicked = true;
                                    start_clicked = true;
                                }
                                ui.label("(or press Enter)");
                            } else {
                                ui.label("Press Start to begin");
                            }
                        });

                    // ---- 2D Histogram Settings (collapsible) ----
                    let mut axes_changed = false;
                    egui::CollapsingHeader::new("2D Histogram")
                        .default_open(true)
                        .show(ui, |ui| {
                            // Axis source selection
                            ui.horizontal(|ui| {
                                ui.label("X axis:");
                                egui::ComboBox::from_id_salt("x_source")
                                    .selected_text(state.x_source.label())
                                    .show_ui(ui, |ui| {
                                        for src in PlotSource::ALL {
                                            if ui.selectable_value(&mut state.x_source, src, src.label()).changed() {
                                                axes_changed = true;
                                            }
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                ui.label("Y axis:");
                                egui::ComboBox::from_id_salt("y_source")
                                    .selected_text(state.y_source.label())
                                    .show_ui(ui, |ui| {
                                        for src in PlotSource::ALL {
                                            if ui.selectable_value(&mut state.y_source, src, src.label()).changed() {
                                                axes_changed = true;
                                            }
                                        }
                                    });
                            });

                            ui.separator();
                            let sel = self.selected_channel as usize;

                            // Range
                            ui.horizontal(|ui| {
                                ui.label("X Max:");
                                let mut max = state.channels[sel].histogram.x_max as u32;
                                if ui
                                    .add(egui::DragValue::new(&mut max).range(1000..=65536))
                                    .changed()
                                {
                                    let max_f = max as f64;
                                    for ch_data in &mut state.channels {
                                        ch_data.histogram.x_max = max_f;
                                        ch_data.histogram_dirty = true;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Y Max:");
                                let mut max = state.channels[sel].histogram.y_max as u32;
                                if ui
                                    .add(egui::DragValue::new(&mut max).range(1000..=65536))
                                    .changed()
                                {
                                    let max_f = max as f64;
                                    for ch_data in &mut state.channels {
                                        ch_data.histogram.y_max = max_f;
                                        ch_data.histogram_dirty = true;
                                    }
                                }
                            });

                            ui.separator();

                            // Bins
                            let current_x_bins = state.channels[sel].histogram.x_bins;
                            let current_y_bins = state.channels[sel].histogram.y_bins;
                            ui.horizontal(|ui| {
                                ui.label("X Bins:");
                                let mut bins = current_x_bins as u32;
                                if ui
                                    .add(egui::DragValue::new(&mut bins).range(16..=4096).speed(16.0))
                                    .changed()
                                {
                                    for ch_data in &mut state.channels {
                                        ch_data.histogram.resize(bins as usize, current_y_bins);
                                        ch_data.histogram_dirty = true;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Y Bins:");
                                let mut bins = current_y_bins as u32;
                                if ui
                                    .add(egui::DragValue::new(&mut bins).range(16..=4096).speed(16.0))
                                    .changed()
                                {
                                    for ch_data in &mut state.channels {
                                        ch_data.histogram.resize(current_x_bins, bins as usize);
                                        ch_data.histogram_dirty = true;
                                    }
                                }
                            });

                            let x_width = (state.channels[sel].histogram.x_max
                                - state.channels[sel].histogram.x_min)
                                / current_x_bins as f64;
                            let y_width = (state.channels[sel].histogram.y_max
                                - state.channels[sel].histogram.y_min)
                                / current_y_bins as f64;
                            ui.label(format!("X bin width: {:.1}", x_width));
                            ui.label(format!("Y bin width: {:.1}", y_width));
                        });

                    // Apply axis source change (after collapsible scope)
                    if axes_changed {
                        let x_max = state.x_source.default_max();
                        let y_max = state.y_source.default_max();
                        for ch_data in &mut state.channels {
                            ch_data.histogram.x_max = x_max;
                            ch_data.histogram.y_max = y_max;
                            ch_data.histogram.clear();
                            ch_data.histogram_dirty = true;
                        }
                    }
                }); // ScrollArea
            });

        if stop_clicked {
            self.stop_acquisition();
        }
        if start_clicked {
            self.start_acquisition();
        }

        egui::TopBottomPanel::bottom("waveform_panel")
            .resizable(true)
            .default_height(200.0)
            .show(ctx, |ui| {
                let sel = self.selected_channel as usize;
                let (points_opt, energy) = {
                    let state = self.shared.lock().unwrap();
                    let ch_data = &state.channels[sel];
                    let energy = ch_data.latest_waveform_energy;
                    if ch_data.waveform_len > 0 {
                        let points: PlotPoints = ch_data.waveform_buffer[..ch_data.waveform_len]
                            .iter()
                            .enumerate()
                            .map(|(i, &v)| [i as f64, v as f64])
                            .collect();
                        (Some(points), energy)
                    } else {
                        (None, energy)
                    }
                };

                ui.horizontal(|ui| {
                    ui.heading("Waveform");
                    if points_opt.is_some() {
                        ui.label(format!("(Energy: {})", energy));
                    }
                });

                if let Some(points) = points_opt {
                    let line = Line::new(points).name("Waveform");
                    Plot::new("waveform_plot")
                        .height(ui.available_height())
                        .x_axis_label("Sample")
                        .y_axis_label("ADC")
                        .show(ui, |plot_ui| {
                            plot_ui.line(line);
                        });
                } else {
                    ui.label("No waveform data");
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let sel = self.selected_channel as usize;

            let channel_changed = self.selected_channel != self.prev_selected_channel;
            self.prev_selected_channel = self.selected_channel;

            // Check if texture needs regeneration (short lock, no clone yet)
            let (needs_update, x_max, y_max, x_label, y_label) = {
                let mut state = self.shared.lock().unwrap();
                let ch_data = &mut state.channels[sel];
                let dirty = ch_data.histogram_dirty;
                let needs = dirty || channel_changed || self.texture.is_none();
                if dirty {
                    ch_data.histogram_dirty = false;
                }
                (
                    needs,
                    ch_data.histogram.x_max,
                    ch_data.histogram.y_max,
                    state.x_source.label(),
                    state.y_source.label(),
                )
            };

            ui.heading(format!("{} vs {} (Ch {})", y_label, x_label, self.selected_channel));

            // Clone and generate texture only when needed
            if needs_update {
                let hist_clone = {
                    let state = self.shared.lock().unwrap();
                    state.channels[sel].histogram.clone()
                };
                let image = hist_clone.to_texture();
                self.texture =
                    Some(ctx.load_texture("histogram", image, egui::TextureOptions::NEAREST));
            }

            if let Some(texture) = &self.texture {
                Plot::new("histogram_plot")
                    .data_aspect(1.0)
                    .x_axis_label(x_label)
                    .y_axis_label(y_label)
                    .show(ui, |plot_ui| {
                        let image = PlotImage::new(
                            texture,
                            PlotPoint::new(x_max / 2.0, y_max / 2.0),
                            [x_max as f32, y_max as f32],
                        );
                        plot_ui.image(image);
                    });
            }
        });
    }

    fn on_exit(&mut self) {
        {
            let state = self.shared.lock().unwrap();
            // Only save param values that differ from register_defs defaults.
            // This ensures register_defs.json default updates take effect on next launch.
            let mut changed_params = HashMap::new();
            for reg in &self.register_defs {
                if let Some(&val) = state.param_values.get(&reg.name) {
                    if val != reg.default && !reg.readonly {
                        changed_params.insert(reg.name.clone(), val);
                    }
                }
            }
            let settings = AppSettings {
                url: self.url.clone(),
                output_path: self.output_path.clone(),
                param_values: changed_params,
                selected_channel: self.selected_channel,
                register_defs_hash: Some(self.register_defs_hash.clone()),
            };
            settings.save();
        }
        self.stop_acquisition();
    }
}

/// Acquisition thread
fn acquisition_thread(
    url: String,
    shared: Arc<Mutex<SharedState>>,
    shutdown: Arc<AtomicBool>,
    register_defs: Vec<RegisterDef>,
    test_pulse: bool,
    force_write: bool,
) {
    eprintln!("[ACQ] Connecting to {}...", url);
    let handle = match CaenHandle::open(&url) {
        Ok(h) => {
            eprintln!("[ACQ] Connected OK");
            let mut state = shared.lock().unwrap();
            state.connected = true;
            state.status_message = if test_pulse {
                "Connected (Test Pulse)".to_string()
            } else {
                "Connected".to_string()
            };
            h
        }
        Err(e) => {
            eprintln!("[ACQ] Connection failed: {}", e);
            let mut state = shared.lock().unwrap();
            state.status_message = format!("Connection failed: {}", e);
            state.running = false;
            return;
        }
    };

    // Enable all channels
    for ch in 0..32 {
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), "True");
    }

    // NOTE: Waveform record length is controlled by AMax FW-specific registers (not ChRecordLengthT)

    // Save original trigger sources and configure test pulse if needed
    let original_gts = handle.get_value("/par/GlobalTriggerSource").ok();
    let original_ats = handle.get_value("/par/AcqTriggerSource").ok();

    // Track whether test pulse is currently active on hardware
    let mut tp_hw_active = false;

    if test_pulse {
        eprintln!("[ACQ] Configuring test pulse...");
        let params = {
            let state = shared.lock().unwrap();
            state.test_pulse_params.clone()
        };
        let errors = apply_test_pulse(&handle, &params);
        tp_hw_active = true;

        let mut state = shared.lock().unwrap();
        if errors.is_empty() {
            let msg = format!(
                "Test Pulse configured ({:.0} Hz, ADC {}-{})",
                params.frequency_hz(),
                params.low_level,
                params.high_level
            );
            eprintln!("[ACQ] {}", msg);
            state.status_message = msg;
        } else {
            let msg = format!("Test Pulse errors: {}", errors.join("; "));
            eprintln!("[ACQ] {}", msg);
            state.status_message = msg;
        }
    }

    // Snapshot user's desired values BEFORE read_hw_params overwrites them
    let desired_values = {
        let state = shared.lock().unwrap();
        state.param_values.clone()
    };

    // Read current HW register values (populates UI with actual hardware state)
    let hw_values = if !register_defs.is_empty() {
        eprintln!("[ACQ] Reading current HW register values...");
        read_hw_params(&handle, &register_defs, &shared)
    } else {
        HashMap::new()
    };

    // Apply register parameters (diff-write: only changed values, skip readonly)
    // Use desired_values (pre-overwrite snapshot), not state.param_values (overwritten by read_hw_params)
    // When force_write is true (FW change or --reset-params), pass empty hw_values
    // to force all registers to be written regardless of current hardware state.
    if register_defs.is_empty() {
        eprintln!("[ACQ] No register parameters to apply (skipped)");
    } else {
        let effective_hw = if force_write {
            eprintln!("[ACQ] Force-writing ALL registers (firmware change or --reset-params)");
            HashMap::new()
        } else {
            hw_values
        };
        let (written, unchanged, readonly_skipped, errors, first_error) =
            apply_params(&handle, &register_defs, &desired_values, &effective_hw);

        // Restore UI to show user's desired values (read_hw_params overwrote them with HW values)
        let mut state = shared.lock().unwrap();
        for (name, val) in &desired_values {
            state.param_values.insert(name.clone(), *val);
        }
        if errors > 0 {
            let msg = format!(
                "Init: wrote {}, unchanged {}, readonly {}, err {}: {}",
                written,
                unchanged,
                readonly_skipped,
                errors,
                first_error.unwrap_or_default()
            );
            eprintln!("[ACQ] {}", msg);
            state.status_message = msg;
        } else {
            let tp_label = if test_pulse { " [TestPulse]" } else { "" };
            let msg = format!(
                "Init: wrote {}, unchanged {}, readonly {}{}",
                written, unchanged, readonly_skipped, tp_label
            );
            eprintln!("[ACQ] {}", msg);
            state.status_message = msg;
        }
    }

    // Configure OpenDPP endpoint with waveform
    eprintln!("[ACQ] Configuring OpenDPP endpoint...");
    let endpoint = match handle.configure_opendpp_endpoint(true) {
        Ok(ep) => {
            eprintln!("[ACQ] Endpoint configured OK");
            ep
        }
        Err(e) => {
            eprintln!("[ACQ] Endpoint error: {}", e);
            let mut state = shared.lock().unwrap();
            state.status_message = format!("Endpoint error: {}", e);
            state.running = false;
            restore_trigger_sources(&handle, &original_gts, &original_ats, test_pulse);
            return;
        }
    };

    // Start acquisition
    eprintln!("[ACQ] Starting acquisition (cleardata → arm → start)...");
    if let Err(e) = handle.send_command("/cmd/cleardata") {
        eprintln!("[ACQ] cleardata failed: {}", e);
    }
    if let Err(e) = handle.send_command("/cmd/armacquisition") {
        eprintln!("[ACQ] armacquisition failed: {}", e);
    }
    if let Err(e) = handle.send_command("/cmd/swstartacquisition") {
        eprintln!("[ACQ] swstartacquisition failed: {}", e);
    }

    {
        let mut state = shared.lock().unwrap();
        state.running = true;
        state.status_message = "Running".to_string();
        eprintln!("[ACQ] Running");
    }

    let mut user_info_buffer = [0u64; 1024]; // FW caenlist max len = 1024
    let mut waveform_buffer = [0u16; 8192];
    let mut last_rate_update = Instant::now();
    let mut events_since_last_update = 0u64;
    let mut consecutive_nones = 0u32;

    while !shutdown.load(Ordering::Relaxed) {
        match endpoint.read_opendpp_event_with_waveform(
            100,
            &mut user_info_buffer,
            &mut waveform_buffer,
        ) {
            Ok(Some(event)) => {
                events_since_last_update += 1;
                consecutive_nones = 0;

                let ch_idx = event.channel as usize;

                {
                    let mut state = shared.lock().unwrap();

                    // Fill per-channel histogram (always, regardless of UI selection)
                    if ch_idx < state.channels.len() {
                        let x_val = state.x_source.extract(event.energy, &event.user_info);
                        let y_val = state.y_source.extract(event.energy, &event.user_info);
                        let ch_data = &mut state.channels[ch_idx];
                        ch_data.histogram.fill(x_val, y_val);
                        ch_data.histogram_dirty = true;
                        ch_data.events_since_last_tick += 1;

                        // Per-channel waveform update (rate-limited per channel)
                        if ch_data.last_waveform_update.elapsed() >= Duration::from_millis(100) {
                            if let Some(ref wf) = event.waveform {
                                let len = wf.len().min(ch_data.waveform_buffer.len());
                                ch_data.waveform_buffer[..len].copy_from_slice(&wf[..len]);
                                ch_data.waveform_len = len;
                                ch_data.latest_waveform_energy = event.energy;
                            }
                            ch_data.last_waveform_update = Instant::now();
                        }
                    }

                    // Recording: always all channels
                    if state.recording {
                        if state.event_buffer.estimated_memory_bytes() >= MAX_BUFFER_BYTES {
                            state.recording = false;
                            state.status_message =
                                "Recording stopped: 10 GB memory limit reached".to_string();
                            eprintln!("Recording auto-stopped: buffer reached 10 GB limit");
                        } else {
                            state.event_buffer.push(
                                event.channel,
                                event.energy,
                                event.timestamp,
                                event.fine_timestamp,
                                event.flags_a,
                                event.flags_b,
                                event.psd,
                                &event.user_info,
                                event.waveform.as_deref(),
                            );
                            state.recorded_count = state.event_buffer.len();
                        }
                    }
                }
            }
            Ok(None) => {
                consecutive_nones += 1;
                // After ~3s of no data (30 × 100ms timeout), check acquisition status
                if consecutive_nones == 30 {
                    eprintln!("[ACQ] No data for ~3s, checking acquisition status...");
                    // FELib parameters
                    for param in [
                        "/par/AcquisitionStatus",
                        "/par/EnEventCountDown",
                        "/par/EventCountDown",
                        "/par/EnAutoDisarmAcq",
                        "/par/NumEventsPerAggregate",
                        "/par/VolatileClockOutDelay",
                        "/par/AcqTriggerSource",
                        "/par/TestPulsePeriod",
                        "/par/TestPulseWidth",
                    ] {
                        if let Ok(v) = handle.get_value(param) {
                            eprintln!("[ACQ]   {} = {}", param, v);
                        }
                    }
                    // Channel status
                    for ch in 0..2 {
                        for param in ["chenable", "SelfTriggerRate"] {
                            if let Ok(v) = handle.get_value(&format!("/ch/{}/par/{}", ch, param)) {
                                eprintln!("[ACQ]   ch{}/{} = {}", ch, param, v);
                            }
                        }
                    }
                    // Read back RUN_CFG registers
                    for (name, addr) in [("ch0_RUN_CFG", 15u32), ("ch1_RUN_CFG", 262159u32)] {
                        if let Ok(v) = handle.get_user_register(addr * 4) {
                            eprintln!("[ACQ]   {} = {} (0x{:X})", name, v, v);
                        }
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(e) => {
                eprintln!(
                    "[ACQ] Read error: code={}, {}: {}",
                    e.code, e.name, e.description
                );
                let mut state = shared.lock().unwrap();
                state.status_message = format!("Read error: {} (code {})", e.name, e.code);
                drop(state);
                if e.code == -12 {
                    eprintln!("[ACQ] STOP signal received, exiting read loop");
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        let elapsed = last_rate_update.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let secs = elapsed.as_secs_f64();
            let rate = events_since_last_update as f64 / secs;
            {
                let mut state = shared.lock().unwrap();
                state.event_rate = rate;
                // Per-channel rate calculation
                for ch_data in &mut state.channels {
                    ch_data.event_rate = ch_data.events_since_last_tick as f64 / secs;
                    ch_data.events_since_last_tick = 0;
                }
            }
            events_since_last_update = 0;
            last_rate_update = Instant::now();
        }

        // Handle runtime test pulse toggle
        {
            let mut state = shared.lock().unwrap();
            if state.test_pulse_toggle_requested {
                state.test_pulse_toggle_requested = false;
                let want_active = state.test_pulse_active;
                let params = state.test_pulse_params.clone();
                drop(state);

                // Stop → reconfigure → restart
                eprintln!(
                    "[ACQ] Test pulse toggle: want_active={}, tp_hw_active={}",
                    want_active, tp_hw_active
                );
                let _ = handle.send_command("/cmd/disarmacquisition");

                if want_active && !tp_hw_active {
                    eprintln!("[ACQ] Enabling test pulse at runtime...");
                    let errors = apply_test_pulse(&handle, &params);
                    tp_hw_active = true;
                    let mut state = shared.lock().unwrap();
                    if errors.is_empty() {
                        let msg = format!("Test Pulse ON ({:.0} Hz)", params.frequency_hz());
                        eprintln!("[ACQ] {}", msg);
                        state.status_message = msg;
                    } else {
                        let msg = format!("Test Pulse errors: {}", errors.join("; "));
                        eprintln!("[ACQ] {}", msg);
                        state.status_message = msg;
                    }
                } else if !want_active && tp_hw_active {
                    eprintln!("[ACQ] Disabling test pulse at runtime...");
                    restore_trigger_sources(&handle, &original_gts, &original_ats, true);
                    tp_hw_active = false;
                    let mut state = shared.lock().unwrap();
                    state.status_message = "Test Pulse OFF — triggers restored".to_string();
                    eprintln!("[ACQ] Test Pulse OFF — triggers restored");
                }

                let _ = handle.send_command("/cmd/cleardata");
                let _ = handle.send_command("/cmd/armacquisition");
                let _ = handle.send_command("/cmd/swstartacquisition");
            } else if state.test_pulse_params_dirty && tp_hw_active {
                // SetInRun: apply params without restart
                state.test_pulse_params_dirty = false;
                let params = state.test_pulse_params.clone();
                drop(state);

                let _ = handle.set_value("/par/TestPulsePeriod", &params.period_ns.to_string());
                let _ = handle.set_value("/par/TestPulseWidth", &params.width_ns.to_string());
                let _ = handle.set_value("/par/TestPulseLowLevel", &params.low_level.to_string());
                let _ = handle.set_value("/par/TestPulseHighLevel", &params.high_level.to_string());
            }
        }
    }

    eprintln!("[ACQ] Stopping acquisition...");
    let _ = handle.send_command("/cmd/disarmacquisition");

    // Restore original trigger sources if test pulse was active on hardware
    if tp_hw_active {
        eprintln!("[ACQ] Restoring original trigger sources...");
    }
    restore_trigger_sources(&handle, &original_gts, &original_ats, tp_hw_active);

    {
        let mut state = shared.lock().unwrap();
        state.running = false;
        state.connected = false;
        state.status_message = "Stopped".to_string();
    }
    eprintln!("[ACQ] Stopped");
}

/// Apply test pulse parameters and set trigger source to TestPulse.
/// Returns a list of error messages (empty = success).
fn apply_test_pulse(handle: &CaenHandle, params: &TestPulseParams) -> Vec<String> {
    let mut errors = Vec::new();

    let tp_settings = [
        ("/par/TestPulsePeriod", params.period_ns.to_string()),
        ("/par/TestPulseWidth", params.width_ns.to_string()),
        ("/par/TestPulseLowLevel", params.low_level.to_string()),
        ("/par/TestPulseHighLevel", params.high_level.to_string()),
    ];
    for (path, value) in &tp_settings {
        if let Err(e) = handle.set_value(path, value) {
            eprintln!(
                "[ACQ] Test pulse set_value {} = {} failed: {}",
                path, value, e
            );
            errors.push(format!("{}: {}", path, e));
        }
    }

    // GlobalTriggerSource — AMax (OpenDPP) FW may not support this parameter at all.
    // Self-trigger goes through FW internal OR gate, so GlobalTriggerSource is optional.
    let gts_candidates = ["TestPulse", "TstTrg", "SwTrg", "TestPulse | SwTrg"];
    let mut gts_set = false;
    for candidate in &gts_candidates {
        if handle
            .set_value("/par/GlobalTriggerSource", candidate)
            .is_ok()
        {
            eprintln!("[ACQ] GlobalTriggerSource = {} (accepted)", candidate);
            gts_set = true;
            break;
        }
    }
    if !gts_set {
        eprintln!("[ACQ] GlobalTriggerSource: no candidate accepted (OK for OpenDPP/AMax FW)");
    }

    // AcqTriggerSource — this is the essential one for test pulse triggering
    let ats_candidates = ["TestPulse", "GlobalTriggerSource", "TstTrg", "SwTrg"];
    let mut ats_set = false;
    for candidate in &ats_candidates {
        if handle.set_value("/par/AcqTriggerSource", candidate).is_ok() {
            eprintln!("[ACQ] AcqTriggerSource = {} (accepted)", candidate);
            ats_set = true;
            break;
        }
    }
    if !ats_set {
        let msg = "AcqTriggerSource: no candidate value accepted".to_string();
        eprintln!("[ACQ] {}", msg);
        errors.push(msg);
    }

    errors
}

/// Restore original trigger source settings and disable test pulse
fn restore_trigger_sources(
    handle: &CaenHandle,
    original_gts: &Option<String>,
    original_ats: &Option<String>,
    was_active: bool,
) {
    if !was_active {
        return;
    }
    if let Some(gts) = original_gts {
        let _ = handle.set_value("/par/GlobalTriggerSource", gts);
    }
    if let Some(ats) = original_ats {
        let _ = handle.set_value("/par/AcqTriggerSource", ats);
    }
    // Disable test pulse by setting period to 0
    let _ = handle.set_value("/par/TestPulsePeriod", "0");
}

/// Read current register values from hardware.
/// Updates shared state param_values so the UI shows actual HW values.
/// Returns the HW values map for use in diff-write.
fn read_hw_params(
    handle: &CaenHandle,
    defs: &[RegisterDef],
    shared: &Arc<Mutex<SharedState>>,
) -> HashMap<String, u32> {
    let mut read_ok = 0;
    let mut read_err = 0;
    let mut hw_values: HashMap<String, u32> = HashMap::new();

    for reg in defs {
        let byte_addr = reg.address * 4;
        match handle.get_user_register(byte_addr) {
            Ok(value) => {
                hw_values.insert(reg.name.clone(), value);
                read_ok += 1;
            }
            Err(e) => {
                eprintln!("[ACQ] Read {} (0x{:X}) failed: {}", reg.name, byte_addr, e);
                read_err += 1;
            }
        }
    }

    // Update shared state with HW values (UI will show actual hardware state)
    {
        let mut state = shared.lock().unwrap();
        for (name, hw_val) in &hw_values {
            state.param_values.insert(name.clone(), *hw_val);
        }
    }

    eprintln!(
        "[ACQ] Read {} HW registers OK, {} failed",
        read_ok, read_err
    );

    hw_values
}

/// Apply register parameters to digitizer with diff-write.
/// Only writes registers where the desired value differs from the HW value.
/// Skips readonly registers entirely.
/// Returns (written, skipped_unchanged, skipped_readonly, errors, first_error_message)
fn apply_params(
    handle: &CaenHandle,
    defs: &[RegisterDef],
    values: &HashMap<String, u32>,
    hw_values: &HashMap<String, u32>,
) -> (usize, usize, usize, usize, Option<String>) {
    let mut written = 0;
    let mut unchanged = 0;
    let mut readonly_skipped = 0;
    let mut errors = 0;
    let mut first_error: Option<String> = None;

    for reg in defs {
        if reg.readonly {
            readonly_skipped += 1;
            continue;
        }

        let desired = values.get(&reg.name).copied().unwrap_or(reg.default);
        let hw_val = hw_values.get(&reg.name).copied();

        // Skip if HW already has the desired value
        if hw_val == Some(desired) {
            unchanged += 1;
            continue;
        }

        let byte_addr = reg.address * 4;
        match handle.set_user_register(byte_addr, desired) {
            Ok(()) => match handle.get_user_register(byte_addr) {
                Ok(readback) => {
                    if readback == desired {
                        written += 1;
                    } else {
                        errors += 1;
                        if first_error.is_none() {
                            first_error = Some(format!(
                                "{} (0x{:X}): wrote {}, read {}",
                                reg.name, byte_addr, desired, readback
                            ));
                        }
                    }
                }
                Err(_) => {
                    written += 1; // write succeeded, readback failed (acceptable)
                }
            },
            Err(e) => {
                errors += 1;
                if first_error.is_none() {
                    first_error = Some(format!("{} (0x{:X}): {}", reg.name, byte_addr, e));
                }
            }
        }
    }

    (written, unchanged, readonly_skipped, errors, first_error)
}

fn main() -> eframe::Result<()> {
    let args = Args::parse();
    let test_pulse = args.test_pulse;
    let reset_params = args.reset_params;
    let register_defs_path = args.register_defs;

    let title = if test_pulse {
        "AMax Viewer - Firmware Development Tool [TEST PULSE]"
    } else {
        "AMax Viewer - Firmware Development Tool"
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title(title),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                eframe::egui_wgpu::WgpuSetupCreateNew {
                    instance_descriptor: eframe::wgpu::InstanceDescriptor {
                        backends: eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::METAL,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ),
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "AMax Viewer",
        options,
        Box::new(move |cc| {
            Ok(Box::new(AmaxViewerApp::new(
                cc,
                test_pulse,
                register_defs_path,
                reset_params,
            )))
        }),
    )
}

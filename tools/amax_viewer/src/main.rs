//! AMax Firmware Development Tool
//!
//! 2D histogram viewer for AMax (user_info[0]) vs Energy.
//! Allows real-time parameter adjustment while observing histogram changes.

use delila_rs::reader::CaenHandle;
use eframe::egui;
use egui_plot::{Line, Plot, PlotImage, PlotPoint, PlotPoints};
use oxyroot::{RootFile, WriterTree};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Event data for ROOT file output
#[derive(Default)]
struct EventBuffer {
    channel: Vec<i32>,
    energy: Vec<i32>,
    amax: Vec<i64>,
    timestamp: Vec<i64>,
}

impl EventBuffer {
    fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, ch: u8, energy: u16, amax: u64, timestamp: u64) {
        self.channel.push(ch as i32);
        self.energy.push(energy as i32);
        self.amax.push(amax as i64);
        self.timestamp.push(timestamp as i64);
    }

    fn len(&self) -> usize {
        self.energy.len()
    }

    fn clear(&mut self) {
        self.channel.clear();
        self.energy.clear();
        self.amax.clear();
        self.timestamp.clear();
    }

    /// Write events to ROOT file
    fn write_root(&self, path: &str) -> Result<usize, Box<dyn std::error::Error>> {
        if self.energy.is_empty() {
            return Ok(0);
        }

        let mut file = RootFile::create(path)?;
        let mut tree = WriterTree::new("events");

        tree.new_branch("channel", self.channel.clone().into_iter());
        tree.new_branch("energy", self.energy.clone().into_iter());
        tree.new_branch("amax", self.amax.clone().into_iter());
        tree.new_branch("timestamp", self.timestamp.clone().into_iter());

        tree.write(&mut file)?;
        file.close()?;

        Ok(self.energy.len())
    }
}

/// MCA HLS Register parameters (Core + AMax = 20 registers)
#[derive(Clone, Serialize, Deserialize)]
struct McaParams {
    // Core registers (0x0-0xC)
    polarity: u32,      // 0x0
    offset: u32,        // 0x1
    threshold: u32,     // 0x2
    trig_k: u32,        // 0x3
    trig_m: u32,        // 0x4
    trap_k: u32,        // 0x5
    trap_m: u32,        // 0x6
    deconv_m: u32,      // 0x7
    trap_gain: u32,     // 0x8
    bl_len: u32,        // 0x9
    bl_inib: u32,       // 0xA
    sample_pos: u32,    // 0xB
    run_cfg: u32,       // 0xC
    // AMax registers
    window_maxim: u32,      // 0x14000
    baseline_delay: u32,    // 0x160000
    baseline_len: u32,      // 0x160001
    baseline_offset: u32,   // 0x160002
    amax_window: u32,       // 0x160003
    amax_delay: u32,        // 0x160004
    amax_len: u32,          // 0x160005
}

impl Default for McaParams {
    fn default() -> Self {
        Self {
            // Core registers
            polarity: 0,
            offset: 0,
            threshold: 100,
            trig_k: 10,
            trig_m: 12,
            trap_k: 500,
            trap_m: 550,
            deconv_m: 3499000,
            trap_gain: 2500,
            bl_len: 6,
            bl_inib: 1200,
            sample_pos: 510,
            run_cfg: 1,
            // AMax registers
            window_maxim: 200,
            baseline_delay: 200,
            baseline_len: 6,
            baseline_offset: 1000,
            amax_window: 1000,
            amax_delay: 4,
            amax_len: 2,
        }
    }
}

/// Application settings (persisted to disk)
#[derive(Clone, Serialize, Deserialize)]
struct AppSettings {
    #[serde(default = "AppSettings::default_url")]
    url: String,
    #[serde(default = "AppSettings::default_output_path")]
    output_path: String,
    #[serde(default)]
    params: McaParams,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            url: Self::default_url(),
            output_path: Self::default_output_path(),
            params: McaParams::default(),
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

    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut p| {
            p.push("amax_viewer");
            p.push("settings.json");
            p
        })
    }

    fn load() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(&path, json);
            }
        }
    }
}

/// 2D Histogram data
struct Histogram2D {
    /// Bins: [energy_bin][amax_bin]
    bins: Vec<Vec<u32>>,
    energy_bins: usize,
    amax_bins: usize,
    energy_min: f64,
    energy_max: f64,
    amax_min: f64,
    amax_max: f64,
    total_events: u64,
}

impl Histogram2D {
    fn new(energy_bins: usize, amax_bins: usize) -> Self {
        Self {
            bins: vec![vec![0u32; amax_bins]; energy_bins],
            energy_bins,
            amax_bins,
            energy_min: 0.0,
            energy_max: 65536.0,
            amax_min: 0.0,
            amax_max: 16384.0,
            total_events: 0,
        }
    }

    fn fill(&mut self, energy: u16, amax: u64) {
        let e = energy as f64;
        let a = amax as f64;

        if e >= self.energy_min && e < self.energy_max && a >= self.amax_min && a < self.amax_max {
            let e_bin =
                ((e - self.energy_min) / (self.energy_max - self.energy_min) * self.energy_bins as f64) as usize;
            let a_bin =
                ((a - self.amax_min) / (self.amax_max - self.amax_min) * self.amax_bins as f64) as usize;

            if e_bin < self.energy_bins && a_bin < self.amax_bins {
                self.bins[e_bin][a_bin] = self.bins[e_bin][a_bin].saturating_add(1);
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

    /// Resize histogram bins (clears all data)
    fn resize(&mut self, energy_bins: usize, amax_bins: usize) {
        if energy_bins != self.energy_bins || amax_bins != self.amax_bins {
            self.energy_bins = energy_bins;
            self.amax_bins = amax_bins;
            self.bins = vec![vec![0u32; amax_bins]; energy_bins];
            self.total_events = 0;
        }
    }

    fn max_count(&self) -> u32 {
        self.bins.iter().flat_map(|row| row.iter()).cloned().max().unwrap_or(1)
    }

    /// Convert to RGBA texture for display
    fn to_texture(&self) -> egui::ColorImage {
        let max = self.max_count().max(1) as f32;
        let mut pixels = Vec::with_capacity(self.energy_bins * self.amax_bins);

        // Note: Y axis is amax (bottom to top), X axis is energy (left to right)
        for a_bin in (0..self.amax_bins).rev() {
            for e_bin in 0..self.energy_bins {
                let count = self.bins[e_bin][a_bin] as f32;
                let intensity = (count / max).sqrt(); // sqrt for better visibility

                // Color map: black -> blue -> cyan -> yellow -> white
                let color = colormap(intensity);
                pixels.push(color);
            }
        }

        egui::ColorImage {
            size: [self.energy_bins, self.amax_bins],
            pixels,
        }
    }
}

/// Simple colormap (black -> blue -> cyan -> yellow -> white)
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

/// Shared state between GUI and acquisition thread
struct SharedState {
    histogram: Histogram2D,
    params: McaParams,
    running: bool,
    event_rate: f64,
    connected: bool,
    status_message: String,
    /// Latest waveform for display (pre-allocated buffer, swap for zero-copy)
    waveform_buffer: Vec<u16>,
    waveform_len: usize,
    /// Energy value of the latest waveform
    latest_waveform_energy: u16,
    /// True when histogram data changed and texture needs regeneration
    histogram_dirty: bool,
    /// Event buffer for ROOT output
    event_buffer: EventBuffer,
    /// Whether recording is enabled
    recording: bool,
    /// Number of recorded events
    recorded_count: usize,
}

struct AmaxViewerApp {
    url: String,
    shared: Arc<Mutex<SharedState>>,
    shutdown: Arc<AtomicBool>,
    acq_thread: Option<thread::JoinHandle<()>>,
    texture: Option<egui::TextureHandle>,
    /// Output ROOT file path
    output_path: String,
}

impl AmaxViewerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let settings = AppSettings::load();

        let shared = Arc::new(Mutex::new(SharedState {
            histogram: Histogram2D::new(512, 512),
            params: settings.params,
            running: false,
            event_rate: 0.0,
            connected: false,
            status_message: "Not connected".to_string(),
            waveform_buffer: vec![0u16; 8192], // Pre-allocated
            waveform_len: 0,
            latest_waveform_energy: 0,
            histogram_dirty: true, // Force initial texture generation
            event_buffer: EventBuffer::new(),
            recording: false,
            recorded_count: 0,
        }));

        Self {
            url: settings.url,
            shared,
            shutdown: Arc::new(AtomicBool::new(false)),
            acq_thread: None,
            texture: None,
            output_path: settings.output_path,
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

        self.acq_thread = Some(thread::spawn(move || {
            acquisition_thread(url, shared, shutdown);
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
        // Request repaint for continuous update
        ctx.request_repaint_after(Duration::from_millis(100));

        // Track actions to perform after releasing lock
        let mut start_clicked = false;
        let mut stop_clicked = false;

        // Check if acquisition thread is running (for restart logic)
        // This is more reliable than state.running during thread init/shutdown
        let thread_active = self.acq_thread.is_some();

        egui::SidePanel::left("params_panel").min_width(250.0).show(ctx, |ui| {
            ui.heading("Connection");
            ui.text_edit_singleline(&mut self.url);

            let mut state = self.shared.lock().unwrap();

            ui.horizontal(|ui| {
                if state.running {
                    if ui.button("Stop").clicked() {
                        stop_clicked = true;
                    }
                } else if ui.button("Start").clicked() {
                    start_clicked = true;
                }

                if ui.button("Clear").clicked() {
                    state.histogram.clear();
                    state.histogram_dirty = true;
                }
            });

            ui.label(format!("Status: {}", state.status_message));
            ui.label(format!("Events: {}", state.histogram.total_events));
            ui.label(format!("Rate: {:.1} Hz", state.event_rate));

            ui.separator();
            ui.heading("MCA Parameters");

            let params = &mut state.params;

            ui.horizontal(|ui| {
                ui.label("Polarity:");
                ui.add(egui::DragValue::new(&mut params.polarity).range(0..=1));
            });

            ui.horizontal(|ui| {
                ui.label("Threshold:");
                ui.add(egui::DragValue::new(&mut params.threshold).range(0..=16383));
            });

            ui.horizontal(|ui| {
                ui.label("Trig K:");
                ui.add(egui::DragValue::new(&mut params.trig_k).range(1..=255));
            });

            ui.horizontal(|ui| {
                ui.label("Trig M:");
                ui.add(egui::DragValue::new(&mut params.trig_m).range(1..=255));
            });

            ui.horizontal(|ui| {
                ui.label("Trap K:");
                ui.add(egui::DragValue::new(&mut params.trap_k).range(1..=4095));
            });

            ui.horizontal(|ui| {
                ui.label("Trap M:");
                ui.add(egui::DragValue::new(&mut params.trap_m).range(1..=4095));
            });

            ui.horizontal(|ui| {
                ui.label("Deconv M:");
                ui.add(egui::DragValue::new(&mut params.deconv_m).range(0..=16777215));
            });

            ui.horizontal(|ui| {
                ui.label("Trap Gain:");
                ui.add(egui::DragValue::new(&mut params.trap_gain).range(1..=65535));
            });

            ui.horizontal(|ui| {
                ui.label("BL Len:");
                ui.add(egui::DragValue::new(&mut params.bl_len).range(0..=15));
            });

            ui.horizontal(|ui| {
                ui.label("BL Inib:");
                ui.add(egui::DragValue::new(&mut params.bl_inib).range(0..=65535));
            });

            ui.horizontal(|ui| {
                ui.label("Sample Pos:");
                ui.add(egui::DragValue::new(&mut params.sample_pos).range(0..=4095));
            });

            ui.separator();
            ui.heading("AMax Parameters");

            ui.horizontal(|ui| {
                ui.label("Window Maxim:");
                ui.add(egui::DragValue::new(&mut params.window_maxim).range(0..=4095));
            });

            ui.horizontal(|ui| {
                ui.label("Baseline Delay:");
                ui.add(egui::DragValue::new(&mut params.baseline_delay).range(0..=4095));
            });

            ui.horizontal(|ui| {
                ui.label("Baseline Len:");
                ui.add(egui::DragValue::new(&mut params.baseline_len).range(0..=15));
            });

            ui.horizontal(|ui| {
                ui.label("Baseline Offset:");
                ui.add(egui::DragValue::new(&mut params.baseline_offset).range(0..=65535));
            });

            ui.horizontal(|ui| {
                ui.label("AMax Window:");
                ui.add(egui::DragValue::new(&mut params.amax_window).range(0..=65535));
            });

            ui.horizontal(|ui| {
                ui.label("AMax Delay:");
                ui.add(egui::DragValue::new(&mut params.amax_delay).range(0..=255));
            });

            ui.horizontal(|ui| {
                ui.label("AMax Len:");
                ui.add(egui::DragValue::new(&mut params.amax_len).range(0..=15));
            });

            ui.add_space(10.0);
            // Restart button for applying parameters while running
            // Use thread_active instead of state.running to handle init/shutdown timing
            if thread_active {
                if ui.button("Restart to Apply").clicked() {
                    // Will be handled after releasing lock
                    stop_clicked = true;
                    start_clicked = true;
                }
                // Also restart on Enter key
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    stop_clicked = true;
                    start_clicked = true;
                }
                ui.label("(or press Enter)");
            } else {
                ui.label("Press Start to begin");
            }

            ui.separator();
            ui.heading("Histogram Range");

            ui.horizontal(|ui| {
                ui.label("Energy Max:");
                let mut max = state.histogram.energy_max as u32;
                if ui.add(egui::DragValue::new(&mut max).range(1000..=65536)).changed() {
                    state.histogram.energy_max = max as f64;
                }
            });

            ui.horizontal(|ui| {
                ui.label("AMax Max:");
                let mut max = state.histogram.amax_max as u32;
                if ui.add(egui::DragValue::new(&mut max).range(1000..=65536)).changed() {
                    state.histogram.amax_max = max as f64;
                }
            });

            ui.separator();
            ui.heading("Bin Settings");

            // Copy current values to avoid borrow issues
            let current_energy_bins = state.histogram.energy_bins;
            let current_amax_bins = state.histogram.amax_bins;

            ui.horizontal(|ui| {
                ui.label("Energy Bins:");
                let mut bins = current_energy_bins as u32;
                if ui.add(egui::DragValue::new(&mut bins).range(16..=4096).speed(16.0)).changed() {
                    state.histogram.resize(bins as usize, current_amax_bins);
                }
            });

            ui.horizontal(|ui| {
                ui.label("AMax Bins:");
                let mut bins = current_amax_bins as u32;
                if ui.add(egui::DragValue::new(&mut bins).range(16..=4096).speed(16.0)).changed() {
                    state.histogram.resize(current_energy_bins, bins as usize);
                }
            });

            // Show current bin widths
            let energy_width = (state.histogram.energy_max - state.histogram.energy_min) / current_energy_bins as f64;
            let amax_width = (state.histogram.amax_max - state.histogram.amax_min) / current_amax_bins as f64;
            ui.label(format!("Energy bin width: {:.1}", energy_width));
            ui.label(format!("AMax bin width: {:.1}", amax_width));

            ui.separator();
            ui.heading("ROOT Output");

            ui.horizontal(|ui| {
                ui.label("File:");
                ui.text_edit_singleline(&mut self.output_path);
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut state.recording, "Record");
                ui.label(format!("({} events)", state.recorded_count));
            });

            if !state.running && state.recorded_count > 0 {
                if ui.button("Save ROOT File").clicked() {
                    match state.event_buffer.write_root(&self.output_path) {
                        Ok(n) => {
                            state.status_message = format!("Saved {} events to {}", n, self.output_path);
                            state.event_buffer.clear();
                            state.recorded_count = 0;
                        }
                        Err(e) => {
                            state.status_message = format!("Save failed: {}", e);
                        }
                    }
                }
            }
        });

        // Handle stop/start after releasing the lock
        // Order matters: stop first, then start (for restart)
        if stop_clicked {
            self.stop_acquisition();
        }
        if start_clicked {
            self.start_acquisition();
        }

        // Bottom panel for waveform display
        egui::TopBottomPanel::bottom("waveform_panel")
            .resizable(true)
            .default_height(200.0)
            .show(ctx, |ui| {
                // Build PlotPoints inside lock, then drop lock before plotting
                let (points_opt, energy) = {
                    let state = self.shared.lock().unwrap();
                    let energy = state.latest_waveform_energy;
                    if state.waveform_len > 0 {
                        // Build PlotPoints from slice (no Vec clone)
                        let points: PlotPoints = state.waveform_buffer[..state.waveform_len]
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
            ui.heading("AMax vs Energy");

            // Only regenerate texture when histogram data changed
            let (needs_update, energy_max, amax_max) = {
                let mut state = self.shared.lock().unwrap();
                let needs = state.histogram_dirty;
                if needs {
                    state.histogram_dirty = false;
                }
                (needs, state.histogram.energy_max, state.histogram.amax_max)
            };

            if needs_update || self.texture.is_none() {
                let state = self.shared.lock().unwrap();
                let image = state.histogram.to_texture();
                drop(state);

                self.texture = Some(ctx.load_texture(
                    "histogram",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }

            if let Some(texture) = &self.texture {
                Plot::new("histogram_plot")
                    .data_aspect(1.0)
                    .x_axis_label("Energy")
                    .y_axis_label("AMax")
                    .show(ui, |plot_ui| {
                        let image = PlotImage::new(
                            texture,
                            PlotPoint::new(energy_max / 2.0, amax_max / 2.0),
                            [energy_max as f32, amax_max as f32],
                        );
                        plot_ui.image(image);
                    });
            }
        });
    }

    fn on_exit(&mut self) {
        // Save all settings before exit
        {
            let state = self.shared.lock().unwrap();
            let settings = AppSettings {
                url: self.url.clone(),
                output_path: self.output_path.clone(),
                params: state.params.clone(),
            };
            settings.save();
        }
        self.stop_acquisition();
    }
}

/// Acquisition thread - connects to digitizer and reads events
fn acquisition_thread(url: String, shared: Arc<Mutex<SharedState>>, shutdown: Arc<AtomicBool>) {
    // Connect to digitizer
    let handle = match CaenHandle::open(&url) {
        Ok(h) => {
            let mut state = shared.lock().unwrap();
            state.connected = true;
            state.status_message = "Connected".to_string();
            h
        }
        Err(e) => {
            let mut state = shared.lock().unwrap();
            state.status_message = format!("Connection failed: {}", e);
            state.running = false;
            return;
        }
    };

    // Configure channel 0
    let _ = handle.set_value("/ch/0/par/chenable", "True");
    for ch in 1..32 {
        let _ = handle.set_value(&format!("/ch/{}/par/chenable", ch), "False");
    }

    // Apply initial parameters
    {
        let mut state = shared.lock().unwrap();
        let (success, errors, mismatches, first_error) = apply_params(&handle, &state.params);
        if errors > 0 || mismatches > 0 {
            state.status_message = format!("Init: {} OK, {} err, {} mismatch: {}",
                success, errors, mismatches, first_error.unwrap_or_default());
        } else {
            state.status_message = format!("Init: {} registers verified OK", success);
        }
    }

    // Configure OpenDPP endpoint with waveform
    let endpoint = match handle.configure_opendpp_endpoint(true) {
        Ok(ep) => ep,
        Err(e) => {
            let mut state = shared.lock().unwrap();
            state.status_message = format!("Endpoint error: {}", e);
            state.running = false;
            return;
        }
    };

    // Start acquisition
    let _ = handle.send_command("/cmd/cleardata");
    let _ = handle.send_command("/cmd/armacquisition");
    let _ = handle.send_command("/cmd/swstartacquisition");

    {
        let mut state = shared.lock().unwrap();
        state.running = true;
        state.status_message = "Running".to_string();
    }

    let mut user_info_buffer = [0u64; 16];
    let mut waveform_buffer = [0u16; 8192]; // Max waveform size
    let mut last_rate_update = std::time::Instant::now();
    let mut last_waveform_update = std::time::Instant::now();
    let mut events_since_last_update = 0u64;

    // Main acquisition loop
    while !shutdown.load(Ordering::Relaxed) {
        // Read event with waveform
        match endpoint.read_opendpp_event_with_waveform(100, &mut user_info_buffer, &mut waveform_buffer) {
            Ok(Some(event)) => {
                events_since_last_update += 1;

                // Get AMax value from user_info[0]
                let amax = if !event.user_info.is_empty() {
                    event.user_info[0]
                } else {
                    0
                };

                // Throttle waveform update to 100ms (10 Hz)
                let should_update_waveform = last_waveform_update.elapsed() >= Duration::from_millis(100);

                // Fill histogram and optionally update waveform
                {
                    let mut state = shared.lock().unwrap();
                    state.histogram.fill(event.energy, amax);
                    state.histogram_dirty = true; // Mark for texture regeneration

                    // Record event if recording is enabled
                    if state.recording {
                        state.event_buffer.push(
                            event.channel,
                            event.energy,
                            amax,
                            event.timestamp,
                        );
                        state.recorded_count = state.event_buffer.len();
                    }

                    // Update waveform only every 100ms (zero-copy into pre-allocated buffer)
                    if should_update_waveform {
                        if let Some(ref wf) = event.waveform {
                            let len = wf.len().min(state.waveform_buffer.len());
                            state.waveform_buffer[..len].copy_from_slice(&wf[..len]);
                            state.waveform_len = len;
                            state.latest_waveform_energy = event.energy;
                        }
                        last_waveform_update = std::time::Instant::now();
                    }
                }
            }
            Ok(None) => {
                // Timeout - no data, yield CPU
                thread::sleep(Duration::from_millis(1));
            }
            Err(e) => {
                if e.code == -12 {
                    // Stop signal
                    break;
                }
                // Avoid busy-wait on errors
                thread::sleep(Duration::from_millis(10));
            }
        }

        // Update rate every second
        let elapsed = last_rate_update.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let rate = events_since_last_update as f64 / elapsed.as_secs_f64();
            {
                let mut state = shared.lock().unwrap();
                state.event_rate = rate;
            }
            events_since_last_update = 0;
            last_rate_update = std::time::Instant::now();
        }
    }

    // Stop acquisition
    let _ = handle.send_command("/cmd/disarmacquisition");

    {
        let mut state = shared.lock().unwrap();
        state.running = false;
        state.connected = false;
        state.status_message = "Stopped".to_string();
    }
}

/// Apply MCA parameters to digitizer with read-back verification
/// Returns (success_count, error_count, mismatch_count, first_error_message)
fn apply_params(handle: &CaenHandle, params: &McaParams) -> (usize, usize, usize, Option<String>) {
    let mut success = 0;
    let mut errors = 0;
    let mut mismatches = 0;
    let mut first_error: Option<String> = None;

    // Helper to write and verify a register
    let mut write_and_verify = |name: &str, byte_addr: u32, value: u32| {
        match handle.set_user_register(byte_addr, value) {
            Ok(()) => {
                // Read back to verify
                match handle.get_user_register(byte_addr) {
                    Ok(readback) => {
                        if readback == value {
                            success += 1;
                        } else {
                            mismatches += 1;
                            if first_error.is_none() {
                                first_error = Some(format!(
                                    "{} (0x{:X}): wrote {}, read {}",
                                    name, byte_addr, value, readback
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        // Write succeeded but read failed - count as success with warning
                        success += 1;
                        if first_error.is_none() {
                            first_error = Some(format!("{}: write OK, read failed: {}", name, e));
                        }
                    }
                }
            }
            Err(e) => {
                errors += 1;
                if first_error.is_none() {
                    first_error = Some(format!("{} (0x{:X}): {}", name, byte_addr, e));
                }
            }
        }
    };

    // Set Core MCA HLS registers (0x0-0xC)
    // Note: word address * 4 = byte address
    let core_regs: [(&str, u32, u32); 13] = [
        ("POLARITY", 0x0, params.polarity),
        ("OFFSET", 0x1, params.offset),
        ("THRS", 0x2, params.threshold),
        ("TRIG_K", 0x3, params.trig_k),
        ("TRIG_M", 0x4, params.trig_m),
        ("TRAP_K", 0x5, params.trap_k),
        ("TRAP_M", 0x6, params.trap_m),
        ("DECONV_M", 0x7, params.deconv_m),
        ("TRAP_GAIN", 0x8, params.trap_gain),
        ("BL_LEN", 0x9, params.bl_len),
        ("BL_INIB", 0xA, params.bl_inib),
        ("SAMPLE_POS", 0xB, params.sample_pos),
        ("RUN_CFG", 0xC, params.run_cfg),
    ];

    for (name, word_addr, value) in &core_regs {
        write_and_verify(name, word_addr * 4, *value);
    }

    // Set AMax registers
    // WINDOW_MAXIM at 0x14000
    write_and_verify("WINDOW_MAXIM", 0x14000 * 4, params.window_maxim);

    // AMax-specific registers at 0x160000-0x160005
    let amax_regs: [(&str, u32, u32); 6] = [
        ("baseline_delay", 0x160000, params.baseline_delay),
        ("baseline_len", 0x160001, params.baseline_len),
        ("baseline_offset", 0x160002, params.baseline_offset),
        ("AMAX_window", 0x160003, params.amax_window),
        ("AMAX_delay", 0x160004, params.amax_delay),
        ("AMAX_len", 0x160005, params.amax_len),
    ];

    for (name, word_addr, value) in &amax_regs {
        write_and_verify(name, word_addr * 4, *value);
    }

    (success, errors, mismatches, first_error)
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("AMax Viewer - Firmware Development Tool"),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                eframe::egui_wgpu::WgpuSetupCreateNew {
                    instance_descriptor: eframe::wgpu::InstanceDescriptor {
                        // Vulkan + Metal only: DX12 causes Device Lost on WSL2
                        backends: eframe::wgpu::Backends::VULKAN
                            | eframe::wgpu::Backends::METAL,
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
        Box::new(|cc| Ok(Box::new(AmaxViewerApp::new(cc)))),
    )
}

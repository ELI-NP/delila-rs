//! Online Event Builder v2
//!
//! チャンク＋ソート＋Safe Horizon 方式のオンラインイベントビルダー。
//!
//! # アーキテクチャ
//!
//! ```text
//! [Receiver]     [Sorter]        [Workers×N]     [Writers×M]
//!  tokio task   std::thread    std::thread×N    std::thread×M
//!     │              │              │              │
//!  ZMQ SUB      accumulate     build_events    local buffer
//!     │          + sort             │           extend(batch)
//!     │          + safe_cut         │           threshold→write
//!     ▼              ▼              ▼              ▼
//!  Vec<Hit> ──→ SortedChunk ──→ Vec<Built> ──→ ROOT files
//! (tokio mpsc) (crossbeam)    (crossbeam MPMC) (parallel I/O)
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel as crossbeam;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use super::built_event::BuiltEvent;
use super::chunk_builder::{
    build_events_from_chunk, sort_and_flush, sort_and_split, SortedChunk, TriggerConfig,
};
use super::config::{load_channel_config, TimeCalibration};
use super::hit::Hit;

use crate::common::Message;

/// Online Event Builder configuration
#[derive(Debug, Clone)]
pub struct OnlineEventBuilderConfig {
    /// ZMQ SUB address to subscribe to
    pub subscribe_address: String,
    /// ZMQ REP address for commands
    pub command_address: String,
    /// Output directory for ROOT files
    pub output_dir: PathBuf,
    /// Coincidence window [ns]
    pub coincidence_window_ns: f64,
    /// Safe horizon [ns] (network disorder buffer)
    pub safe_horizon_ns: f64,
    /// Channel settings file path (for trigger/AC config)
    pub ch_settings_file: Option<String>,
    /// Time calibration file path
    pub time_calib_file: Option<String>,
    /// Number of worker threads
    pub n_workers: usize,
    /// Number of writer threads for parallel ROOT I/O
    pub n_writers: usize,
    /// Events per ROOT file before rotation
    pub events_per_file: usize,
    /// Sorter accumulation threshold (hits)
    pub sorter_threshold: usize,
    /// Sorter timeout (force flush if no data for this duration)
    pub sorter_timeout: Duration,
}

impl Default for OnlineEventBuilderConfig {
    fn default() -> Self {
        Self {
            subscribe_address: "tcp://localhost:5557".to_string(),
            command_address: "tcp://*:5595".to_string(),
            output_dir: PathBuf::from("./data/events"),
            coincidence_window_ns: 100.0,
            safe_horizon_ns: 50_000_000.0, // 50ms
            ch_settings_file: None,
            time_calib_file: None,
            n_workers: 4,
            n_writers: 4,
            events_per_file: 100_000,
            sorter_threshold: 500_000,
            sorter_timeout: Duration::from_millis(500),
        }
    }
}

/// Pipeline statistics
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    pub received_hits: u64,
    pub received_batches: u64,
    pub dropped_batches: u64,
    pub chunks_processed: u64,
    pub events_built: u64,
    pub files_written: u64,
}

/// Internal messages between pipeline stages
enum SorterInput {
    Hits(Vec<Hit>),
    Eos,
}

/// Online Event Builder
///
/// Receives hits via ZMQ SUB, builds events, writes to ROOT files.
pub struct OnlineEventBuilder {
    config: OnlineEventBuilderConfig,
    trigger_config: Arc<TriggerConfig>,
    time_calibration: TimeCalibration,
}

impl OnlineEventBuilder {
    /// Create a new Online Event Builder
    pub fn new(config: OnlineEventBuilderConfig) -> anyhow::Result<Self> {
        // Load trigger config from channel settings
        let trigger_config = if let Some(ref path) = config.ch_settings_file {
            load_trigger_config_from_file(path, config.coincidence_window_ns)?
        } else {
            warn!("No ch_settings_file configured, using empty trigger config");
            TriggerConfig {
                triggers: Default::default(),
                priorities: Default::default(),
                ac_pairs: Default::default(),
                coincidence_window_ns: config.coincidence_window_ns,
            }
        };

        // Load time calibration
        let time_calibration = if let Some(ref path) = config.time_calib_file {
            match TimeCalibration::from_json_file(std::path::Path::new(path)) {
                Ok(tc) => {
                    info!(path = %path, offsets = tc.offsets().len(), "Loaded time calibration");
                    tc
                }
                Err(e) => {
                    warn!(path = %path, error = %e, "Failed to load time calibration, using defaults");
                    TimeCalibration::new(0, 0)
                }
            }
        } else {
            TimeCalibration::new(0, 0)
        };

        Ok(Self {
            config,
            trigger_config: Arc::new(trigger_config),
            time_calibration,
        })
    }

    /// Run the event builder pipeline
    ///
    /// Blocks until shutdown signal is received or EOS is detected.
    pub async fn run(self, mut shutdown: broadcast::Receiver<()>) -> anyhow::Result<PipelineStats> {
        let config = self.config.clone();
        let trigger_config = self.trigger_config.clone();
        let time_calibration = self.time_calibration.clone();

        // Stats counters
        let received_hits = Arc::new(AtomicU64::new(0));
        let received_batches = Arc::new(AtomicU64::new(0));
        let dropped_batches = Arc::new(AtomicU64::new(0));
        let chunks_processed = Arc::new(AtomicU64::new(0));
        let events_built = Arc::new(AtomicU64::new(0));
        let files_written = Arc::new(AtomicU64::new(0));

        // Channels
        let (hit_tx, hit_rx) = tokio::sync::mpsc::unbounded_channel::<SorterInput>();
        let (chunk_tx, chunk_rx) = crossbeam::bounded::<SortedChunk>(16);
        let (writer_tx, writer_rx) = crossbeam::bounded::<Vec<BuiltEvent>>(64);

        // --- Writer threads (parallel ROOT I/O) ---
        let file_index = Arc::new(AtomicU32::new(0));
        let mut writer_handles = Vec::new();

        // Ensure output directory exists (once, before spawning writers)
        if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
            error!(error = %e, dir = %config.output_dir.display(), "Failed to create output directory");
            anyhow::bail!("Failed to create output directory: {}", e);
        }

        for i in 0..config.n_writers {
            let writer_rx = writer_rx.clone();
            let output_dir = config.output_dir.clone();
            let file_index = file_index.clone();
            let files_written = files_written.clone();
            let events_per_file = config.events_per_file;

            let handle = std::thread::Builder::new()
                .name(format!("eb-writer-{}", i))
                .spawn(move || {
                    writer_thread(
                        writer_rx,
                        output_dir,
                        events_per_file,
                        file_index,
                        files_written,
                    );
                })?;
            writer_handles.push(handle);
        }
        drop(writer_rx);

        // --- Worker threads ---
        let mut worker_handles = Vec::new();
        let next_event_id = Arc::new(AtomicU64::new(0));

        for i in 0..config.n_workers {
            let chunk_rx = chunk_rx.clone();
            let writer_tx = writer_tx.clone();
            let trigger_config = trigger_config.clone();
            let events_built = events_built.clone();
            let next_event_id = next_event_id.clone();

            let handle = std::thread::Builder::new()
                .name(format!("eb-worker-{}", i))
                .spawn(move || {
                    worker_thread(
                        chunk_rx,
                        writer_tx,
                        &trigger_config,
                        events_built,
                        next_event_id,
                    );
                })?;
            worker_handles.push(handle);
        }
        drop(chunk_rx);
        drop(writer_tx);

        // --- Sorter thread ---
        let sorter_threshold = config.sorter_threshold;
        let sorter_timeout = config.sorter_timeout;
        let safe_horizon_ns = config.safe_horizon_ns;
        let sorter_chunks_processed = chunks_processed.clone();
        let sorter_time_calib = time_calibration;

        let sorter_handle =
            std::thread::Builder::new()
                .name("eb-sorter".into())
                .spawn(move || {
                    sorter_thread(
                        hit_rx,
                        chunk_tx,
                        safe_horizon_ns,
                        sorter_threshold,
                        sorter_timeout,
                        sorter_chunks_processed,
                        sorter_time_calib,
                    );
                })?;

        // --- Receiver task (tokio) ---
        let recv_hits = received_hits.clone();
        let recv_batches = received_batches.clone();
        let recv_dropped = dropped_batches.clone();

        let mut receiver_handle = tokio::spawn(async move {
            receiver_task(
                config.subscribe_address.clone(),
                hit_tx,
                recv_hits,
                recv_batches,
                recv_dropped,
            )
            .await;
        });

        // --- Wait for shutdown or receiver completion ---
        tokio::select! {
            biased;
            _ = shutdown.recv() => {
                info!("Online Event Builder received shutdown signal");
            }
            _ = &mut receiver_handle => {
                info!("Receiver task completed (EOS or connection lost)");
            }
        }

        // Abort receiver to drop hit_tx → sorter detects channel closure
        receiver_handle.abort();

        // Wait for pipeline to drain
        info!("Waiting for pipeline to drain...");
        if let Err(e) = sorter_handle.join() {
            error!("Sorter thread panicked: {:?}", e);
        }
        for (i, handle) in worker_handles.into_iter().enumerate() {
            if let Err(e) = handle.join() {
                error!("Worker thread {} panicked: {:?}", i, e);
            }
        }
        for (i, handle) in writer_handles.into_iter().enumerate() {
            if let Err(e) = handle.join() {
                error!("Writer thread {} panicked: {:?}", i, e);
            }
        }

        let stats = PipelineStats {
            received_hits: received_hits.load(Ordering::Relaxed),
            received_batches: received_batches.load(Ordering::Relaxed),
            dropped_batches: dropped_batches.load(Ordering::Relaxed),
            chunks_processed: chunks_processed.load(Ordering::Relaxed),
            events_built: events_built.load(Ordering::Relaxed),
            files_written: files_written.load(Ordering::Relaxed),
        };

        info!(
            received_hits = stats.received_hits,
            received_batches = stats.received_batches,
            dropped_batches = stats.dropped_batches,
            chunks_processed = stats.chunks_processed,
            events_built = stats.events_built,
            files_written = stats.files_written,
            "Online Event Builder finished"
        );

        Ok(stats)
    }
}

// ===========================================================================
// Pipeline stages
// ===========================================================================

/// Receiver task: ZMQ SUB → tokio mpsc → Sorter
async fn receiver_task(
    subscribe_address: String,
    hit_tx: tokio::sync::mpsc::UnboundedSender<SorterInput>,
    received_hits: Arc<AtomicU64>,
    received_batches: Arc<AtomicU64>,
    dropped_batches: Arc<AtomicU64>,
) {
    use futures::StreamExt;
    use tmq::AsZmqSocket;

    let context = tmq::Context::new();
    let mut socket = match tmq::subscribe(&context)
        .connect(&subscribe_address)
        .and_then(|s| s.subscribe(b""))
    {
        Ok(s) => {
            // Never drop messages — buffer in memory instead (DAQ: no data loss)
            if let Err(e) = s.get_socket().set_rcvhwm(0) {
                warn!(error = %e, "Failed to set ZMQ RCVHWM=0");
            }
            s
        }
        Err(e) => {
            error!(address = %subscribe_address, error = %e, "Failed to connect ZMQ SUB");
            return;
        }
    };

    info!(address = %subscribe_address, "Receiver connected");

    // Timeout: if no data received for this duration after first data, assume EOS lost
    let no_data_timeout = Duration::from_secs(5);
    let mut has_received_data = false;

    loop {
        let recv_future = socket.next();

        // Apply timeout only after we've received at least some data
        let result = if has_received_data {
            match tokio::time::timeout(no_data_timeout, recv_future).await {
                Ok(Some(result)) => result,
                Ok(None) => break, // Stream ended
                Err(_) => {
                    // Timeout — likely EOS was lost (ZMQ PUB HWM drop)
                    warn!(
                        timeout_secs = no_data_timeout.as_secs(),
                        "No data received, assuming EOS lost (ZMQ HWM drop)"
                    );
                    let _ = hit_tx.send(SorterInput::Eos);
                    break;
                }
            }
        } else {
            match socket.next().await {
                Some(result) => result,
                None => break,
            }
        };

        match result {
            Ok(multipart) => {
                if let Some(frame) = multipart.iter().next() {
                    match Message::from_msgpack(frame) {
                        Ok(Message::Data(batch)) => {
                            has_received_data = true;
                            let n = batch.events.len() as u64;
                            received_batches.fetch_add(1, Ordering::Relaxed);
                            received_hits.fetch_add(n, Ordering::Relaxed);

                            // Convert EventData → Hit
                            let hits: Vec<Hit> =
                                batch.events.iter().map(Hit::from_event_data).collect();

                            if hit_tx.send(SorterInput::Hits(hits)).is_err() {
                                dropped_batches.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Ok(Message::EndOfStream { source_id }) => {
                            info!(source_id, "Received EOS");
                            let _ = hit_tx.send(SorterInput::Eos);
                            break;
                        }
                        Ok(Message::Heartbeat(_)) => {
                            has_received_data = true;
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to deserialize message");
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "ZMQ receive error");
            }
        }
    }

    info!("Receiver task finished");
}

/// Sorter thread: accumulate → sort → safe_cut → send to workers
fn sorter_thread(
    hit_rx: tokio::sync::mpsc::UnboundedReceiver<SorterInput>,
    chunk_tx: crossbeam::Sender<SortedChunk>,
    safe_horizon_ns: f64,
    threshold: usize,
    timeout: Duration,
    chunks_processed: Arc<AtomicU64>,
    time_calibration: TimeCalibration,
) {
    // We need a blocking receiver. tokio mpsc Receiver has blocking_recv().
    let mut hit_rx = hit_rx;
    let mut buffer: Vec<Hit> = Vec::with_capacity(threshold * 2);
    let mut last_flush = Instant::now();

    let mut channel_closed = false;

    loop {
        // Try to receive with timeout
        let input = if buffer.len() >= threshold {
            // Buffer full, try non-blocking receive then process
            match hit_rx.try_recv() {
                Ok(input) => Some(input),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    channel_closed = true;
                    None
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
            }
        } else {
            // Wait for data or timeout
            let remaining = timeout.saturating_sub(last_flush.elapsed());
            if remaining.is_zero() {
                None // Timeout expired, process what we have
            } else {
                // blocking_recv with timeout
                match hit_rx.blocking_recv() {
                    Some(input) => Some(input),
                    None => {
                        channel_closed = true;
                        None
                    }
                }
            }
        };

        match input {
            Some(SorterInput::Hits(mut hits)) => {
                // Apply time calibration
                for hit in &mut hits {
                    let offset = time_calibration.get_offset(hit.module, hit.channel);
                    hit.timestamp_ns -= offset;
                }
                buffer.extend(hits);
            }
            Some(SorterInput::Eos) => {
                channel_closed = true;
            }
            None => {
                // Timeout, buffer full, or channel closed — try to split below
            }
        }

        // Channel closed (EOS or sender dropped): flush all remaining data and exit
        if channel_closed {
            info!(buffer_size = buffer.len(), "Sorter flushing remaining data");
            if let Some(chunk) = sort_and_flush(buffer) {
                chunks_processed.fetch_add(1, Ordering::Relaxed);
                if chunk_tx.send(chunk).is_err() {
                    error!("Failed to send final chunk to workers");
                }
            }
            break;
        }

        // Try to produce a chunk if we have enough data
        if buffer.len() >= threshold || last_flush.elapsed() >= timeout {
            match sort_and_split(buffer, safe_horizon_ns) {
                Ok((chunk, retained)) => {
                    chunks_processed.fetch_add(1, Ordering::Relaxed);
                    if chunk_tx.send(chunk).is_err() {
                        error!("Failed to send chunk to workers (channel closed)");
                        break;
                    }
                    buffer = retained;
                    last_flush = Instant::now();
                }
                Err(returned) => {
                    buffer = returned;
                    // Not enough time spread — reset timer to avoid tight spin.
                    // Without this, the next iteration sees timeout expired and
                    // never blocks on blocking_recv(), causing 100% CPU spin.
                    last_flush = Instant::now();
                }
            }
        }
    }

    // chunk_tx is dropped here → workers detect channel close
    info!("Sorter thread finished");
}

/// Worker thread: receive chunks → build events → send to writers
fn worker_thread(
    chunk_rx: crossbeam::Receiver<SortedChunk>,
    writer_tx: crossbeam::Sender<Vec<BuiltEvent>>,
    trigger_config: &TriggerConfig,
    events_built: Arc<AtomicU64>,
    next_event_id: Arc<AtomicU64>,
) {
    while let Ok(chunk) = chunk_rx.recv() {
        let mut events = build_events_from_chunk(&chunk, trigger_config);

        // Assign sequential event IDs (atomic for cross-worker ordering)
        for event in &mut events {
            event.event_id = next_event_id.fetch_add(1, Ordering::Relaxed);
        }

        let n = events.len() as u64;
        events_built.fetch_add(n, Ordering::Relaxed);

        if !events.is_empty() && writer_tx.send(events).is_err() {
            error!("Failed to send events to writer (channel closed)");
            break;
        }
    }
}

/// Writer thread: receive event batches → accumulate → write to ROOT files
///
/// Each writer thread maintains a local buffer. When it reaches the threshold,
/// it sorts by trigger_time and writes to a ROOT file. Multiple writer threads
/// run concurrently for parallel I/O (MPMC via crossbeam).
#[cfg(feature = "root")]
fn writer_thread(
    writer_rx: crossbeam::Receiver<Vec<BuiltEvent>>,
    output_dir: PathBuf,
    events_per_file: usize,
    file_index: Arc<AtomicU32>,
    files_written: Arc<AtomicU64>,
) {
    use super::root_io::write_events_to_root;

    let run_id = 9999; // TODO: get from config/command
                       // Pre-allocate with +10% margin; clear() preserves capacity for reuse
    let mut buffer: Vec<BuiltEvent> = Vec::with_capacity(events_per_file + events_per_file / 10);

    while let Ok(batch) = writer_rx.recv() {
        buffer.extend(batch); // move each event, no clone

        if buffer.len() >= events_per_file {
            buffer.sort_unstable_by(|a, b| a.trigger_time.total_cmp(&b.trigger_time));

            let idx = file_index.fetch_add(1, Ordering::Relaxed);
            let file_path = output_dir.join(format!("eb_run{:04}_{:04}_events.root", run_id, idx));

            match write_events_to_root(&file_path, "EventTree", &buffer) {
                Ok(()) => {
                    files_written.fetch_add(1, Ordering::Relaxed);
                    info!(
                        file = %file_path.display(),
                        events = buffer.len(),
                        "Wrote ROOT file"
                    );
                }
                Err(e) => {
                    error!(error = %e, file = %file_path.display(), "Failed to write ROOT file");
                }
            }
            buffer.clear(); // capacity preserved for reuse
        }
    }

    // Write remaining events on channel close
    if !buffer.is_empty() {
        buffer.sort_unstable_by(|a, b| a.trigger_time.total_cmp(&b.trigger_time));

        let idx = file_index.fetch_add(1, Ordering::Relaxed);
        let file_path = output_dir.join(format!("eb_run{:04}_{:04}_events.root", run_id, idx));

        match write_events_to_root(&file_path, "EventTree", &buffer) {
            Ok(()) => {
                files_written.fetch_add(1, Ordering::Relaxed);
                info!(
                    file = %file_path.display(),
                    events = buffer.len(),
                    "Wrote remaining events"
                );
            }
            Err(e) => {
                error!(error = %e, file = %file_path.display(), "Failed to write remaining ROOT file");
            }
        }
    }
}

/// Writer thread stub when ROOT feature is not enabled
#[cfg(not(feature = "root"))]
fn writer_thread(
    writer_rx: crossbeam::Receiver<Vec<BuiltEvent>>,
    _output_dir: PathBuf,
    _events_per_file: usize,
    _file_index: Arc<AtomicU32>,
    _files_written: Arc<AtomicU64>,
) {
    let mut total = 0u64;
    while let Ok(batch) = writer_rx.recv() {
        total += batch.len() as u64;
    }
    info!(
        events_received = total,
        "Writer thread finished (ROOT disabled, no files written)"
    );
}

// ===========================================================================
// Helper functions
// ===========================================================================

/// Load trigger config from channel settings JSON file
///
/// ChannelConfig = Vec<Vec<ChSettings>> — outer = modules, inner = channels
fn load_trigger_config_from_file(
    path: &str,
    coincidence_window_ns: f64,
) -> anyhow::Result<TriggerConfig> {
    use std::path::Path;

    let ch_settings = load_channel_config(Path::new(path))?;
    let mut triggers = std::collections::HashSet::new();
    let mut priorities = std::collections::HashMap::new();
    let mut ac_pairs = std::collections::HashMap::new();

    for module_channels in &ch_settings {
        for settings in module_channels {
            let key = (settings.module, settings.channel);
            if settings.is_event_trigger {
                triggers.insert(key);
                // Use detector ID as priority (lower ID = higher priority)
                priorities.insert(key, settings.id as u32);
            }
            if settings.has_ac {
                ac_pairs.insert(key, (settings.ac_module, settings.ac_channel));
            }
        }
    }

    info!(
        triggers = triggers.len(),
        ac_pairs = ac_pairs.len(),
        window_ns = coincidence_window_ns,
        "Loaded trigger config from {}",
        path
    );

    Ok(TriggerConfig {
        triggers,
        priorities,
        ac_pairs,
        coincidence_window_ns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = OnlineEventBuilderConfig::default();
        assert_eq!(config.coincidence_window_ns, 100.0);
        assert_eq!(config.safe_horizon_ns, 50_000_000.0);
        assert_eq!(config.n_workers, 4);
        assert_eq!(config.n_writers, 4);
        assert_eq!(config.events_per_file, 100_000);
    }

    #[test]
    fn test_pipeline_stats_default() {
        let stats = PipelineStats::default();
        assert_eq!(stats.received_hits, 0);
        assert_eq!(stats.events_built, 0);
    }
}

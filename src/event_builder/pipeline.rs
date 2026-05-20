//! Unified Event Builder Pipeline
//!
//! Sorter → Workers → Writers パイプライン。
//! HitSource trait で入力を抽象化し、オンライン/オフラインで同一コードを使用。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel as crossbeam;
use tracing::{error, info, warn};

use super::built_event::BuiltEvent;
use super::chunk_builder::{
    build_events_from_chunk, sort_and_flush, sort_and_split, TriggerConfig,
};
use super::config::TimeCalibration;
use super::eb_message::{BuiltEventBatch, EbMessage};
use super::l2_eval::L2Filter;
use super::source::{HitBatch, HitSource, SourceError};

/// Pipeline configuration
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Safe Horizon [ns] for sort_and_split boundary
    pub safe_horizon_ns: f64,
    /// Number of worker threads (event building)
    pub n_workers: usize,
    /// Number of writer threads (ROOT I/O)
    pub n_writers: usize,
    /// Events per ROOT file before rotation
    pub events_per_file: usize,
    /// Sorter threshold: process when buffer reaches this many hits
    pub sorter_threshold: usize,
    /// Sorter timeout: process even if threshold not reached
    pub sorter_timeout: Duration,
    /// Output directory for ROOT files
    pub output_dir: PathBuf,
    /// Run ID for file naming
    pub run_id: u32,
    /// Output tree name
    pub output_tree: String,
    /// Optional ZMQ PUB endpoint for downstream EB Monitor (SPEC § 9.3).
    /// When set, every batch of built events is also published as a
    /// MessagePack [`EbMessage::Events`] frame. `None` disables the PUB
    /// stream entirely (no extra thread, no clones).
    pub zmq_pub_endpoint: Option<String>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            safe_horizon_ns: 50_000_000.0, // 50ms
            n_workers: 4,
            n_writers: 2,
            events_per_file: 100_000,
            sorter_threshold: 500_000,
            sorter_timeout: Duration::from_millis(500),
            output_dir: PathBuf::from("."),
            run_id: 0,
            output_tree: "EventTree".to_string(),
            zmq_pub_endpoint: None,
        }
    }
}

/// Pipeline statistics
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    pub received_hits: u64,
    pub received_batches: u64,
    pub chunks_processed: u64,
    /// Events that survived L1 (i.e. successfully built by chunk_builder).
    pub events_built: u64,
    /// Events that survived L2 (== events_built when no L2 filter is set).
    pub events_kept: u64,
    pub files_written: u64,
    /// Number of [`EbMessage::Events`] batches successfully published on
    /// the ZMQ PUB endpoint (0 when no endpoint is configured).
    pub batches_published: u64,
}

/// Unified Event Builder Pipeline
///
/// Orchestrates Sorter → Workers → Writers threads.
/// Input is provided via HitSource trait (online: ZMQ, offline: file).
pub struct EventBuilderPipeline {
    pub config: PipelineConfig,
    pub trigger_config: Arc<TriggerConfig>,
    pub time_calibration: TimeCalibration,
    /// Optional L2 filter applied to each built event before it leaves
    /// the worker (see SPEC § 7). When `None`, every L1-built event flows
    /// through unchanged (legacy behaviour).
    pub l2_filter: Option<Arc<L2Filter>>,
}

impl EventBuilderPipeline {
    pub fn new(
        config: PipelineConfig,
        trigger_config: TriggerConfig,
        time_calibration: TimeCalibration,
    ) -> Self {
        Self {
            config,
            trigger_config: Arc::new(trigger_config),
            time_calibration,
            l2_filter: None,
        }
    }

    /// Attach an L2 filter (named-ops chain). The pipeline will drop any
    /// event for which no `Accept` op evaluates to true.
    #[must_use]
    pub fn with_l2_filter(mut self, filter: L2Filter) -> Self {
        self.l2_filter = Some(Arc::new(filter));
        self
    }

    /// Run the pipeline to completion.
    ///
    /// This blocks until the source returns Eos or Disconnected,
    /// all events are processed, and all output files are written.
    pub fn run(self, source: impl HitSource + 'static) -> PipelineStats {
        let config = &self.config;

        // Shared atomic counters
        let received_hits = Arc::new(AtomicU64::new(0));
        let received_batches = Arc::new(AtomicU64::new(0));
        let chunks_processed = Arc::new(AtomicU64::new(0));
        let events_built = Arc::new(AtomicU64::new(0));
        let events_kept = Arc::new(AtomicU64::new(0));
        let files_written = Arc::new(AtomicU64::new(0));
        let batches_published = Arc::new(AtomicU64::new(0));
        let next_event_id = Arc::new(AtomicU64::new(0));
        let next_batch_id = Arc::new(AtomicU64::new(0));
        let file_index = Arc::new(AtomicU32::new(0));

        // Channels: Sorter → Workers → Writers
        let (chunk_tx, chunk_rx) = crossbeam::bounded(16);
        let (writer_tx, writer_rx) = crossbeam::bounded(64);
        // Optional fan-out channel: Workers → ZMQ PUB thread.
        // Unbounded because the PUB socket itself has HWM=0 (zero-loss
        // policy) — backpressure here would defeat that.
        let pub_pair: Option<(
            crossbeam::Sender<BuiltEventBatch>,
            crossbeam::Receiver<BuiltEventBatch>,
        )> = config
            .zmq_pub_endpoint
            .as_ref()
            .map(|_| crossbeam::unbounded());

        info!(
            source = source.name(),
            n_workers = config.n_workers,
            n_writers = config.n_writers,
            safe_horizon_ns = config.safe_horizon_ns,
            events_per_file = config.events_per_file,
            "Starting EventBuilderPipeline"
        );

        // Spawn writer threads
        let mut writer_handles = Vec::with_capacity(config.n_writers);
        for _ in 0..config.n_writers {
            let rx = writer_rx.clone();
            let dir = config.output_dir.clone();
            let epf = config.events_per_file;
            let fi = file_index.clone();
            let fw = files_written.clone();
            let run_id = config.run_id;
            let tree_name = config.output_tree.clone();
            writer_handles.push(std::thread::spawn(move || {
                writer_thread(rx, dir, epf, fi, fw, run_id, &tree_name);
            }));
        }
        drop(writer_rx); // Close our copy so writers detect close when all senders drop

        // Spawn the ZMQ PUB thread if an endpoint is configured.
        let pub_thread_handle = if let (Some(endpoint), Some((_pub_tx, pub_rx))) =
            (config.zmq_pub_endpoint.as_ref(), pub_pair.as_ref())
        {
            let endpoint = endpoint.clone();
            let rx = pub_rx.clone();
            let bp = batches_published.clone();
            let run_id = config.run_id;
            Some(
                std::thread::Builder::new()
                    .name("eb-zmq-pub".into())
                    .spawn(move || zmq_pub_thread(endpoint, rx, bp, run_id))
                    .expect("failed to spawn eb-zmq-pub thread"),
            )
        } else {
            None
        };
        // Drop the original receiver — only the PUB thread owns it now.
        let pub_tx_for_workers: Option<crossbeam::Sender<BuiltEventBatch>> =
            pub_pair.map(|(tx, _rx)| tx);

        // Spawn worker threads
        let mut worker_handles = Vec::with_capacity(config.n_workers);
        for _ in 0..config.n_workers {
            let crx = chunk_rx.clone();
            let wtx = writer_tx.clone();
            let tc = self.trigger_config.clone();
            let eb = events_built.clone();
            let ek = events_kept.clone();
            let nei = next_event_id.clone();
            let nbi = next_batch_id.clone();
            let l2 = self.l2_filter.clone();
            let ptx = pub_tx_for_workers.clone();
            let run_id = config.run_id;
            worker_handles.push(std::thread::spawn(move || {
                worker_thread(crx, wtx, &tc, eb, ek, nei, l2, ptx, nbi, run_id);
            }));
        }
        drop(chunk_rx); // Close our copy
        drop(writer_tx); // Close our copy so writers detect close when all workers drop
        drop(pub_tx_for_workers); // Close our copy so PUB detects close when all workers drop

        // Run sorter on this thread (blocking)
        sorter_thread(
            source,
            chunk_tx,
            config.safe_horizon_ns,
            config.sorter_threshold,
            config.sorter_timeout,
            chunks_processed.clone(),
            received_hits.clone(),
            received_batches.clone(),
            self.time_calibration,
        );

        // Wait for workers to finish
        for h in worker_handles {
            let _ = h.join();
        }
        // Wait for writers to finish
        for h in writer_handles {
            let _ = h.join();
        }
        // Wait for the ZMQ PUB thread (if any). It only exits after the
        // last worker drops its sender, so this never blocks indefinitely.
        if let Some(h) = pub_thread_handle {
            let _ = h.join();
        }

        let stats = PipelineStats {
            received_hits: received_hits.load(Ordering::Relaxed),
            received_batches: received_batches.load(Ordering::Relaxed),
            chunks_processed: chunks_processed.load(Ordering::Relaxed),
            events_built: events_built.load(Ordering::Relaxed),
            events_kept: events_kept.load(Ordering::Relaxed),
            files_written: files_written.load(Ordering::Relaxed),
            batches_published: batches_published.load(Ordering::Relaxed),
        };

        info!(
            hits = stats.received_hits,
            batches = stats.received_batches,
            chunks = stats.chunks_processed,
            events_built = stats.events_built,
            events_kept = stats.events_kept,
            files = stats.files_written,
            batches_published = stats.batches_published,
            "EventBuilderPipeline finished"
        );

        stats
    }
}

/// Sorter thread: read from HitSource → accumulate → sort → split → send chunks
#[allow(clippy::too_many_arguments)]
fn sorter_thread(
    mut source: impl HitSource,
    chunk_tx: crossbeam::Sender<super::chunk_builder::SortedChunk>,
    safe_horizon_ns: f64,
    threshold: usize,
    timeout: Duration,
    chunks_processed: Arc<AtomicU64>,
    received_hits: Arc<AtomicU64>,
    received_batches: Arc<AtomicU64>,
    time_calibration: TimeCalibration,
) {
    let mut buffer: Vec<super::hit::Hit> = Vec::with_capacity(threshold * 2);
    let mut last_flush = Instant::now();
    let poll_interval = Duration::from_millis(100);

    loop {
        // Poll the source
        match source.next_batch(poll_interval) {
            Ok(HitBatch::Hits(mut hits)) => {
                received_batches.fetch_add(1, Ordering::Relaxed);
                received_hits.fetch_add(hits.len() as u64, Ordering::Relaxed);

                // Apply time calibration
                for hit in &mut hits {
                    let offset = time_calibration.get_offset(hit.module, hit.channel);
                    hit.timestamp_ns -= offset;
                }
                buffer.extend(hits);
            }
            Ok(HitBatch::Eos) => {
                // End of stream: flush remaining and exit
                info!(buffer_size = buffer.len(), "Sorter received EOS, flushing");
                if let Some(chunk) = sort_and_flush(buffer) {
                    chunks_processed.fetch_add(1, Ordering::Relaxed);
                    let _ = chunk_tx.send(chunk);
                }
                break;
            }
            Err(SourceError::Timeout) => {
                // No data — continue to check if we should flush
            }
            Err(SourceError::Disconnected) => {
                info!(
                    buffer_size = buffer.len(),
                    "Sorter source disconnected, flushing"
                );
                if let Some(chunk) = sort_and_flush(buffer) {
                    chunks_processed.fetch_add(1, Ordering::Relaxed);
                    let _ = chunk_tx.send(chunk);
                }
                break;
            }
        }

        // Try to produce a chunk if we have enough data or timeout
        if buffer.len() >= threshold || (!buffer.is_empty() && last_flush.elapsed() >= timeout) {
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
                    last_flush = Instant::now();
                }
            }
        }
    }

    // chunk_tx is dropped here → workers detect channel close
    info!("Sorter thread finished");
}

/// Worker thread: receive chunks → build events → L2 filter → send to writers
/// (and optionally to the ZMQ PUB fan-out thread).
#[allow(clippy::too_many_arguments)]
fn worker_thread(
    chunk_rx: crossbeam::Receiver<super::chunk_builder::SortedChunk>,
    writer_tx: crossbeam::Sender<Vec<BuiltEvent>>,
    trigger_config: &TriggerConfig,
    events_built: Arc<AtomicU64>,
    events_kept: Arc<AtomicU64>,
    next_event_id: Arc<AtomicU64>,
    l2_filter: Option<Arc<L2Filter>>,
    pub_tx: Option<crossbeam::Sender<BuiltEventBatch>>,
    next_batch_id: Arc<AtomicU64>,
    run_id: u32,
) {
    while let Ok(chunk) = chunk_rx.recv() {
        let mut events = build_events_from_chunk(&chunk, trigger_config);
        let built_n = events.len() as u64;
        events_built.fetch_add(built_n, Ordering::Relaxed);

        // L2 filter (optional). Drops rejected events before they're sent
        // to writers; this is the post-build filtering step from SPEC § 7.
        if let Some(ref filter) = l2_filter {
            events.retain(|ev| filter.keeps(ev));
        }

        // Assign sequential event IDs AFTER L2 so rejected events don't burn
        // ID slots — keeps the on-disk EventID sequence dense.
        for event in &mut events {
            event.event_id = next_event_id.fetch_add(1, Ordering::Relaxed);
        }

        let kept_n = events.len() as u64;
        events_kept.fetch_add(kept_n, Ordering::Relaxed);

        if events.is_empty() {
            continue;
        }

        // Tee to ZMQ PUB before moving `events` into the writer channel.
        // Cloning the Vec is the price for fan-out; the PUB consumer can be
        // dropped (endpoint unconfigured) with zero overhead because
        // `pub_tx` is None in that case.
        if let Some(ref tx) = pub_tx {
            let batch_id = next_batch_id.fetch_add(1, Ordering::Relaxed);
            let batch = BuiltEventBatch {
                run_number: run_id,
                batch_id,
                events: events.clone(),
            };
            // Unbounded channel → only fails if the receiver is gone,
            // which means the PUB thread already exited (probably
            // shutting down). Drop silently.
            let _ = tx.send(batch);
        }

        if writer_tx.send(events).is_err() {
            error!("Failed to send events to writer (channel closed)");
            break;
        }
    }
}

/// ZMQ PUB thread: takes built-event batches from the workers and emits
/// each as one [`EbMessage::Events`] frame on a SUB-addressable endpoint.
///
/// On channel close (all workers dropped their senders) the thread emits a
/// final [`EbMessage::EndOfStream`] and exits. The PUB socket is set to
/// `SNDHWM = 0` so it buffers indefinitely rather than dropping under slow
/// subscribers (SPEC § 12 / CLAUDE.md zero-loss policy).
fn zmq_pub_thread(
    endpoint: String,
    rx: crossbeam::Receiver<BuiltEventBatch>,
    batches_published: Arc<AtomicU64>,
    run_id: u32,
) {
    let context = zmq::Context::new();
    let socket = match context.socket(zmq::PUB) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, endpoint = %endpoint, "PUB socket() failed");
            return;
        }
    };
    if let Err(e) = socket.set_sndhwm(0) {
        warn!(error = %e, "Failed to set PUB SNDHWM=0; continuing");
    }
    if let Err(e) = socket.bind(&endpoint) {
        error!(error = %e, endpoint = %endpoint, "PUB bind() failed");
        return;
    }
    info!(endpoint = %endpoint, run_id, "EB ZMQ PUB bound");

    while let Ok(batch) = rx.recv() {
        match EbMessage::Events(batch).to_msgpack() {
            Ok(bytes) => match socket.send(bytes, 0) {
                Ok(()) => {
                    batches_published.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    error!(error = %e, "PUB send failed; dropping batch");
                }
            },
            Err(e) => {
                error!(error = %e, "PUB msgpack encode failed; dropping batch");
            }
        }
    }

    // Channel closed → all workers done. Emit EOS so subscribers can
    // distinguish "no more data this run" from "still warming up".
    let eos = EbMessage::EndOfStream { run_number: run_id };
    match eos.to_msgpack() {
        Ok(bytes) => {
            if let Err(e) = socket.send(bytes, 0) {
                warn!(error = %e, "PUB send EOS failed");
            } else {
                info!(run_id, "EB ZMQ PUB emitted EOS");
            }
        }
        Err(e) => {
            error!(error = %e, "PUB msgpack encode of EOS failed");
        }
    }
}

#[cfg(test)]
mod zmq_pub_tests {
    use super::*;
    use crate::event_builder::built_event::EventHit;

    fn unique_ipc_endpoint() -> String {
        format!(
            "ipc:///tmp/delila-eb-pub-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    fn dummy_event(id: u64) -> BuiltEvent {
        BuiltEvent {
            event_id: id,
            trigger_time: id as f64 * 100.0,
            trigger_module: 1,
            trigger_channel: 2,
            hits: vec![EventHit {
                module: 1,
                channel: 2,
                energy: 1000,
                energy_short: 500,
                relative_time: 0.0,
                with_ac: false,
            }],
        }
    }

    /// End-to-end: spawn the PUB thread, send one batch, then close the
    /// channel; verify the subscriber sees Events followed by EOS.
    #[test]
    fn pub_thread_emits_events_then_eos() {
        let endpoint = unique_ipc_endpoint();
        let (tx, rx) = crossbeam::unbounded::<BuiltEventBatch>();
        let counter = Arc::new(AtomicU64::new(0));

        // Spawn subscriber FIRST so it's bound before the PUB starts;
        // for ipc transport the SUB connect can happen before PUB bind
        // and is queued, but we sleep a tick to dodge timing races.
        let sub_endpoint = endpoint.clone();
        let sub_handle = std::thread::spawn(move || {
            let context = zmq::Context::new();
            let socket = context.socket(zmq::SUB).unwrap();
            socket.set_rcvhwm(0).unwrap();
            socket.connect(&sub_endpoint).unwrap();
            socket.set_subscribe(b"").unwrap();
            socket.set_rcvtimeo(5_000).unwrap();

            let mut got_events = false;
            let mut got_eos = false;
            for _ in 0..10 {
                match socket.recv_bytes(0) {
                    Ok(b) => match EbMessage::from_msgpack(&b).unwrap() {
                        EbMessage::Events(batch) => {
                            assert_eq!(batch.run_number, 99);
                            assert_eq!(batch.events.len(), 2);
                            got_events = true;
                        }
                        EbMessage::EndOfStream { run_number } => {
                            assert_eq!(run_number, 99);
                            got_eos = true;
                            break;
                        }
                        EbMessage::Heartbeat { .. } => continue,
                    },
                    Err(_) => break, // timeout
                }
            }
            (got_events, got_eos)
        });

        // Tiny sleep so the SUB has a chance to connect before we publish.
        std::thread::sleep(Duration::from_millis(100));

        let counter_for_pub = counter.clone();
        let pub_handle = std::thread::spawn(move || {
            zmq_pub_thread(endpoint, rx, counter_for_pub, 99);
        });

        // Another tiny sleep so the PUB bind completes before send. ipc:
        // SUB queues sends pre-bind on the publisher side, but the
        // subscriber needs the bind socket up.
        std::thread::sleep(Duration::from_millis(200));

        tx.send(BuiltEventBatch {
            run_number: 99,
            batch_id: 1,
            events: vec![dummy_event(0), dummy_event(1)],
        })
        .unwrap();

        // Drop the sender → PUB thread will emit EOS and exit.
        drop(tx);
        pub_handle.join().unwrap();

        let (got_events, got_eos) = sub_handle.join().unwrap();
        assert!(got_events, "subscriber did not receive Events frame");
        assert!(got_eos, "subscriber did not receive EOS frame");
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
}

/// Writer thread: receive event batches → accumulate → write to ROOT files
#[cfg(feature = "root")]
fn writer_thread(
    writer_rx: crossbeam::Receiver<Vec<BuiltEvent>>,
    output_dir: PathBuf,
    events_per_file: usize,
    file_index: Arc<AtomicU32>,
    files_written: Arc<AtomicU64>,
    run_id: u32,
    tree_name: &str,
) {
    let mut buffer: Vec<BuiltEvent> = Vec::with_capacity(events_per_file + events_per_file / 10);

    while let Ok(batch) = writer_rx.recv() {
        buffer.extend(batch);

        if buffer.len() >= events_per_file {
            write_buffer_to_root(
                &mut buffer,
                &output_dir,
                run_id,
                tree_name,
                &file_index,
                &files_written,
            );
        }
    }

    // Write remaining events on channel close
    if !buffer.is_empty() {
        write_buffer_to_root(
            &mut buffer,
            &output_dir,
            run_id,
            tree_name,
            &file_index,
            &files_written,
        );
    }
}

#[cfg(feature = "root")]
fn write_buffer_to_root(
    buffer: &mut Vec<BuiltEvent>,
    output_dir: &std::path::Path,
    run_id: u32,
    tree_name: &str,
    file_index: &AtomicU32,
    files_written: &AtomicU64,
) {
    use super::root_io::write_events_to_root;

    buffer.sort_unstable_by(|a, b| a.trigger_time.total_cmp(&b.trigger_time));

    let idx = file_index.fetch_add(1, Ordering::Relaxed);
    let file_path = output_dir.join(format!("eb_run{:04}_{:04}_events.root", run_id, idx));

    match write_events_to_root(&file_path, tree_name, buffer) {
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
    buffer.clear();
}

/// Writer thread stub when ROOT feature is not enabled
#[cfg(not(feature = "root"))]
fn writer_thread(
    writer_rx: crossbeam::Receiver<Vec<BuiltEvent>>,
    _output_dir: PathBuf,
    _events_per_file: usize,
    _file_index: Arc<AtomicU32>,
    _files_written: Arc<AtomicU64>,
    _run_id: u32,
    _tree_name: &str,
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

#[cfg(test)]
mod tests {
    use super::super::chunk_builder::TriggerConfig;
    use super::super::source::{HitBatch, HitSource, SourceError};
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// Simple in-memory hit source for testing
    struct TestHitSource {
        batches: Vec<Vec<super::super::hit::Hit>>,
        index: usize,
    }

    impl HitSource for TestHitSource {
        fn next_batch(&mut self, _timeout: Duration) -> Result<HitBatch, SourceError> {
            if self.index < self.batches.len() {
                let batch = std::mem::take(&mut self.batches[self.index]);
                self.index += 1;
                Ok(HitBatch::Hits(batch))
            } else {
                Ok(HitBatch::Eos)
            }
        }

        fn name(&self) -> &str {
            "Test"
        }
    }

    fn make_hit(module: u8, channel: u8, ts: f64) -> super::super::hit::Hit {
        super::super::hit::Hit::new(module, channel, 1000, 500, ts)
    }

    #[test]
    fn test_pipeline_empty_source() {
        let source = TestHitSource {
            batches: vec![],
            index: 0,
        };

        let trigger_config = TriggerConfig {
            triggers: HashSet::from([(0, 0)]),
            priorities: HashMap::from([((0, 0), 0)]),
            ac_pairs: HashMap::new(),
            coincidence_window_ns: 500.0,
            multiplicity_triggers: Vec::new(),
        };

        let config = PipelineConfig {
            n_workers: 1,
            n_writers: 1,
            sorter_threshold: 100,
            sorter_timeout: Duration::from_millis(50),
            ..Default::default()
        };

        let pipeline =
            EventBuilderPipeline::new(config, trigger_config, TimeCalibration::new(0, 0));

        let stats = pipeline.run(source);
        assert_eq!(stats.received_hits, 0);
        assert_eq!(stats.events_built, 0);
    }

    #[test]
    fn test_pipeline_builds_events() {
        // Create hits: trigger (0,0) with coincident (0,1) and (1,0)
        let hits = vec![
            make_hit(0, 0, 1000.0),
            make_hit(0, 1, 1050.0),
            make_hit(1, 0, 1100.0),
            make_hit(0, 0, 5000.0),
            make_hit(0, 1, 5060.0),
        ];

        let source = TestHitSource {
            batches: vec![hits],
            index: 0,
        };

        let trigger_config = TriggerConfig {
            triggers: HashSet::from([(0, 0)]),
            priorities: HashMap::from([((0, 0), 0)]),
            ac_pairs: HashMap::new(),
            coincidence_window_ns: 500.0,
            multiplicity_triggers: Vec::new(),
        };

        let tmp_dir = tempfile::tempdir().unwrap();
        let config = PipelineConfig {
            n_workers: 1,
            n_writers: 1,
            sorter_threshold: 10,
            sorter_timeout: Duration::from_millis(50),
            output_dir: tmp_dir.path().to_path_buf(),
            events_per_file: 1000,
            ..Default::default()
        };

        let pipeline =
            EventBuilderPipeline::new(config, trigger_config, TimeCalibration::new(0, 0));

        let stats = pipeline.run(source);
        assert_eq!(stats.received_hits, 5);
        assert_eq!(stats.received_batches, 1);
        assert_eq!(stats.events_built, 2); // 2 triggers → 2 events
        assert!(stats.chunks_processed >= 1);
    }
}

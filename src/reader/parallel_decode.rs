//! Parallel decode pipeline: Dispatcher thread → N worker threads → Collector.
//!
//! ```text
//! ReadLoop (spawn_blocking)
//!     ↓ [tokio mpsc(1000) ReadLoopOutput]
//! Dispatcher (std::thread)
//!     ├─ classify (cheap header check)
//!     ├─ DIG1 only: sequential BTT rollover pre-scan (scan_extended_btts)
//!     ├─ AMax/x743: coalesce pre-decoded events into batches
//!     ↓ [crossbeam bounded(8) WorkItem]
//! Worker pool (N std::thread, own DecoderKind each)
//!     ├─ decode (+ pre-computed BTTs for DIG1) → convert → serialize
//!     ↓ [tokio mpsc(16) CollectorItem, blocking_send]
//! Collector (async, owns the ZMQ PUB socket — lives in decode_loop)
//!     └─ ReorderBuffer (BTreeMap) → publish in dispatch order
//! ```
//!
//! ## Ordering & sequence invariants
//!
//! * Every dispatched item (event batch, Start, Stop) gets a monotonically
//!   increasing `index`; the collector publishes strictly in index order, so
//!   the output stream is byte-identical in order to the old sequential loop
//!   (Stop's EOS is emitted only after every prior batch — by construction).
//! * `sequence_number` is assigned by the dispatcher (reset to 0 on Start)
//!   and embedded by the worker during serialization. To keep the sequence
//!   gap-free, batches that decode to zero events (or are fully removed by
//!   the `adc_min` filter) are still published as empty batches instead of
//!   being dropped. A worker panic or serialization failure produces a
//!   `Skip` so the reorder buffer can advance; the resulting sequence gap is
//!   visible downstream (Merger gap stats) rather than wedging the pipeline.
//! * DIG1 rollover state (`RolloverTracker`) lives exclusively in the
//!   dispatcher's scan decoder; workers receive pre-extended BTTs and are
//!   stateless across batches (their decoder state is diagnostics only).
//!
//! ## Shutdown
//!
//! Cascade: ReadLoop drops its sender → dispatcher's `blocking_recv` returns
//! `None` → dispatcher drops the crossbeam sender → workers drain and exit →
//! all collector senders drop → collector `recv` returns `None`. No data is
//! dropped on the normal Stop path; the in-flight backlog is fully decoded
//! and published before the channels close.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::decoder::{self, DataType};
use super::{
    caen, convert_event_to_common, opendpp_to_event_data, DecoderKind, ReadLoopOutput,
    ReaderConfig, ReaderMetrics, MAX_CHANNELS,
};
use crate::common::{EventDataBatch, Message};

/// Dispatcher → worker queue depth (per design doc: small, memory pressure).
const WORK_QUEUE_DEPTH: usize = 8;
/// Worker → collector queue depth.
const COLLECTOR_QUEUE_DEPTH: usize = 16;
/// Max pre-decoded events (AMax/x743) coalesced into one published batch.
/// 256 × ~8 KB waveform events ≈ 2 MB serialized — large enough to amortize
/// per-batch overhead, small enough for prompt Monitor updates.
const COALESCE_MAX: usize = 256;

/// Items flowing from dispatcher/workers to the collector. All variants
/// carry the dispatch `index` used by [`ReorderBuffer`].
#[derive(Debug)]
pub(crate) enum CollectorItem {
    /// Serialized event batch ready for ZMQ publish.
    Batch {
        index: u64,
        bytes: Vec<u8>,
        n_events: usize,
        seq: u64,
    },
    /// Index consumed but nothing to publish (worker panic / serialize
    /// failure — already logged at the source).
    Skip { index: u64 },
    /// Start marker (digitizer stream or ReadLoop) — collector resets
    /// heartbeat counter; sequence reset already happened in the dispatcher.
    Start { index: u64 },
    /// Stop marker — collector publishes EOS once all prior indices flushed.
    Stop { index: u64 },
}

impl CollectorItem {
    fn index(&self) -> u64 {
        match self {
            Self::Batch { index, .. }
            | Self::Skip { index }
            | Self::Start { index }
            | Self::Stop { index } => *index,
        }
    }
}

/// Work unit handed to the worker pool.
struct WorkItem {
    index: u64,
    seq: u64,
    /// Run epoch — bumped on Start so workers reset per-run decoder
    /// diagnostics (warn-once flags etc.) without a broadcast.
    epoch: u64,
    payload: WorkPayload,
}

enum WorkPayload {
    /// Raw aggregate needing decode (PSD1/PSD2/PHA1/PHA2).
    /// `btts`: pre-extended board time tags (DIG1 only, one per aggregate).
    Raw {
        raw: decoder::RawData,
        btts: Option<Vec<u64>>,
    },
    /// Pre-decoded events (x743) — convert + serialize only.
    Events(Vec<decoder::EventData>),
    /// Untranslated OpenDPP events (AMax) — 4-lane unpack + convert +
    /// serialize. All events in a batch share one `enable_acq` snapshot
    /// (the dispatcher flushes early when the flag changes mid-stream).
    OpenDpp {
        events: Vec<caen::OpenDppEvent>,
        enable_acq: bool,
    },
}

/// Reorders out-of-order collector items back into dispatch order.
pub(crate) struct ReorderBuffer {
    next: u64,
    pending: std::collections::BTreeMap<u64, CollectorItem>,
}

impl ReorderBuffer {
    pub(crate) fn new() -> Self {
        Self {
            next: 0,
            pending: std::collections::BTreeMap::new(),
        }
    }

    /// Insert an item; returns every item that is now ready in order.
    pub(crate) fn push(&mut self, item: CollectorItem) -> Vec<CollectorItem> {
        self.pending.insert(item.index(), item);
        let mut ready = Vec::new();
        while let Some(item) = self.pending.remove(&self.next) {
            self.next += 1;
            ready.push(item);
        }
        ready
    }

    /// Number of items waiting for a missing predecessor.
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Drain whatever is left in index order (end-of-stream best effort).
    pub(crate) fn drain_remaining(&mut self) -> Vec<CollectorItem> {
        let remaining: Vec<CollectorItem> =
            std::mem::take(&mut self.pending).into_values().collect();
        if !remaining.is_empty() {
            warn!(
                count = remaining.len(),
                next_expected = self.next,
                "ReorderBuffer drained with missing indices (worker died?)"
            );
        }
        remaining
    }
}

/// Resolve the worker count: explicit config value, or `0` = auto
/// (half the logical CPUs minus one, clamped to [1, 8] — leaves room for
/// the FELib receive thread, ReadLoop, collector and sibling components).
pub(crate) fn resolve_worker_count(configured: usize) -> usize {
    if configured > 0 {
        return configured.min(64);
    }
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (logical / 2).saturating_sub(1).clamp(1, 8)
}

/// Spawn dispatcher + worker threads. Returns the channel the collector
/// (decode_loop) consumes. Threads exit via channel-closure cascade.
pub(crate) fn spawn_pipeline(
    config: &ReaderConfig,
    rx: mpsc::Receiver<ReadLoopOutput>,
    metrics: Arc<ReaderMetrics>,
) -> mpsc::Receiver<CollectorItem> {
    let workers = resolve_worker_count(config.decode_workers);
    let (work_tx, work_rx) = crossbeam_channel::bounded::<WorkItem>(WORK_QUEUE_DEPTH);
    let (collector_tx, collector_rx) = mpsc::channel::<CollectorItem>(COLLECTOR_QUEUE_DEPTH);

    info!(
        workers,
        firmware = ?config.firmware,
        "Parallel decode pipeline starting"
    );

    for worker_id in 0..workers {
        let work_rx = work_rx.clone();
        let collector_tx = collector_tx.clone();
        let config = config.clone();
        let metrics = Arc::clone(&metrics);
        std::thread::Builder::new()
            .name(format!("decode-worker-{worker_id}"))
            .spawn(move || worker_loop(worker_id, &config, work_rx, collector_tx, metrics))
            .unwrap_or_else(|e| panic!("failed to spawn decode-worker-{worker_id}: {e}"));
    }

    let config = config.clone();
    std::thread::Builder::new()
        .name("decode-dispatcher".to_string())
        .spawn(move || dispatcher_loop(&config, rx, work_tx, collector_tx, metrics))
        .unwrap_or_else(|e| panic!("failed to spawn decode-dispatcher: {e}"));

    collector_rx
}

fn dispatcher_loop(
    config: &ReaderConfig,
    mut rx: mpsc::Receiver<ReadLoopOutput>,
    work_tx: crossbeam_channel::Sender<WorkItem>,
    collector_tx: mpsc::Sender<CollectorItem>,
    metrics: Arc<ReaderMetrics>,
) {
    // Owns the only stateful (rollover-tracking) decoder instance. Used for
    // classify + DIG1 BTT pre-scan; never decodes events itself.
    let mut scan_decoder = DecoderKind::for_config(config);
    let is_dig1 = config.firmware.is_dig1();

    let mut next_index: u64 = 0;
    let mut seq: u64 = 0;
    let mut epoch: u64 = 0;
    let mut btt_buf: Vec<u64> = Vec::new();
    // An item pulled during coalescing that wasn't a Decoded event.
    let mut carry: Option<ReadLoopOutput> = None;

    macro_rules! dispatch_work {
        ($payload:expr) => {{
            let item = WorkItem {
                index: next_index,
                seq,
                epoch,
                payload: $payload,
            };
            next_index += 1;
            seq += 1;
            if work_tx.send(item).is_err() {
                error!("decode workers gone — dispatcher exiting");
                return;
            }
        }};
    }

    macro_rules! send_control {
        ($variant:ident) => {{
            let item = CollectorItem::$variant { index: next_index };
            next_index += 1;
            if collector_tx.blocking_send(item).is_err() {
                info!("collector gone — dispatcher exiting");
                return;
            }
        }};
    }

    loop {
        let output = match carry.take() {
            Some(o) => o,
            None => match rx.blocking_recv() {
                Some(o) => o,
                None => break,
            },
        };

        match output {
            ReadLoopOutput::Raw(raw) => {
                metrics.queue_length.fetch_sub(1, Ordering::Relaxed);
                match scan_decoder.classify(&raw) {
                    DataType::Event => {
                        let btts = if is_dig1 {
                            scan_decoder.scan_extended_btts(&raw, &mut btt_buf);
                            Some(btt_buf.clone())
                        } else {
                            None
                        };
                        dispatch_work!(WorkPayload::Raw { raw, btts });
                    }
                    DataType::Start => {
                        info!("Received START signal from digitizer");
                        seq = 0;
                        epoch += 1;
                        scan_decoder.reset_for_new_run();
                        send_control!(Start);
                    }
                    DataType::Stop => {
                        info!("Received STOP signal from digitizer");
                        send_control!(Stop);
                    }
                    DataType::Unknown => {
                        warn!("Received unknown data type");
                    }
                }
            }

            ReadLoopOutput::Decoded(event) => {
                metrics.queue_length.fetch_sub(1, Ordering::Relaxed);
                let mut events = Vec::with_capacity(COALESCE_MAX.min(64));
                events.push(*event);
                // Coalesce whatever is already queued (without blocking) so
                // high-rate sources publish ~COALESCE_MAX-event batches
                // instead of one ZMQ message per event.
                while events.len() < COALESCE_MAX {
                    match rx.try_recv() {
                        Ok(ReadLoopOutput::Decoded(e)) => {
                            metrics.queue_length.fetch_sub(1, Ordering::Relaxed);
                            events.push(*e);
                        }
                        Ok(other) => {
                            carry = Some(other);
                            break;
                        }
                        Err(_) => break, // empty or disconnected — flush now
                    }
                }
                dispatch_work!(WorkPayload::Events(events));
            }

            ReadLoopOutput::OpenDpp { event, enable_acq } => {
                metrics.queue_length.fetch_sub(1, Ordering::Relaxed);
                let mut events = Vec::with_capacity(COALESCE_MAX.min(64));
                events.push(*event);
                // Same coalescing as Decoded, but a batch must hold a single
                // enable_acq snapshot — flush early if the flag flips
                // (Tune Up ENABLE_ACQ hot-swap mid-stream).
                while events.len() < COALESCE_MAX {
                    match rx.try_recv() {
                        Ok(ReadLoopOutput::OpenDpp {
                            event: e,
                            enable_acq: ea,
                        }) if ea == enable_acq => {
                            metrics.queue_length.fetch_sub(1, Ordering::Relaxed);
                            events.push(*e);
                        }
                        Ok(other) => {
                            carry = Some(other);
                            break;
                        }
                        Err(_) => break, // empty or disconnected — flush now
                    }
                }
                dispatch_work!(WorkPayload::OpenDpp { events, enable_acq });
            }

            ReadLoopOutput::Start => {
                info!("Received START signal from ReadLoop");
                seq = 0;
                epoch += 1;
                scan_decoder.reset_for_new_run();
                send_control!(Start);
            }

            ReadLoopOutput::Stop => {
                info!("Received STOP signal from ReadLoop");
                send_control!(Stop);
            }
        }
    }

    info!(
        dispatched = next_index,
        "Data channel closed, decode dispatcher exiting"
    );
    // work_tx and collector_tx drop here → workers drain → collector ends.
}

fn worker_loop(
    worker_id: usize,
    config: &ReaderConfig,
    work_rx: crossbeam_channel::Receiver<WorkItem>,
    collector_tx: mpsc::Sender<CollectorItem>,
    metrics: Arc<ReaderMetrics>,
) {
    let mut decoder = DecoderKind::for_config(config);
    let mut last_epoch: u64 = 0;
    let mut events_buf: Vec<decoder::EventData> = Vec::with_capacity(1024);

    while let Ok(item) = work_rx.recv() {
        if item.epoch != last_epoch {
            decoder.reset_for_new_run();
            last_epoch = item.epoch;
        }
        let index = item.index;

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_item(item, config, &mut decoder, &mut events_buf, &metrics)
        }));

        let msg = match result {
            Ok(Some((bytes, n_events, seq))) => CollectorItem::Batch {
                index,
                bytes,
                n_events,
                seq,
            },
            Ok(None) => CollectorItem::Skip { index },
            Err(_) => {
                error!(
                    worker_id,
                    index, "decode worker panicked on batch — skipping"
                );
                // Buffer state is unknown after a panic; replace it.
                events_buf = Vec::with_capacity(1024);
                CollectorItem::Skip { index }
            }
        };

        if collector_tx.blocking_send(msg).is_err() {
            break; // collector gone — shutdown
        }
    }
}

/// Decode/convert/serialize one work item. Returns `None` on serialization
/// failure (logged); zero-event batches still serialize (sequence stays
/// gap-free).
fn process_item(
    item: WorkItem,
    config: &ReaderConfig,
    decoder: &mut DecoderKind,
    events_buf: &mut Vec<decoder::EventData>,
    metrics: &ReaderMetrics,
) -> Option<(Vec<u8>, usize, u64)> {
    let seq = item.seq;
    let is_dig1 = config.firmware.is_dig1();

    // The `adc_min` energy floor (config, default 0 = off) applies to every
    // decoded event regardless of how it arrived — raw-decoded (DIG1),
    // pre-decoded (V1743 / X743Std), or OpenDpp (AMax). It is gated purely on
    // `config.adc_min > 0` in the drain loop below.
    let events: &mut Vec<decoder::EventData> = match item.payload {
        WorkPayload::Raw { raw, btts } => {
            decoder.decode_into_with_btts(&raw, btts.as_deref(), events_buf);
            if events_buf.is_empty() {
                warn!(
                    raw_size = raw.size,
                    raw_n_events = raw.n_events,
                    "Decoded 0 events from raw data"
                );
            }
            events_buf
        }
        WorkPayload::Events(events) => {
            *events_buf = events;
            events_buf
        }
        WorkPayload::OpenDpp { events, enable_acq } => {
            // 4-lane debug unpack + EventData conversion — the CPU-heavy
            // part deliberately moved off the ReadLoop thread.
            events_buf.clear();
            events_buf.extend(
                events
                    .iter()
                    .map(|e| opendpp_to_event_data(e, config.module_id, enable_acq)),
            );
            events_buf
        }
    };

    let n_events = events.len();
    let mut batch = EventDataBatch::with_capacity(config.source_id, seq, n_events);

    for event in events.drain(..) {
        let common_event = convert_event_to_common(event, config.firmware);
        if is_dig1 {
            if common_event.has_trigger_lost() {
                metrics
                    .trigger_lost_flag_events
                    .fetch_add(1, Ordering::Relaxed);
            }
            if (common_event.flags & crate::common::flags::FLAG_N_LOST_TRIGGER) != 0 {
                metrics
                    .n_lost_trigger_flag_events
                    .fetch_add(1, Ordering::Relaxed);
                // Each N_LOST flag ≈ 1024 lost triggers
                metrics
                    .trigger_loss_count
                    .fetch_add(1024, Ordering::Relaxed);
            }
        }
        if config.adc_min > 0 && common_event.energy < config.adc_min {
            metrics.filtered_events.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let ch = common_event.channel as usize;
        if ch < MAX_CHANNELS {
            metrics.per_channel_counts[ch].fetch_add(1, Ordering::Relaxed);
        }
        batch.push(common_event);
    }

    metrics
        .events_decoded
        .fetch_add(n_events as u64, Ordering::Relaxed);

    let kept = batch.len();
    let msg = Message::data(batch);
    match msg.to_msgpack() {
        Ok(bytes) => Some((bytes, kept, seq)),
        Err(e) => {
            error!(error = %e, events = kept, "Failed to serialize event batch");
            None
        }
    }
}

/// Collector-side state for the rate-limited DIG1 trigger-loss warning.
pub(crate) struct TriggerLossWarner {
    last_warn: std::time::Instant,
    last_logged: u64,
}

impl TriggerLossWarner {
    pub(crate) fn new() -> Self {
        Self {
            last_warn: std::time::Instant::now(),
            last_logged: 0,
        }
    }

    fn check(&mut self, metrics: &ReaderMetrics) {
        let lost = metrics.trigger_loss_count.load(Ordering::Relaxed);
        if lost > self.last_logged && self.last_warn.elapsed() >= std::time::Duration::from_secs(10)
        {
            let flag_events = metrics.trigger_lost_flag_events.load(Ordering::Relaxed);
            let n_lost_events = metrics.n_lost_trigger_flag_events.load(Ordering::Relaxed);
            warn!(
                estimated_lost = lost,
                trigger_lost_flags = flag_events,
                n_lost_flags = n_lost_events,
                "Trigger loss detected (DIG1 EXTRAS flags)"
            );
            self.last_warn = std::time::Instant::now();
            self.last_logged = lost;
        }
    }
}

/// Publish one in-order collector item on the ZMQ socket. Returns the number
/// of batches published (0 or 1) so the caller can track totals.
pub(crate) async fn publish_ready(
    item: CollectorItem,
    config: &ReaderConfig,
    data_socket: &mut tmq::publish::Publish,
    metrics: &ReaderMetrics,
    heartbeat_counter: &mut u64,
    trigger_warner: &mut TriggerLossWarner,
) -> u64 {
    use futures::SinkExt;
    match item {
        CollectorItem::Batch {
            bytes,
            n_events,
            seq,
            ..
        } => {
            let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
            let published = if let Err(e) = data_socket.send(zmq_msg).await {
                error!(error = %e, events = n_events, "Failed to send event batch via ZMQ");
                0
            } else {
                metrics.batches_published.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(events = n_events, seq, "Decoded and published batch");
                1
            };
            if config.firmware.is_dig1() {
                trigger_warner.check(metrics);
            }
            published
        }
        CollectorItem::Skip { .. } => 0,
        CollectorItem::Start { .. } => {
            *heartbeat_counter = 0;
            info!("Sequence number and decoder state reset on Start");
            0
        }
        CollectorItem::Stop { .. } => {
            // Carry the current run number so the Recorder's stale-EOS filter
            // matches (TODO 58 C1 — hardcoded 0 meant every real EOS of
            // run_number >= 1 was discarded as stale).
            let run_number = metrics.current_run.load(Ordering::Relaxed);
            let eos = Message::eos(config.source_id, run_number);
            match eos.to_msgpack() {
                Ok(bytes) => {
                    let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                    if let Err(e) = data_socket.send(zmq_msg).await {
                        error!(error = %e, "Failed to send EOS via ZMQ");
                    } else {
                        info!(source_id = config.source_id, run_number, "Published EOS");
                    }
                }
                Err(e) => error!(error = %e, "Failed to serialize EOS"),
            }
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(index: u64) -> CollectorItem {
        CollectorItem::Batch {
            index,
            bytes: vec![index as u8],
            n_events: 1,
            seq: index,
        }
    }

    fn indices(items: &[CollectorItem]) -> Vec<u64> {
        items.iter().map(|i| i.index()).collect()
    }

    #[test]
    fn reorder_in_order_passthrough() {
        let mut rb = ReorderBuffer::new();
        assert_eq!(indices(&rb.push(batch(0))), vec![0]);
        assert_eq!(indices(&rb.push(batch(1))), vec![1]);
        assert_eq!(rb.pending_len(), 0);
    }

    #[test]
    fn reorder_out_of_order_release() {
        let mut rb = ReorderBuffer::new();
        assert!(rb.push(batch(2)).is_empty());
        assert!(rb.push(batch(1)).is_empty());
        assert_eq!(rb.pending_len(), 2);
        assert_eq!(indices(&rb.push(batch(0))), vec![0, 1, 2]);
        assert_eq!(rb.pending_len(), 0);
    }

    #[test]
    fn reorder_control_tokens_interleave() {
        let mut rb = ReorderBuffer::new();
        assert!(rb.push(CollectorItem::Stop { index: 3 }).is_empty());
        assert!(rb.push(batch(1)).is_empty());
        assert!(rb.push(CollectorItem::Skip { index: 2 }).is_empty());
        let ready = rb.push(CollectorItem::Start { index: 0 });
        assert_eq!(indices(&ready), vec![0, 1, 2, 3]);
        assert!(matches!(ready[0], CollectorItem::Start { .. }));
        assert!(matches!(ready[2], CollectorItem::Skip { .. }));
        assert!(matches!(ready[3], CollectorItem::Stop { .. }));
    }

    #[test]
    fn reorder_drain_remaining_skips_gap() {
        let mut rb = ReorderBuffer::new();
        assert_eq!(indices(&rb.push(batch(0))), vec![0]);
        assert!(rb.push(batch(2)).is_empty()); // index 1 missing forever
        let drained = rb.drain_remaining();
        assert_eq!(indices(&drained), vec![2]);
        assert_eq!(rb.pending_len(), 0);
    }

    #[test]
    fn resolve_worker_count_explicit_and_auto() {
        assert_eq!(resolve_worker_count(6), 6);
        assert_eq!(resolve_worker_count(100), 64); // clamped
        let auto = resolve_worker_count(0);
        assert!((1..=8).contains(&auto));
    }

    // -----------------------------------------------------------------------
    // End-to-end pipeline (dispatcher → workers → collector channel)
    // -----------------------------------------------------------------------

    use crate::config::digitizer::FirmwareType;

    fn test_event(i: u64) -> decoder::EventData {
        decoder::EventData {
            timestamp_ns: i as f64,
            energy: (i % 1000) as u16,
            ..Default::default()
        }
    }

    fn amax_config(workers: usize) -> ReaderConfig {
        ReaderConfig {
            firmware: FirmwareType::AMax,
            decode_workers: workers,
            ..Default::default()
        }
    }

    /// Drain the collector channel through a ReorderBuffer, deserializing
    /// each batch. Returns (per-batch (seq, event_count) list, stop_count).
    async fn drain_pipeline(
        mut collector_rx: mpsc::Receiver<CollectorItem>,
    ) -> (Vec<(u64, usize)>, usize) {
        let mut reorder = ReorderBuffer::new();
        let mut batches = Vec::new();
        let mut stops = 0;
        let mut drained: Vec<CollectorItem> = Vec::new();
        while let Some(item) = collector_rx.recv().await {
            drained.extend(reorder.push(item));
        }
        drained.extend(reorder.drain_remaining());
        for item in drained {
            match item {
                CollectorItem::Batch {
                    bytes,
                    n_events,
                    seq,
                    ..
                } => {
                    // The serialized form must round-trip and carry the
                    // dispatcher-assigned sequence number.
                    match Message::from_msgpack(&bytes).expect("valid msgpack") {
                        Message::Data(b) => {
                            assert_eq!(b.sequence_number, seq);
                            assert_eq!(b.len(), n_events);
                        }
                        other => panic!("expected Data message, got {other:?}"),
                    }
                    batches.push((seq, n_events));
                }
                CollectorItem::Stop { .. } => stops += 1,
                CollectorItem::Start { .. } | CollectorItem::Skip { .. } => {}
            }
        }
        (batches, stops)
    }

    #[tokio::test]
    async fn pipeline_preserves_event_count_order_and_stop() {
        let metrics = Arc::new(ReaderMetrics::default());
        let (tx, rx) = mpsc::channel::<ReadLoopOutput>(1000);
        let collector_rx = spawn_pipeline(&amax_config(4), rx, Arc::clone(&metrics));

        const N: u64 = 500;
        for i in 0..N {
            tx.send(ReadLoopOutput::Decoded(Box::new(test_event(i))))
                .await
                .expect("send");
        }
        tx.send(ReadLoopOutput::Stop).await.expect("send stop");
        drop(tx); // closes the cascade

        let (batches, stops) = drain_pipeline(collector_rx).await;
        assert_eq!(stops, 1);
        let total: usize = batches.iter().map(|(_, n)| n).sum();
        assert_eq!(total as u64, N, "no event lost or duplicated");
        // Sequence numbers must be contiguous from 0 in publish order.
        for (expect, (seq, _)) in batches.iter().enumerate() {
            assert_eq!(*seq, expect as u64);
        }
        assert_eq!(metrics.events_decoded.load(Ordering::Relaxed), N);
    }

    fn opendpp_event(i: u64, waveform: Option<Vec<u16>>) -> caen::OpenDppEvent {
        caen::OpenDppEvent {
            channel: 0,
            timestamp: i,
            fine_timestamp: 0,
            energy: (i % 1000) as u16,
            flags_b: 0,
            flags_a: 0,
            psd: 0,
            user_info: Vec::new(),
            waveform,
            event_size: 16,
        }
    }

    /// OpenDpp path: untranslated events are unpacked/converted on workers,
    /// counts and sequence order preserved, EOS after all batches.
    #[tokio::test]
    async fn pipeline_opendpp_preserves_count_and_order() {
        let metrics = Arc::new(ReaderMetrics::default());
        let (tx, rx) = mpsc::channel::<ReadLoopOutput>(1000);
        let collector_rx = spawn_pipeline(&amax_config(4), rx, Arc::clone(&metrics));

        const N: u64 = 300;
        for i in 0..N {
            tx.send(ReadLoopOutput::OpenDpp {
                event: Box::new(opendpp_event(i, Some(vec![1, 2, 3, 4]))),
                enable_acq: false,
            })
            .await
            .expect("send");
        }
        tx.send(ReadLoopOutput::Stop).await.expect("send stop");
        drop(tx);

        let (batches, stops) = drain_pipeline(collector_rx).await;
        assert_eq!(stops, 1);
        let total: usize = batches.iter().map(|(_, n)| n).sum();
        assert_eq!(total as u64, N);
        for (expect, (seq, _)) in batches.iter().enumerate() {
            assert_eq!(*seq, expect as u64);
        }
        assert_eq!(metrics.events_decoded.load(Ordering::Relaxed), N);
    }

    /// enable_acq=true → the worker performs the 4-lane debug unpack
    /// (lane 0/1/2 → analog_probe1/2/3 as signed 16-bit).
    #[tokio::test]
    async fn pipeline_opendpp_unpacks_four_lanes_on_worker() {
        let metrics = Arc::new(ReaderMetrics::default());
        let (tx, rx) = mpsc::channel::<ReadLoopOutput>(1000);
        let mut collector_rx = spawn_pipeline(&amax_config(2), rx, Arc::clone(&metrics));

        tx.send(ReadLoopOutput::OpenDpp {
            event: Box::new(opendpp_event(7, Some(vec![1000, 2000, 3000, 0xA800]))),
            enable_acq: true,
        })
        .await
        .expect("send");
        drop(tx);

        let mut wf_checked = false;
        while let Some(item) = collector_rx.recv().await {
            if let CollectorItem::Batch { bytes, .. } = item {
                let Message::Data(batch) = Message::from_msgpack(&bytes).expect("valid msgpack")
                else {
                    panic!("expected Data");
                };
                let wf = batch.events[0].waveform.as_ref().expect("waveform present");
                assert_eq!(wf.analog_probe1, vec![1000i16]);
                assert_eq!(wf.analog_probe2, vec![2000i16]);
                assert_eq!(wf.analog_probe3, vec![3000i16]);
                // 0xA800 → digital bits 15 (trig_out), 13, 11 set
                assert_eq!(wf.digital_probe1, vec![1u8]);
                wf_checked = true;
            }
        }
        assert!(wf_checked, "no batch reached the collector");
    }

    /// An enable_acq flip mid-stream must not lose events (the dispatcher
    /// flushes the coalesce buffer at the boundary).
    #[tokio::test]
    async fn pipeline_opendpp_enable_acq_flip_no_loss() {
        let metrics = Arc::new(ReaderMetrics::default());
        let (tx, rx) = mpsc::channel::<ReadLoopOutput>(1000);
        let collector_rx = spawn_pipeline(&amax_config(2), rx, Arc::clone(&metrics));

        for i in 0..60u64 {
            tx.send(ReadLoopOutput::OpenDpp {
                event: Box::new(opendpp_event(i, Some(vec![1, 2, 3, 4]))),
                enable_acq: i >= 30, // flips halfway
            })
            .await
            .expect("send");
        }
        tx.send(ReadLoopOutput::Stop).await.expect("send stop");
        drop(tx);

        let (batches, stops) = drain_pipeline(collector_rx).await;
        assert_eq!(stops, 1);
        let total: usize = batches.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 60, "no event lost across the flag flip");
    }

    #[tokio::test]
    async fn pipeline_start_resets_sequence() {
        let metrics = Arc::new(ReaderMetrics::default());
        let (tx, rx) = mpsc::channel::<ReadLoopOutput>(1000);
        let collector_rx = spawn_pipeline(&amax_config(2), rx, Arc::clone(&metrics));

        for i in 0..10u64 {
            tx.send(ReadLoopOutput::Decoded(Box::new(test_event(i))))
                .await
                .expect("send");
        }
        // Give the dispatcher time to flush run 1's events so they don't
        // coalesce across the Start marker.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        tx.send(ReadLoopOutput::Start).await.expect("send start");
        for i in 0..10u64 {
            tx.send(ReadLoopOutput::Decoded(Box::new(test_event(i))))
                .await
                .expect("send");
        }
        tx.send(ReadLoopOutput::Stop).await.expect("send stop");
        drop(tx);

        let (batches, stops) = drain_pipeline(collector_rx).await;
        assert_eq!(stops, 1);
        // Sequence restarts at 0 after Start: the seq values across all
        // batches must contain two runs each starting at 0.
        let zero_starts = batches.iter().filter(|(seq, _)| *seq == 0).count();
        assert_eq!(zero_starts, 2, "sequence must reset on Start: {batches:?}");
        let total: usize = batches.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 20);
    }
}

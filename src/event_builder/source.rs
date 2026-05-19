//! Hit data sources for the event builder pipeline.
//!
//! `HitSource` trait abstracts input sources so that the same pipeline
//! can be used for both online (ZMQ) and offline (.delila / ROOT) processing.

use super::hit::Hit;
use std::path::PathBuf;
use std::time::Duration;

/// A batch of hits or end-of-stream marker.
#[derive(Debug)]
pub enum HitBatch {
    /// A batch of hits to process.
    Hits(Vec<Hit>),
    /// End of stream — no more data.
    Eos,
}

/// Error type for HitSource::next_batch timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// No data available within the timeout period.
    Timeout,
    /// The source has been disconnected / closed.
    Disconnected,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Timeout => write!(f, "source timeout"),
            SourceError::Disconnected => write!(f, "source disconnected"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Trait for providing hit data to the event builder pipeline.
///
/// Implementations provide batches of hits from various sources:
/// - `DelilaFileHitSource`: reads .delila binary files (offline)
/// - `RootFileHitSource`: reads ELIFANT ROOT files (offline, feature="root")
/// - `ZmqHitSource`: receives hits via ZMQ (online)
pub trait HitSource: Send {
    /// Get the next batch of hits, waiting up to `timeout`.
    ///
    /// Returns:
    /// - `Ok(HitBatch::Hits(..))` — a batch of hits
    /// - `Ok(HitBatch::Eos)` — end of stream (all data consumed)
    /// - `Err(SourceError::Timeout)` — no data within timeout
    /// - `Err(SourceError::Disconnected)` — source closed unexpectedly
    fn next_batch(&mut self, timeout: Duration) -> Result<HitBatch, SourceError>;

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}

/// Hit source that reads .delila binary files directly.
///
/// Uses the existing `DataFileReader` to stream batches of EventData,
/// converting each to `Hit` via `Hit::from_event_data()`.
/// Files are processed sequentially, one at a time.
pub struct DelilaFileHitSource {
    files: Vec<PathBuf>,
    current_file_index: usize,
    current_reader: Option<DelilaBlockIterator>,
    finished: bool,
}

/// Internal wrapper around DataFileReader's block iterator.
struct DelilaBlockIterator {
    // We store the collected batches from a single file since DataFileReader
    // returns a borrowing iterator. We read all blocks from the current file
    // into memory (they're already in batches, so this is efficient).
    batches: Vec<Vec<Hit>>,
    batch_index: usize,
}

impl DelilaFileHitSource {
    /// Create a new source from a list of .delila files.
    pub fn new(files: Vec<PathBuf>) -> Self {
        Self {
            files,
            current_file_index: 0,
            current_reader: None,
            finished: false,
        }
    }

    /// Open the next file and load its batches.
    fn open_next_file(&mut self) -> Result<(), SourceError> {
        use crate::recorder::DataFileReader;
        use std::io::BufReader;

        if self.current_file_index >= self.files.len() {
            self.finished = true;
            return Ok(());
        }

        let path = &self.files[self.current_file_index];
        let file = std::fs::File::open(path).map_err(|_| SourceError::Disconnected)?;
        let buf_reader = BufReader::new(file);
        let mut reader = DataFileReader::new(buf_reader).map_err(|_| SourceError::Disconnected)?;

        // Collect batches: convert EventDataBatch -> Vec<Hit>
        let mut batches = Vec::new();
        for block_result in reader.data_blocks() {
            match block_result {
                Ok(event_batch) => {
                    let hits: Vec<Hit> = event_batch
                        .events
                        .iter()
                        .map(Hit::from_event_data)
                        .collect();
                    if !hits.is_empty() {
                        batches.push(hits);
                    }
                }
                Err(_) => {
                    // Skip corrupted blocks, continue reading
                    continue;
                }
            }
        }

        self.current_reader = Some(DelilaBlockIterator {
            batches,
            batch_index: 0,
        });
        self.current_file_index += 1;

        Ok(())
    }
}

impl HitSource for DelilaFileHitSource {
    fn next_batch(&mut self, _timeout: Duration) -> Result<HitBatch, SourceError> {
        if self.finished {
            return Ok(HitBatch::Eos);
        }

        loop {
            // Try to get next batch from current file
            if let Some(ref mut iter) = self.current_reader {
                if iter.batch_index < iter.batches.len() {
                    let batch = std::mem::take(&mut iter.batches[iter.batch_index]);
                    iter.batch_index += 1;
                    return Ok(HitBatch::Hits(batch));
                }
                // Current file exhausted
                self.current_reader = None;
            }

            // Open next file
            self.open_next_file()?;
            if self.finished {
                return Ok(HitBatch::Eos);
            }
        }
    }

    fn name(&self) -> &str {
        "Delila"
    }
}

/// Hit source that reads ELIFANT ROOT files via oxyroot.
///
/// Loads one file at a time, yielding hits in configurable batch sizes.
#[cfg(feature = "root")]
pub struct RootFileHitSource {
    files: Vec<PathBuf>,
    current_file_index: usize,
    tree_name: String,
    batch_size: usize,
    /// Buffered hits from the current file
    current_hits: Vec<Hit>,
    /// Current position within current_hits
    current_offset: usize,
    finished: bool,
}

#[cfg(feature = "root")]
impl RootFileHitSource {
    /// Create a new ROOT file hit source.
    ///
    /// `batch_size` controls how many hits are yielded per batch (default: 500_000).
    pub fn new(files: Vec<PathBuf>, tree_name: &str, batch_size: usize) -> Self {
        Self {
            files,
            current_file_index: 0,
            tree_name: tree_name.to_string(),
            batch_size,
            current_hits: Vec::new(),
            current_offset: 0,
            finished: false,
        }
    }

    fn load_next_file(&mut self) -> Result<(), SourceError> {
        use super::root_io::read_hits_from_root;

        if self.current_file_index >= self.files.len() {
            self.finished = true;
            return Ok(());
        }

        let path = &self.files[self.current_file_index];
        self.current_hits =
            read_hits_from_root(path, &self.tree_name).map_err(|_| SourceError::Disconnected)?;
        self.current_offset = 0;
        self.current_file_index += 1;

        Ok(())
    }
}

#[cfg(feature = "root")]
impl HitSource for RootFileHitSource {
    fn next_batch(&mut self, _timeout: Duration) -> Result<HitBatch, SourceError> {
        if self.finished {
            return Ok(HitBatch::Eos);
        }

        loop {
            // Yield a slice from the current file's hits
            if self.current_offset < self.current_hits.len() {
                let end = (self.current_offset + self.batch_size).min(self.current_hits.len());
                let batch = self.current_hits[self.current_offset..end].to_vec();
                self.current_offset = end;
                return Ok(HitBatch::Hits(batch));
            }

            // Load next file
            self.load_next_file()?;
            if self.finished {
                return Ok(HitBatch::Eos);
            }
        }
    }

    fn name(&self) -> &str {
        "RootFile"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hit_batch_eos() {
        let source_err = SourceError::Timeout;
        assert_eq!(source_err.to_string(), "source timeout");
        let source_err = SourceError::Disconnected;
        assert_eq!(source_err.to_string(), "source disconnected");
    }

    #[test]
    fn test_delila_file_source_empty() {
        // Empty file list should immediately return Eos
        let mut source = DelilaFileHitSource::new(vec![]);
        let result = source.next_batch(Duration::from_millis(100));
        assert!(matches!(result, Ok(HitBatch::Eos)));
    }

    #[test]
    fn test_delila_file_source_nonexistent() {
        // Non-existent file should return Disconnected
        let mut source = DelilaFileHitSource::new(vec![PathBuf::from("/nonexistent/file.delila")]);
        let result = source.next_batch(Duration::from_millis(100));
        assert!(matches!(result, Err(SourceError::Disconnected)));
    }
}

#[cfg(all(test, feature = "root"))]
mod root_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_root_file_source_empty() {
        let mut source = RootFileHitSource::new(vec![], "tree", 1000);
        let result = source.next_batch(Duration::from_millis(100));
        assert!(matches!(result, Ok(HitBatch::Eos)));
    }

    #[test]
    fn test_root_file_source_reads_hits() {
        use super::super::root_io::write_hits_to_root;
        use crate::event_builder::Hit;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.root");

        // Write test data
        let hits = vec![
            Hit::new(0, 0, 1000, 500, 100.0),
            Hit::new(0, 1, 2000, 600, 200.0),
            Hit::new(1, 0, 3000, 700, 300.0),
        ];
        write_hits_to_root(&path, "test_tree", &hits).unwrap();

        // Read via RootFileHitSource with batch_size=2
        let mut source = RootFileHitSource::new(vec![path], "test_tree", 2);

        // First batch: 2 hits
        let batch1 = source.next_batch(Duration::from_millis(100)).unwrap();
        match batch1 {
            HitBatch::Hits(h) => assert_eq!(h.len(), 2),
            _ => panic!("Expected Hits"),
        }

        // Second batch: 1 hit
        let batch2 = source.next_batch(Duration::from_millis(100)).unwrap();
        match batch2 {
            HitBatch::Hits(h) => assert_eq!(h.len(), 1),
            _ => panic!("Expected Hits"),
        }

        // Third: Eos
        let batch3 = source.next_batch(Duration::from_millis(100)).unwrap();
        assert!(matches!(batch3, HitBatch::Eos));
    }
}

#[cfg(test)]
mod delila_integration_tests {
    use super::*;
    use crate::common::{EventData, EventDataBatch};
    use crate::recorder::{FileFooter, FileHeader};
    use std::io::Write;
    use tempfile::tempdir;

    /// Helper: write a .delila file with given batches
    fn write_test_delila(path: &std::path::Path, batches: &[EventDataBatch]) {
        let mut file = std::fs::File::create(path).unwrap();

        // Write header
        let header = FileHeader::new(1, "test".to_string(), 0);
        header.write_to(&mut file).unwrap();

        // Write data blocks: [u32_le length] + [msgpack data]
        for batch in batches {
            let data = batch.to_msgpack().unwrap();
            let len = (data.len() as u32).to_le_bytes();
            file.write_all(&len).unwrap();
            file.write_all(&data).unwrap();
        }

        // Write footer
        let mut footer = FileFooter::new();
        footer.finalize();
        footer.write_to(&mut file).unwrap();
    }

    #[test]
    fn test_delila_file_source_reads_batches() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_data.delila");

        let batch1 = EventDataBatch {
            source_id: 0,
            sequence_number: 0,
            timestamp: 1000,
            events: vec![
                EventData {
                    module: 0,
                    channel: 0,
                    energy: 1000,
                    energy_short: 500,
                    timestamp_ns: 100.0,
                    flags: 0,
                    user_info: [0; 4],
                    waveform: None,
                },
                EventData {
                    module: 0,
                    channel: 1,
                    energy: 2000,
                    energy_short: 600,
                    timestamp_ns: 200.0,
                    flags: 0,
                    user_info: [0; 4],
                    waveform: None,
                },
            ],
        };
        let batch2 = EventDataBatch {
            source_id: 0,
            sequence_number: 1,
            timestamp: 2000,
            events: vec![EventData {
                module: 1,
                channel: 0,
                energy: 3000,
                energy_short: 700,
                timestamp_ns: 300.0,
                flags: 0,
                user_info: [0; 4],
                waveform: None,
            }],
        };

        write_test_delila(&path, &[batch1, batch2]);

        // Read via DelilaFileHitSource
        let mut source = DelilaFileHitSource::new(vec![path]);

        // First batch: 2 hits
        let b1 = source.next_batch(Duration::from_millis(100)).unwrap();
        match b1 {
            HitBatch::Hits(h) => {
                assert_eq!(h.len(), 2);
                assert_eq!(h[0].module, 0);
                assert_eq!(h[0].channel, 0);
                assert_eq!(h[0].energy, 1000);
                assert!((h[0].timestamp_ns - 100.0).abs() < 1e-6);
                assert_eq!(h[1].module, 0);
                assert_eq!(h[1].channel, 1);
            }
            _ => panic!("Expected Hits"),
        }

        // Second batch: 1 hit
        let b2 = source.next_batch(Duration::from_millis(100)).unwrap();
        match b2 {
            HitBatch::Hits(h) => {
                assert_eq!(h.len(), 1);
                assert_eq!(h[0].module, 1);
                assert_eq!(h[0].channel, 0);
            }
            _ => panic!("Expected Hits"),
        }

        // Eos
        let b3 = source.next_batch(Duration::from_millis(100)).unwrap();
        assert!(matches!(b3, HitBatch::Eos));
    }

    #[test]
    fn test_delila_file_source_multiple_files() {
        let dir = tempdir().unwrap();

        // Create two .delila files
        for i in 0..2u32 {
            let path = dir.path().join(format!("test_{}.delila", i));
            let batch = EventDataBatch {
                source_id: 0,
                sequence_number: i as u64,
                timestamp: (i as u64) * 1000,
                events: vec![EventData {
                    module: i as u8,
                    channel: 0,
                    energy: 1000,
                    energy_short: 500,
                    timestamp_ns: (i as f64) * 100.0,
                    flags: 0,
                    user_info: [0; 4],
                    waveform: None,
                }],
            };
            write_test_delila(&path, &[batch]);
        }

        let files: Vec<PathBuf> = (0..2)
            .map(|i| dir.path().join(format!("test_{}.delila", i)))
            .collect();
        let mut source = DelilaFileHitSource::new(files);

        // File 0, batch
        let b1 = source.next_batch(Duration::from_millis(100)).unwrap();
        match b1 {
            HitBatch::Hits(h) => {
                assert_eq!(h.len(), 1);
                assert_eq!(h[0].module, 0);
            }
            _ => panic!("Expected Hits from file 0"),
        }

        // File 1, batch
        let b2 = source.next_batch(Duration::from_millis(100)).unwrap();
        match b2 {
            HitBatch::Hits(h) => {
                assert_eq!(h.len(), 1);
                assert_eq!(h[0].module, 1);
            }
            _ => panic!("Expected Hits from file 1"),
        }

        // Eos
        let b3 = source.next_batch(Duration::from_millis(100)).unwrap();
        assert!(matches!(b3, HitBatch::Eos));
    }
}

// ===========================================================================
// ZmqHitSource — Merger PUB → OnlineHit (online EB path)
// ===========================================================================

/// ZMQ SUB-backed [`HitSource`].
///
/// Subscribes to a Merger PUB endpoint, decodes each MessagePack
/// `Message::Data(EventDataBatch)` into a batch of `Hit` (= `OnlineHit`),
/// and surfaces `Message::EndOfStream` as `HitBatch::Eos`.
///
/// # Design notes
///
/// * Uses the raw `zmq` crate (not `tmq`) because the [`HitSource`] trait
///   is synchronous — wrapping an async tmq socket in a runtime + channel
///   only to call it from a `std::thread` adds latency and complexity.
/// * `RCVHWM = 0` (zero-loss policy from CLAUDE.md): the SUB socket buffers
///   in memory indefinitely rather than dropping messages.
/// * Heartbeat messages do not produce hits — we return `SourceError::Timeout`
///   so the sorter thread treats them as "no data yet, keep polling".
/// * After the first EOS the source is latched; subsequent `next_batch`
///   calls return [`HitBatch::Eos`] without touching the socket.
pub struct ZmqHitSource {
    socket: zmq::Socket,
    _context: zmq::Context,
    name: String,
    saw_eos: bool,
    last_timeout_ms: i32,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Handle for requesting graceful shutdown of a [`ZmqHitSource`] from
/// another thread / async task (typically a Ctrl+C handler).
///
/// Setting the flag makes the next `next_batch` call (after the current
/// blocking `recv` returns or times out) drain its latch and surface
/// `SourceError::Disconnected` to the pipeline so it can flush and exit.
#[derive(Debug, Clone)]
pub struct ZmqHitSourceShutdown {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ZmqHitSourceShutdown {
    pub fn request(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Release);
    }
}

impl ZmqHitSource {
    /// Connect to a Merger PUB endpoint (e.g. `tcp://localhost:5556`).
    pub fn connect(subscribe_address: &str) -> Result<Self, ZmqHitSourceError> {
        let context = zmq::Context::new();
        let socket = context.socket(zmq::SUB)?;
        socket.set_rcvhwm(0)?; // never drop — buffer in memory (SPEC § 12, CLAUDE.md)
        socket.connect(subscribe_address)?;
        socket.set_subscribe(b"")?;

        Ok(Self {
            socket,
            _context: context,
            name: format!("zmq:{subscribe_address}"),
            saw_eos: false,
            last_timeout_ms: -1,
            shutdown: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Get a clonable handle that other threads can use to request shutdown.
    pub fn shutdown_handle(&self) -> ZmqHitSourceShutdown {
        ZmqHitSourceShutdown {
            flag: self.shutdown.clone(),
        }
    }

    fn ensure_rcvtimeo(&mut self, timeout: Duration) -> Result<(), SourceError> {
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        if ms != self.last_timeout_ms {
            self.socket
                .set_rcvtimeo(ms)
                .map_err(|_| SourceError::Disconnected)?;
            self.last_timeout_ms = ms;
        }
        Ok(())
    }
}

impl HitSource for ZmqHitSource {
    fn next_batch(&mut self, timeout: Duration) -> Result<HitBatch, SourceError> {
        use crate::common::Message;

        if self.saw_eos {
            return Ok(HitBatch::Eos);
        }
        if self.shutdown.load(std::sync::atomic::Ordering::Acquire) {
            // External shutdown — tell the pipeline to flush and exit.
            tracing::info!(source = self.name, "ZmqHitSource shutdown requested");
            return Err(SourceError::Disconnected);
        }

        self.ensure_rcvtimeo(timeout)?;

        let msg = match self.socket.recv_bytes(0) {
            Ok(b) => b,
            Err(zmq::Error::EAGAIN) => return Err(SourceError::Timeout),
            Err(e) => {
                tracing::error!(error = %e, source = self.name, "ZmqHitSource recv failed");
                return Err(SourceError::Disconnected);
            }
        };

        match Message::from_msgpack(&msg) {
            Ok(Message::Data(batch)) => {
                let hits: Vec<Hit> = batch.events.iter().map(Hit::from_event_data).collect();
                Ok(HitBatch::Hits(hits))
            }
            Ok(Message::EndOfStream {
                source_id,
                run_number,
            }) => {
                tracing::info!(
                    source = self.name,
                    source_id,
                    run_number,
                    "ZmqHitSource: EOS received"
                );
                self.saw_eos = true;
                Ok(HitBatch::Eos)
            }
            Ok(Message::Heartbeat(_)) => {
                // No data; tell the sorter to keep polling.
                Err(SourceError::Timeout)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    source = self.name,
                    bytes = msg.len(),
                    "ZmqHitSource: msgpack decode failed (skipping frame)"
                );
                // Treat as no-data so the sorter keeps trying instead of bailing.
                Err(SourceError::Timeout)
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ZmqHitSourceError {
    #[error("ZMQ error: {0}")]
    Zmq(#[from] zmq::Error),
}

#[cfg(test)]
mod zmq_tests {
    use super::*;
    use crate::common::{EventData, EventDataBatch, Message};

    fn fresh_endpoint() -> String {
        // Use inproc to avoid port collisions in concurrent test runs.
        format!(
            "inproc://zmq-hit-source-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    /// Bind a PUB socket sharing the same context as the SUB (required for
    /// `inproc://` transport). Returns (ctx, pub_socket, sub_source).
    fn paired(endpoint: &str) -> (zmq::Context, zmq::Socket, ZmqHitSource) {
        let context = zmq::Context::new();
        let publisher = context.socket(zmq::PUB).unwrap();
        publisher.bind(endpoint).unwrap();

        // Build the SUB on the SAME context so inproc works.
        let socket = context.socket(zmq::SUB).unwrap();
        socket.set_rcvhwm(0).unwrap();
        socket.connect(endpoint).unwrap();
        socket.set_subscribe(b"").unwrap();
        let source = ZmqHitSource {
            socket,
            _context: context.clone(),
            name: format!("zmq:{endpoint}"),
            saw_eos: false,
            last_timeout_ms: -1,
            shutdown: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        // SUB late-join on PUB needs a brief warmup window before first send.
        std::thread::sleep(Duration::from_millis(50));
        (context, publisher, source)
    }

    fn send(publisher: &zmq::Socket, msg: &Message) {
        publisher.send(msg.to_msgpack().unwrap(), 0).unwrap();
    }

    #[test]
    fn data_batch_round_trips() {
        let ep = fresh_endpoint();
        let (_ctx, publisher, mut source) = paired(&ep);

        let mut batch = EventDataBatch::new(0, 1);
        batch.events.push(EventData::new(1, 2, 100, 50, 1000.0, 0));
        batch.events.push(EventData::new(1, 3, 200, 60, 1010.0, 0));
        send(&publisher, &Message::data(batch));

        // Poll up to ~500 ms total across short windows.
        let mut got = None;
        for _ in 0..20 {
            match source.next_batch(Duration::from_millis(25)) {
                Ok(HitBatch::Hits(h)) => {
                    got = Some(h);
                    break;
                }
                Ok(HitBatch::Eos) => panic!("unexpected EOS"),
                Err(SourceError::Timeout) => continue,
                Err(SourceError::Disconnected) => panic!("disconnected"),
            }
        }
        let hits = got.expect("expected a Data batch within 500ms");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].channel, 2);
        assert_eq!(hits[1].channel, 3);
        assert_eq!(hits[0].timestamp_ns, 1000.0);
    }

    #[test]
    fn eos_returns_eos_and_latches() {
        let ep = fresh_endpoint();
        let (_ctx, publisher, mut source) = paired(&ep);

        send(&publisher, &Message::eos(7, 42));

        let mut got_eos = false;
        for _ in 0..20 {
            match source.next_batch(Duration::from_millis(25)) {
                Ok(HitBatch::Eos) => {
                    got_eos = true;
                    break;
                }
                Err(SourceError::Timeout) => continue,
                other => panic!("unexpected: {other:?}"),
            }
        }
        assert!(got_eos, "expected EOS within 500ms");

        // Latched: subsequent calls return EOS immediately even if no message arrives.
        for _ in 0..3 {
            assert!(matches!(
                source.next_batch(Duration::from_millis(1)),
                Ok(HitBatch::Eos)
            ));
        }
    }

    #[test]
    fn timeout_when_idle() {
        let ep = fresh_endpoint();
        let (_ctx, _publisher, mut source) = paired(&ep);

        let r = source.next_batch(Duration::from_millis(20));
        assert!(matches!(r, Err(SourceError::Timeout)), "got {r:?}");
    }

    #[test]
    fn shutdown_handle_breaks_polling_loop() {
        let ep = fresh_endpoint();
        let (_ctx, _publisher, mut source) = paired(&ep);
        let handle = source.shutdown_handle();

        // Idle source: first poll times out.
        assert!(matches!(
            source.next_batch(Duration::from_millis(10)),
            Err(SourceError::Timeout)
        ));

        // Request shutdown; the next poll should report Disconnected so
        // the pipeline can drain and exit.
        handle.request();
        assert!(matches!(
            source.next_batch(Duration::from_millis(10)),
            Err(SourceError::Disconnected)
        ));
    }

    #[test]
    fn heartbeat_is_a_timeout_not_eos() {
        use crate::common::Heartbeat;

        let ep = fresh_endpoint();
        let (_ctx, publisher, mut source) = paired(&ep);

        send(&publisher, &Message::Heartbeat(Heartbeat::new(0, 0)));

        // Should not produce Hits or Eos. Within 500ms we should still
        // see only Timeout errors (or no event at all).
        for _ in 0..20 {
            match source.next_batch(Duration::from_millis(25)) {
                Err(SourceError::Timeout) => continue,
                Ok(HitBatch::Eos) => panic!("heartbeat must not surface as EOS"),
                Ok(HitBatch::Hits(_)) => panic!("heartbeat must not surface as Hits"),
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        assert!(!source.saw_eos);
    }
}

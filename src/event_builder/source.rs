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
                    waveform: None,
                },
                EventData {
                    module: 0,
                    channel: 1,
                    energy: 2000,
                    energy_short: 600,
                    timestamp_ns: 200.0,
                    flags: 0,
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

//! Time-sorted buffer for event building
//!
//! BTreeMap ベースのタイムソートバッファ。
//! Watermark 方式で安全にデータを取り出す。
//!
//! Note: Used internally by L1Builder (offline). Not exported publicly.
#![allow(dead_code)]

use super::hit::Hit;
use std::collections::BTreeMap;

/// Time-sorted buffer for hits
///
/// Hits are stored sorted by timestamp. The buffer uses a watermark
/// to determine when hits are safe to extract (all earlier hits have arrived).
#[derive(Debug)]
pub struct TimeSortBuffer {
    /// Sorted storage: timestamp_ns -> Vec<Hit> (multiple hits at same time possible)
    buffer: BTreeMap<i64, Vec<Hit>>,
    /// Buffer delay in nanoseconds (watermark = max_time - delay)
    buffer_delay_ns: f64,
    /// Maximum timestamp seen so far
    max_timestamp: f64,
    /// Total number of hits in buffer
    count: usize,
}

impl TimeSortBuffer {
    /// Create a new buffer with the given delay
    ///
    /// # Arguments
    /// * `buffer_delay_ns` - How far behind the maximum timestamp before data is "ready"
    ///   Typically 1000-10000 ns depending on expected time disorder
    pub fn new(buffer_delay_ns: f64) -> Self {
        Self {
            buffer: BTreeMap::new(),
            buffer_delay_ns,
            max_timestamp: f64::NEG_INFINITY,
            count: 0,
        }
    }

    /// Insert a hit into the buffer
    ///
    /// Updates the maximum timestamp if this hit is later.
    pub fn insert(&mut self, hit: Hit) {
        let ts = hit.timestamp_ns;
        if ts > self.max_timestamp {
            self.max_timestamp = ts;
        }

        // Convert to integer key (picosecond precision is overkill, use integer ns)
        let key = ts as i64;
        self.buffer.entry(key).or_default().push(hit);
        self.count += 1;
    }

    /// Get the current watermark (timestamps below this are safe to extract)
    #[inline]
    pub fn watermark(&self) -> f64 {
        if self.max_timestamp == f64::NEG_INFINITY {
            f64::NEG_INFINITY
        } else {
            self.max_timestamp - self.buffer_delay_ns
        }
    }

    /// Drain all hits with timestamps below the watermark
    ///
    /// Returns hits in time-sorted order.
    pub fn drain_ready(&mut self) -> Vec<Hit> {
        let watermark = self.watermark();
        if watermark == f64::NEG_INFINITY {
            return Vec::new();
        }

        let watermark_key = watermark as i64;
        let mut result = Vec::new();

        // Collect keys to remove
        let keys_to_remove: Vec<i64> = self
            .buffer
            .range(..=watermark_key)
            .map(|(k, _)| *k)
            .collect();

        for key in keys_to_remove {
            if let Some(hits) = self.buffer.remove(&key) {
                self.count -= hits.len();
                result.extend(hits);
            }
        }

        // Sort by exact timestamp (integer key may group slightly different times)
        result.sort_by(|a, b| {
            a.timestamp_ns
                .partial_cmp(&b.timestamp_ns)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        result
    }

    /// Flush all remaining hits from the buffer
    ///
    /// Used at end of run to get any remaining hits.
    pub fn flush(&mut self) -> Vec<Hit> {
        let mut result = Vec::new();

        for (_, hits) in self.buffer.iter() {
            result.extend(hits.iter().cloned());
        }

        result.sort_by(|a, b| {
            a.timestamp_ns
                .partial_cmp(&b.timestamp_ns)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        self.buffer.clear();
        self.count = 0;
        self.max_timestamp = f64::NEG_INFINITY;

        result
    }

    /// Number of hits currently in the buffer
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if buffer is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get the maximum timestamp seen
    #[inline]
    pub fn max_timestamp(&self) -> f64 {
        self.max_timestamp
    }
}

impl Default for TimeSortBuffer {
    fn default() -> Self {
        Self::new(1000.0) // 1 microsecond default delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(module: u8, channel: u8, ts: f64) -> Hit {
        Hit::new(module, channel, 1000, 500, ts)
    }

    #[test]
    fn test_new_buffer() {
        let buffer = TimeSortBuffer::new(500.0);
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.watermark(), f64::NEG_INFINITY);
    }

    #[test]
    fn test_insert_updates_max_timestamp() {
        let mut buffer = TimeSortBuffer::new(100.0);

        buffer.insert(make_hit(0, 0, 1000.0));
        assert_eq!(buffer.max_timestamp(), 1000.0);
        assert_eq!(buffer.len(), 1);

        buffer.insert(make_hit(0, 1, 500.0)); // Earlier hit
        assert_eq!(buffer.max_timestamp(), 1000.0); // Max unchanged

        buffer.insert(make_hit(0, 2, 1500.0)); // Later hit
        assert_eq!(buffer.max_timestamp(), 1500.0);
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn test_watermark_calculation() {
        let mut buffer = TimeSortBuffer::new(200.0);

        buffer.insert(make_hit(0, 0, 1000.0));
        assert_eq!(buffer.watermark(), 800.0); // 1000 - 200

        buffer.insert(make_hit(0, 1, 1500.0));
        assert_eq!(buffer.watermark(), 1300.0); // 1500 - 200
    }

    #[test]
    fn test_drain_ready_returns_sorted_hits() {
        let mut buffer = TimeSortBuffer::new(200.0);

        // Insert out of order
        buffer.insert(make_hit(0, 0, 1000.0));
        buffer.insert(make_hit(0, 1, 800.0));
        buffer.insert(make_hit(0, 2, 900.0));
        buffer.insert(make_hit(0, 3, 1100.0));

        // Advance time to make some ready
        buffer.insert(make_hit(0, 4, 1500.0));
        // Watermark = 1500 - 200 = 1300
        // Hits at 800, 900, 1000, 1100 should be ready

        let ready = buffer.drain_ready();
        assert_eq!(ready.len(), 4);
        assert_eq!(ready[0].timestamp_ns, 800.0);
        assert_eq!(ready[1].timestamp_ns, 900.0);
        assert_eq!(ready[2].timestamp_ns, 1000.0);
        assert_eq!(ready[3].timestamp_ns, 1100.0);

        // Buffer should only have the 1500 hit left
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn test_drain_ready_empty_when_nothing_ready() {
        let mut buffer = TimeSortBuffer::new(1000.0);

        buffer.insert(make_hit(0, 0, 500.0));
        buffer.insert(make_hit(0, 1, 800.0));
        // Max = 800, watermark = 800 - 1000 = -200
        // No hits below -200

        let ready = buffer.drain_ready();
        assert!(ready.is_empty());
        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn test_flush_returns_all_sorted() {
        let mut buffer = TimeSortBuffer::new(1000.0);

        buffer.insert(make_hit(0, 0, 300.0));
        buffer.insert(make_hit(0, 1, 100.0));
        buffer.insert(make_hit(0, 2, 200.0));

        let all = buffer.flush();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].timestamp_ns, 100.0);
        assert_eq!(all[1].timestamp_ns, 200.0);
        assert_eq!(all[2].timestamp_ns, 300.0);

        assert!(buffer.is_empty());
        assert_eq!(buffer.max_timestamp(), f64::NEG_INFINITY);
    }

    #[test]
    fn test_multiple_hits_same_timestamp() {
        let mut buffer = TimeSortBuffer::new(100.0);

        buffer.insert(make_hit(0, 0, 1000.0));
        buffer.insert(make_hit(0, 1, 1000.0)); // Same time, different channel
        buffer.insert(make_hit(1, 0, 1000.0)); // Same time, different module

        assert_eq!(buffer.len(), 3);

        buffer.insert(make_hit(0, 0, 1200.0)); // Advance time
        let ready = buffer.drain_ready();

        assert_eq!(ready.len(), 3);
        // All three should have timestamp 1000.0
        for hit in &ready {
            assert_eq!(hit.timestamp_ns, 1000.0);
        }
    }

    #[test]
    fn test_default_buffer() {
        let buffer = TimeSortBuffer::default();
        assert!(buffer.is_empty());
        // Default delay is 1000 ns (1 microsecond)
    }

    #[test]
    fn test_realistic_data_flow() {
        // Simulate realistic data flow with some time disorder
        let mut buffer = TimeSortBuffer::new(500.0); // 500 ns buffer
        let mut extracted = Vec::new();

        // Batch 1: hits around t=10000
        buffer.insert(make_hit(0, 0, 10100.0));
        buffer.insert(make_hit(0, 1, 9900.0)); // Slight disorder
        buffer.insert(make_hit(1, 0, 10050.0));

        // Batch 2: hits around t=11000
        buffer.insert(make_hit(0, 0, 11000.0));
        buffer.insert(make_hit(0, 1, 10800.0));
        extracted.extend(buffer.drain_ready());

        // Batch 3: hits around t=12000
        buffer.insert(make_hit(0, 0, 12000.0));
        buffer.insert(make_hit(1, 1, 11500.0));
        extracted.extend(buffer.drain_ready());

        // Flush remaining
        extracted.extend(buffer.flush());

        // Should have all 7 hits in time order
        assert_eq!(extracted.len(), 7);
        for i in 1..extracted.len() {
            assert!(
                extracted[i].timestamp_ns >= extracted[i - 1].timestamp_ns,
                "Hits should be in time order"
            );
        }
    }
}

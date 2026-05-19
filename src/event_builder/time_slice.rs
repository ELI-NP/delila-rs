//! Time Slice - Data structure for time-based event segmentation
//!
//! Time Slice 方式: 時間軸を固定サイズのスライスに分割し、
//! 各スライス内でコインシデンス検出を行う。
//! CBM/FLES で実績のある方式。
//!
//! Note: Used internally by SliceBuilder (offline). Not exported publicly.
#![allow(dead_code)]

use super::hit::Hit;

/// A time slice containing hits within a time window
///
/// ```text
/// 時間軸 ─────────────────────────────────────────────────────────▶
///
/// Slice N:   [========= Core =========][= Overlap =]
///                                       ↑
///                                  coincidence_window
///
/// Slice N+1:                     [= Overlap =][======= Core =======]
/// ```
#[derive(Debug, Clone)]
pub struct TimeSlice {
    /// Slice start time [ns]
    pub start_ns: f64,
    /// Slice end time [ns] (exclusive)
    pub end_ns: f64,
    /// Overlap duration [ns] (= coincidence_window)
    pub overlap_ns: f64,
    /// Hits within this slice (time-sorted)
    pub hits: Vec<Hit>,
}

impl TimeSlice {
    /// Create a new time slice
    pub fn new(start_ns: f64, end_ns: f64, overlap_ns: f64) -> Self {
        Self {
            start_ns,
            end_ns,
            overlap_ns,
            hits: Vec::new(),
        }
    }

    /// Create a new time slice with pre-allocated capacity
    pub fn with_capacity(start_ns: f64, end_ns: f64, overlap_ns: f64, capacity: usize) -> Self {
        Self {
            start_ns,
            end_ns,
            overlap_ns,
            hits: Vec::with_capacity(capacity),
        }
    }

    /// Get the core region end time
    ///
    /// Triggers in the overlap region are processed in the next slice.
    #[inline]
    pub fn core_end_ns(&self) -> f64 {
        self.end_ns - self.overlap_ns
    }

    /// Check if a timestamp is in the core region
    ///
    /// Core region: [start_ns, end_ns - overlap_ns)
    #[inline]
    pub fn is_in_core(&self, timestamp_ns: f64) -> bool {
        timestamp_ns >= self.start_ns && timestamp_ns < self.core_end_ns()
    }

    /// Check if a timestamp is in the overlap region
    ///
    /// Overlap region: [end_ns - overlap_ns, end_ns)
    #[inline]
    pub fn is_in_overlap(&self, timestamp_ns: f64) -> bool {
        timestamp_ns >= self.core_end_ns() && timestamp_ns < self.end_ns
    }

    /// Check if a timestamp is within this slice (core or overlap)
    #[inline]
    pub fn contains(&self, timestamp_ns: f64) -> bool {
        timestamp_ns >= self.start_ns && timestamp_ns < self.end_ns
    }

    /// Add a hit to this slice
    pub fn add_hit(&mut self, hit: Hit) {
        self.hits.push(hit);
    }

    /// Get the slice duration [ns]
    #[inline]
    pub fn duration_ns(&self) -> f64 {
        self.end_ns - self.start_ns
    }

    /// Get the core duration [ns]
    #[inline]
    pub fn core_duration_ns(&self) -> f64 {
        self.duration_ns() - self.overlap_ns
    }

    /// Get the number of hits in this slice
    #[inline]
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// Check if this slice has no hits
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }
}

/// Create time slices from a sorted list of hits
///
/// # Arguments
/// * `hits` - Time-sorted hits
/// * `slice_duration_ns` - Duration of each slice [ns]
/// * `overlap_ns` - Overlap between slices [ns] (typically = coincidence_window)
///
/// # Returns
/// Vector of TimeSlice, each containing hits within its time range.
/// Hits in the overlap region appear in both adjacent slices.
pub fn create_slices(hits: &[Hit], slice_duration_ns: f64, overlap_ns: f64) -> Vec<TimeSlice> {
    if hits.is_empty() {
        return Vec::new();
    }

    // Find time range
    let min_time = hits.first().map(|h| h.timestamp_ns).unwrap_or(0.0);
    let max_time = hits.last().map(|h| h.timestamp_ns).unwrap_or(0.0);

    // Calculate number of slices needed
    // Core duration = slice_duration - overlap
    let core_duration = slice_duration_ns - overlap_ns;
    if core_duration <= 0.0 {
        // Invalid parameters: overlap >= slice_duration
        // Fall back to single slice
        let mut slice = TimeSlice::new(min_time, max_time + overlap_ns, overlap_ns);
        slice.hits = hits.to_vec();
        return vec![slice];
    }

    let time_range = max_time - min_time;
    let n_slices = ((time_range / core_duration).ceil() as usize).max(1);

    // Create slices
    let mut slices: Vec<TimeSlice> = (0..n_slices)
        .map(|i| {
            let start = min_time + (i as f64) * core_duration;
            let end = start + slice_duration_ns;
            TimeSlice::with_capacity(start, end, overlap_ns, hits.len() / n_slices + 100)
        })
        .collect();

    // Distribute hits to slices
    // Hits in overlap regions are added to both adjacent slices
    for hit in hits {
        let ts = hit.timestamp_ns;

        // Find which slice(s) this hit belongs to
        for slice in slices.iter_mut() {
            if slice.contains(ts) {
                slice.add_hit(*hit);
            }
        }
    }

    slices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(ts: f64) -> Hit {
        Hit::new(0, 0, 1000, 500, ts)
    }

    #[test]
    fn test_slice_new() {
        let slice = TimeSlice::new(0.0, 1000.0, 100.0);
        assert_eq!(slice.start_ns, 0.0);
        assert_eq!(slice.end_ns, 1000.0);
        assert_eq!(slice.overlap_ns, 100.0);
        assert!(slice.is_empty());
    }

    #[test]
    fn test_slice_core_end() {
        let slice = TimeSlice::new(0.0, 1000.0, 100.0);
        assert_eq!(slice.core_end_ns(), 900.0);
    }

    #[test]
    fn test_slice_is_in_core() {
        let slice = TimeSlice::new(0.0, 1000.0, 100.0);

        // Core region: [0, 900)
        assert!(slice.is_in_core(0.0));
        assert!(slice.is_in_core(500.0));
        assert!(slice.is_in_core(899.9));
        assert!(!slice.is_in_core(900.0)); // Overlap start
        assert!(!slice.is_in_core(950.0));
        assert!(!slice.is_in_core(-1.0)); // Before slice
    }

    #[test]
    fn test_slice_is_in_overlap() {
        let slice = TimeSlice::new(0.0, 1000.0, 100.0);

        // Overlap region: [900, 1000)
        assert!(!slice.is_in_overlap(0.0));
        assert!(!slice.is_in_overlap(899.9));
        assert!(slice.is_in_overlap(900.0));
        assert!(slice.is_in_overlap(950.0));
        assert!(slice.is_in_overlap(999.9));
        assert!(!slice.is_in_overlap(1000.0)); // End of slice
    }

    #[test]
    fn test_slice_contains() {
        let slice = TimeSlice::new(0.0, 1000.0, 100.0);

        assert!(slice.contains(0.0));
        assert!(slice.contains(500.0));
        assert!(slice.contains(999.9));
        assert!(!slice.contains(-1.0));
        assert!(!slice.contains(1000.0));
    }

    #[test]
    fn test_create_slices_empty() {
        let hits: Vec<Hit> = vec![];
        let slices = create_slices(&hits, 1000.0, 100.0);
        assert!(slices.is_empty());
    }

    #[test]
    fn test_create_slices_single() {
        let hits = vec![make_hit(100.0), make_hit(200.0), make_hit(300.0)];
        let slices = create_slices(&hits, 1000.0, 100.0);

        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].len(), 3);
    }

    #[test]
    fn test_create_slices_multiple() {
        // Hits spanning 0 to 2000 ns
        let hits = vec![
            make_hit(0.0),
            make_hit(400.0),
            make_hit(800.0),
            make_hit(900.0),  // In overlap of slice 0
            make_hit(1000.0), // In slice 1
            make_hit(1500.0),
        ];

        // slice_duration=1000, overlap=100
        // core_duration = 900
        // Slice 0: [0, 1000), core [0, 900)
        // Slice 1: [900, 1900), core [900, 1800)
        // Slice 2: [1800, 2800), core [1800, 2700)
        let slices = create_slices(&hits, 1000.0, 100.0);

        assert!(slices.len() >= 2);

        // Hit at 900.0 should be in both slice 0 (overlap) and slice 1 (core)
        let hit_900_in_slice0 = slices[0]
            .hits
            .iter()
            .any(|h| (h.timestamp_ns - 900.0).abs() < 0.01);
        let hit_900_in_slice1 = slices[1]
            .hits
            .iter()
            .any(|h| (h.timestamp_ns - 900.0).abs() < 0.01);

        assert!(
            hit_900_in_slice0,
            "Hit at 900ns should be in slice 0 overlap"
        );
        assert!(hit_900_in_slice1, "Hit at 900ns should be in slice 1 core");
    }

    #[test]
    fn test_create_slices_overlap_distribution() {
        // Test that hits in overlap appear in both slices
        // slice_duration=1000, overlap=100, core_duration=900
        // With min_time=0:
        // Slice 0: [0, 1000), core [0, 900), overlap [900, 1000)
        // Slice 1: [900, 1900), core [900, 1800), overlap [1800, 1900)
        let hits = vec![
            make_hit(0.0),    // Core of slice 0
            make_hit(500.0),  // Core of slice 0
            make_hit(950.0),  // In overlap of slice 0 [900,1000) AND in slice 1 [900,1900)
            make_hit(1500.0), // Core of slice 1
        ];

        let slices = create_slices(&hits, 1000.0, 100.0);

        // Should have 2 slices
        assert!(
            slices.len() >= 2,
            "Expected at least 2 slices, got {}",
            slices.len()
        );

        // Hit at 950ns should be in both:
        // - Slice 0: overlap region [900, 1000)
        // - Slice 1: [900, 1900)
        let count_950 = slices
            .iter()
            .filter(|s| s.hits.iter().any(|h| (h.timestamp_ns - 950.0).abs() < 0.01))
            .count();

        assert_eq!(
            count_950, 2,
            "Hit at 950ns should appear in 2 slices (overlap region)"
        );
    }
}

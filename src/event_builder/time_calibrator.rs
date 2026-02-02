//! Time Calibration - Measure channel time offsets
//!
//! ヒストグラムベースのピーク検出でタイムキャリブレーションを行う。
//! ELIFANT-Event の TimeAlignment に相当。

use super::config::TimeCalibration;
use super::hit::Hit;
use std::collections::HashMap;

/// Time histogram for a single channel
#[derive(Debug, Clone)]
pub struct TimeHistogram {
    /// Histogram bins (bin index -> count)
    bins: Vec<u64>,
    /// Bin width in nanoseconds
    bin_width: f64,
    /// Histogram range: min value
    min_ns: f64,
    /// Total entries
    entries: u64,
}

impl TimeHistogram {
    /// Create a new histogram
    ///
    /// # Arguments
    /// * `min_ns` - Minimum value (left edge of first bin)
    /// * `max_ns` - Maximum value (right edge of last bin)
    /// * `bin_width` - Width of each bin in nanoseconds
    pub fn new(min_ns: f64, max_ns: f64, bin_width: f64) -> Self {
        let n_bins = ((max_ns - min_ns) / bin_width).ceil() as usize;
        Self {
            bins: vec![0; n_bins],
            bin_width,
            min_ns,
            entries: 0,
        }
    }

    /// Fill the histogram with a value
    pub fn fill(&mut self, value: f64) {
        if value < self.min_ns {
            return;
        }
        let bin = ((value - self.min_ns) / self.bin_width) as usize;
        if bin < self.bins.len() {
            self.bins[bin] += 1;
            self.entries += 1;
        }
    }

    /// Merge another histogram into this one
    ///
    /// Both histograms must have the same parameters (min, max, bin_width).
    pub fn merge(&mut self, other: &TimeHistogram) {
        assert_eq!(self.bins.len(), other.bins.len(), "Histogram size mismatch");
        for (i, &count) in other.bins.iter().enumerate() {
            self.bins[i] += count;
        }
        self.entries += other.entries;
    }

    /// Get the bin center with maximum counts
    pub fn find_peak(&self) -> Option<f64> {
        if self.entries == 0 {
            return None;
        }

        let (max_bin, &max_count) = self
            .bins
            .iter()
            .enumerate()
            .max_by_key(|(_, &count)| count)?;

        if max_count == 0 {
            return None;
        }

        // Return bin center
        Some(self.min_ns + (max_bin as f64 + 0.5) * self.bin_width)
    }

    /// Get the bin center with maximum counts using weighted average around peak
    ///
    /// More accurate than simple bin center for well-defined peaks.
    pub fn find_peak_centroid(&self, window_bins: usize) -> Option<f64> {
        if self.entries == 0 {
            return None;
        }

        let (max_bin, &max_count) = self
            .bins
            .iter()
            .enumerate()
            .max_by_key(|(_, &count)| count)?;

        if max_count == 0 {
            return None;
        }

        // Calculate weighted centroid around peak
        let start = max_bin.saturating_sub(window_bins);
        let end = (max_bin + window_bins + 1).min(self.bins.len());

        let mut sum_weight = 0.0;
        let mut sum_value = 0.0;

        for bin in start..end {
            let count = self.bins[bin] as f64;
            let center = self.min_ns + (bin as f64 + 0.5) * self.bin_width;
            sum_weight += count;
            sum_value += count * center;
        }

        if sum_weight > 0.0 {
            Some(sum_value / sum_weight)
        } else {
            None
        }
    }

    /// Get total entries
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Get number of bins
    pub fn n_bins(&self) -> usize {
        self.bins.len()
    }

    /// Get bin contents
    pub fn bins(&self) -> &[u64] {
        &self.bins
    }

    /// Get bin center for a given bin index
    pub fn bin_center(&self, bin: usize) -> f64 {
        self.min_ns + (bin as f64 + 0.5) * self.bin_width
    }
}

/// Time calibrator
///
/// Accumulates time differences between detector hits and reference trigger,
/// then calculates offsets from histogram peaks.
pub struct TimeCalibrator {
    /// Reference trigger module
    ref_module: u8,
    /// Reference trigger channel
    ref_channel: u8,
    /// Time window for coincidence [ns]
    window_ns: f64,
    /// Histograms: (module, channel) -> histogram
    histograms: HashMap<(u8, u8), TimeHistogram>,
    /// Histogram range: min
    hist_min: f64,
    /// Histogram range: max
    hist_max: f64,
    /// Histogram bin width
    bin_width: f64,
    /// Minimum entries required for valid calibration
    min_entries: u64,
}

impl TimeCalibrator {
    /// Create a new time calibrator
    ///
    /// # Arguments
    /// * `ref_module` - Reference trigger module
    /// * `ref_channel` - Reference trigger channel
    /// * `window_ns` - Coincidence window [ns] (symmetric around 0)
    pub fn new(ref_module: u8, ref_channel: u8, window_ns: f64) -> Self {
        Self {
            ref_module,
            ref_channel,
            window_ns,
            histograms: HashMap::new(),
            hist_min: -window_ns,
            hist_max: window_ns,
            bin_width: 1.0, // 1 ns bins by default
            min_entries: 100,
        }
    }

    /// Set histogram parameters
    pub fn set_histogram_params(&mut self, min_ns: f64, max_ns: f64, bin_width: f64) {
        self.hist_min = min_ns;
        self.hist_max = max_ns;
        self.bin_width = bin_width;
    }

    /// Set minimum entries for valid calibration
    pub fn set_min_entries(&mut self, min_entries: u64) {
        self.min_entries = min_entries;
    }

    /// Process a batch of hits (non-sorted, O(n*m) complexity)
    ///
    /// For time-sorted data, use `process_hits_sorted` instead.
    pub fn process_hits(&mut self, hits: &[Hit]) {
        // Find all reference triggers
        let triggers: Vec<usize> = hits
            .iter()
            .enumerate()
            .filter(|(_, h)| h.module == self.ref_module && h.channel == self.ref_channel)
            .map(|(i, _)| i)
            .collect();

        for &trig_idx in &triggers {
            let trigger = &hits[trig_idx];
            let trig_time = trigger.timestamp_ns;

            // Find coincident hits
            for (idx, hit) in hits.iter().enumerate() {
                if idx == trig_idx {
                    continue; // Skip the trigger itself
                }

                let dt = hit.timestamp_ns - trig_time;

                // Check if within window
                if dt.abs() <= self.window_ns {
                    self.fill_histogram(hit.module, hit.channel, dt);
                }
            }
        }
    }

    /// Process time-sorted hits using sliding window (O(n) complexity)
    ///
    /// This is much faster than `process_hits` for time-sorted data.
    /// It only looks at nearby hits for each trigger.
    pub fn process_hits_sorted(&mut self, hits: &[Hit]) {
        if hits.is_empty() {
            return;
        }

        let window = self.window_ns;

        for (trig_idx, trigger) in hits.iter().enumerate() {
            // Check if this is a reference trigger
            if trigger.module != self.ref_module || trigger.channel != self.ref_channel {
                continue;
            }

            let trig_time = trigger.timestamp_ns;
            let window_start = trig_time - window;
            let window_end = trig_time + window;

            // Scan backwards for past coincidences
            // Note: Using index-based loop to iterate in reverse order efficiently
            #[allow(clippy::needless_range_loop)]
            for i in (0..trig_idx).rev() {
                let hit = &hits[i];
                if hit.timestamp_ns < window_start {
                    break; // Past the window, stop
                }
                let dt = hit.timestamp_ns - trig_time;
                self.fill_histogram(hit.module, hit.channel, dt);
            }

            // Scan forwards for future coincidences
            for hit in hits.iter().skip(trig_idx + 1) {
                if hit.timestamp_ns > window_end {
                    break; // Past the window, stop
                }
                let dt = hit.timestamp_ns - trig_time;
                self.fill_histogram(hit.module, hit.channel, dt);
            }
        }
    }

    /// Fill histogram for a channel
    fn fill_histogram(&mut self, module: u8, channel: u8, dt: f64) {
        let hist = self
            .histograms
            .entry((module, channel))
            .or_insert_with(|| TimeHistogram::new(self.hist_min, self.hist_max, self.bin_width));

        hist.fill(dt);
    }

    /// Calculate time calibration from accumulated histograms
    ///
    /// Returns offsets for each channel. Channels with insufficient statistics
    /// will have offset = 0.0.
    pub fn calculate_calibration(&self) -> TimeCalibration {
        let mut calib = TimeCalibration::new(self.ref_module, self.ref_channel);

        for (&(module, channel), hist) in &self.histograms {
            if hist.entries() < self.min_entries {
                continue; // Not enough statistics
            }

            // Use centroid for better accuracy
            if let Some(peak) = hist.find_peak_centroid(3) {
                // Offset is the peak position
                // Positive offset means channel is ahead of reference
                calib.set_offset(module, channel, peak);
            }
        }

        calib
    }

    /// Get histogram for a specific channel
    pub fn get_histogram(&self, module: u8, channel: u8) -> Option<&TimeHistogram> {
        self.histograms.get(&(module, channel))
    }

    /// Get all channel keys that have histograms
    pub fn channels(&self) -> impl Iterator<Item = &(u8, u8)> {
        self.histograms.keys()
    }

    /// Get reference channel
    pub fn reference(&self) -> (u8, u8) {
        (self.ref_module, self.ref_channel)
    }

    /// Merge another calibrator's histograms into this one
    ///
    /// Both calibrators must have the same reference channel and histogram parameters.
    pub fn merge(&mut self, other: TimeCalibrator) {
        for ((module, channel), other_hist) in other.histograms {
            if let Some(hist) = self.histograms.get_mut(&(module, channel)) {
                hist.merge(&other_hist);
            } else {
                self.histograms.insert((module, channel), other_hist);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(module: u8, channel: u8, ts: f64) -> Hit {
        Hit::new(module, channel, 1000, 500, ts)
    }

    #[test]
    fn test_histogram_new() {
        let hist = TimeHistogram::new(-100.0, 100.0, 2.0);
        assert_eq!(hist.n_bins(), 100);
        assert_eq!(hist.entries(), 0);
    }

    #[test]
    fn test_histogram_fill() {
        let mut hist = TimeHistogram::new(-100.0, 100.0, 2.0);

        hist.fill(0.0);
        hist.fill(1.0);
        hist.fill(-1.0);

        assert_eq!(hist.entries(), 3);
    }

    #[test]
    fn test_histogram_fill_out_of_range() {
        let mut hist = TimeHistogram::new(-100.0, 100.0, 2.0);

        hist.fill(-200.0); // Below range
        hist.fill(200.0); // Above range

        assert_eq!(hist.entries(), 0);
    }

    #[test]
    fn test_histogram_find_peak() {
        let mut hist = TimeHistogram::new(-100.0, 100.0, 2.0);

        // Create a peak at 10 ns
        for _ in 0..100 {
            hist.fill(10.0);
        }
        for _ in 0..10 {
            hist.fill(0.0);
        }

        let peak = hist.find_peak().unwrap();
        // Peak should be in the bin containing 10.0
        assert!((peak - 11.0).abs() < 2.0); // Bin center
    }

    #[test]
    fn test_histogram_find_peak_centroid() {
        let mut hist = TimeHistogram::new(-100.0, 100.0, 2.0);

        // Create a Gaussian-like distribution centered at 10 ns
        for _ in 0..50 {
            hist.fill(8.0);
        }
        for _ in 0..100 {
            hist.fill(10.0);
        }
        for _ in 0..50 {
            hist.fill(12.0);
        }

        let peak = hist.find_peak_centroid(2).unwrap();
        // Should be close to 10.0
        assert!((peak - 10.0).abs() < 2.0);
    }

    #[test]
    fn test_calibrator_new() {
        let cal = TimeCalibrator::new(0, 0, 500.0);
        assert_eq!(cal.reference(), (0, 0));
    }

    #[test]
    fn test_calibrator_process_hits() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        // Create hits: reference trigger at t=1000, detector at t=1010
        let hits = vec![
            make_hit(0, 0, 1000.0), // Reference trigger
            make_hit(0, 1, 1010.0), // Detector: dt = +10 ns
            make_hit(0, 0, 2000.0), // Another trigger
            make_hit(0, 1, 2008.0), // dt = +8 ns
        ];

        cal.process_hits(&hits);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 2);
    }

    #[test]
    fn test_calibrator_calculate_calibration() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_min_entries(5); // Lower threshold for test

        // Create many hits with consistent offset
        let mut hits = Vec::new();
        for i in 0..20 {
            let t = (i * 1000) as f64;
            hits.push(make_hit(0, 0, t)); // Reference trigger
            hits.push(make_hit(0, 1, t + 15.0)); // Ch 1: +15 ns offset
            hits.push(make_hit(0, 2, t - 10.0)); // Ch 2: -10 ns offset
        }

        cal.process_hits(&hits);

        let calib = cal.calculate_calibration();

        // Check offsets
        let offset1 = calib.get_offset(0, 1);
        let offset2 = calib.get_offset(0, 2);

        assert!((offset1 - 15.0).abs() < 2.0, "Ch1 offset: {}", offset1);
        assert!((offset2 - (-10.0)).abs() < 2.0, "Ch2 offset: {}", offset2);
    }

    #[test]
    fn test_calibrator_insufficient_statistics() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_min_entries(100); // High threshold

        // Only a few hits
        let hits = vec![make_hit(0, 0, 1000.0), make_hit(0, 1, 1010.0)];

        cal.process_hits(&hits);

        let calib = cal.calculate_calibration();

        // Offset should be 0 (default) due to insufficient stats
        assert_eq!(calib.get_offset(0, 1), 0.0);
    }

    #[test]
    fn test_calibrator_window() {
        let mut cal = TimeCalibrator::new(0, 0, 100.0); // ±100 ns window
        cal.set_min_entries(1);

        let hits = vec![
            make_hit(0, 0, 1000.0), // Reference trigger
            make_hit(0, 1, 1050.0), // Within window: +50 ns
            make_hit(0, 2, 1200.0), // Outside window: +200 ns
        ];

        cal.process_hits(&hits);

        // Ch 1 should have histogram entry
        assert!(cal.get_histogram(0, 1).is_some());
        assert_eq!(cal.get_histogram(0, 1).unwrap().entries(), 1);

        // Ch 2 should have no entries (outside window)
        let ch2_hist = cal.get_histogram(0, 2);
        assert!(ch2_hist.is_none() || ch2_hist.unwrap().entries() == 0);
    }

    #[test]
    fn test_histogram_bin_center() {
        let hist = TimeHistogram::new(-100.0, 100.0, 10.0);

        // First bin: [-100, -90), center = -95
        assert!((hist.bin_center(0) - (-95.0)).abs() < 0.01);

        // Bin at 0: [0, 10), center = 5
        // Bin index for 0 = (0 - (-100)) / 10 = 10
        assert!((hist.bin_center(10) - 5.0).abs() < 0.01);
    }
}

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

    /// Check if the histogram has a statistically significant peak
    ///
    /// Computes peak height vs background average (excluding peak region).
    /// Returns Some((peak_centroid, significance)) if significant, None otherwise.
    ///
    /// Significance = peak_count / sqrt(background_mean) (Poisson approximation).
    /// A significance > 5 is roughly equivalent to 5-sigma.
    pub fn peak_significance(&self, window_bins: usize, min_sigma: f64) -> Option<(f64, f64)> {
        if self.entries == 0 {
            return None;
        }

        // Find peak bin
        let (max_bin, &max_count) = self
            .bins
            .iter()
            .enumerate()
            .max_by_key(|(_, &count)| count)?;

        if max_count == 0 {
            return None;
        }

        // Calculate background: average of bins excluding peak region
        let peak_start = max_bin.saturating_sub(window_bins);
        let peak_end = (max_bin + window_bins + 1).min(self.bins.len());
        let n_peak_bins = peak_end - peak_start;

        let bg_bins = self.bins.len() - n_peak_bins;
        if bg_bins == 0 {
            return None;
        }

        let bg_sum: u64 =
            self.bins[..peak_start].iter().sum::<u64>() + self.bins[peak_end..].iter().sum::<u64>();
        let bg_mean = bg_sum as f64 / bg_bins as f64;

        // Significance: excess counts over background, normalized by Poisson sigma
        // For Poisson: sigma = sqrt(expected), expected = bg_mean for peak bin
        let sigma = if bg_mean > 0.0 {
            (max_count as f64 - bg_mean) / bg_mean.sqrt()
        } else {
            // Zero background — any peak is infinitely significant
            if max_count > 0 {
                f64::INFINITY
            } else {
                0.0
            }
        };

        if sigma >= min_sigma {
            self.find_peak_centroid(window_bins).map(|c| (c, sigma))
        } else {
            None
        }
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
    /// Reference energy range [min, max] (inclusive, 16-bit ADC units)
    ref_energy_min: u16,
    ref_energy_max: u16,
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
            ref_energy_min: 0,
            ref_energy_max: u16::MAX,
        }
    }

    /// Set energy range for reference trigger acceptance [min, max] inclusive.
    ///
    /// Only hits on the reference channel with energy in this range are treated
    /// as triggers. Useful for selecting specific peaks (e.g. Co-60 double peak).
    /// Default: 0..=65535 (accept all).
    pub fn set_ref_energy_range(&mut self, min: u16, max: u16) {
        self.ref_energy_min = min;
        self.ref_energy_max = max;
    }

    /// Check if a hit qualifies as a reference trigger (channel + energy gate).
    #[inline]
    fn is_ref_trigger(&self, hit: &Hit) -> bool {
        hit.module == self.ref_module
            && hit.channel == self.ref_channel
            && hit.energy >= self.ref_energy_min
            && hit.energy <= self.ref_energy_max
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
        // Find all reference triggers (with energy gate)
        let triggers: Vec<usize> = hits
            .iter()
            .enumerate()
            .filter(|(_, h)| self.is_ref_trigger(h))
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
            // Check if this is a reference trigger (with energy gate)
            if !self.is_ref_trigger(trigger) {
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
    /// Returns offsets for ALL channels that have histograms.
    /// Two-tier acceptance:
    ///   1. entries >= min_entries → use centroid
    ///   2. entries < min_entries but peak significance >= 5σ → use centroid (low-stats rescue)
    ///      Otherwise → offset = 0.0
    pub fn calculate_calibration(&self) -> TimeCalibration {
        const PEAK_SIGMA_THRESHOLD: f64 = 5.0;
        let mut calib = TimeCalibration::new(self.ref_module, self.ref_channel);

        for (&(module, channel), hist) in &self.histograms {
            if hist.entries() >= self.min_entries {
                // Tier 1: enough statistics
                if let Some(peak) = hist.find_peak_centroid(3) {
                    calib.set_offset(module, channel, peak);
                } else {
                    calib.set_offset(module, channel, 0.0);
                }
            } else if let Some((peak, sigma)) = hist.peak_significance(3, PEAK_SIGMA_THRESHOLD) {
                // Tier 2: low stats but significant peak (≥5σ over background)
                tracing::info!(
                    "Ch({}, {}): low stats ({} entries) but clear peak at {:.1} ns ({:.1}σ) — accepted",
                    module, channel, hist.entries(), peak, sigma
                );
                calib.set_offset(module, channel, peak);
            } else {
                // No significant peak
                calib.set_offset(module, channel, 0.0);
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

    /// Process hits using trigger-index method (no pre-sorting required).
    ///
    /// **Phase 1**: Collects trigger timestamps, sorts only triggers O(t log t),
    /// then binary-searches per detector hit O(n log t).
    /// Much faster than sort-all + `process_hits_sorted` when triggers << total hits.
    ///
    /// Use `process_blocks_streaming` for even better performance on block-structured data.
    pub fn process_hits_by_trigger_index(&mut self, hits: &[Hit]) {
        // Collect trigger timestamps only (with energy gate, typically 1-5% of all hits)
        let mut trigger_times: Vec<f64> = hits
            .iter()
            .filter(|h| self.is_ref_trigger(h))
            .map(|h| h.timestamp_ns)
            .collect();

        if trigger_times.is_empty() {
            return;
        }

        trigger_times.sort_unstable_by(|a, b| a.total_cmp(b));

        let window = self.window_ns;

        for hit in hits {
            // Skip ref channel hits (regardless of energy gate)
            if hit.module == self.ref_module && hit.channel == self.ref_channel {
                continue;
            }

            let min_trig = hit.timestamp_ns - window;
            let max_trig = hit.timestamp_ns + window;

            // Binary search: first trigger >= (hit_time - window)
            let start = trigger_times.partition_point(|&t| t < min_trig);

            // Linear scan within window
            for &trig_ts in &trigger_times[start..] {
                if trig_ts > max_trig {
                    break;
                }
                self.fill_histogram(hit.module, hit.channel, hit.timestamp_ns - trig_ts);
            }
        }
    }

    /// Process block-structured data with streaming stateful scanner.
    ///
    /// **Phase 2**: Takes pre-sorted trigger timestamps and a slice of hits
    /// from a single data block (roughly time-ordered within the block).
    /// Uses a stateful linear scan instead of binary search per hit,
    /// achieving amortized O(1) per hit within each block.
    ///
    /// Call this once per data block after collecting and sorting all trigger timestamps.
    pub fn process_block_with_sorted_triggers(&mut self, sorted_triggers: &[f64], block: &[Hit]) {
        if sorted_triggers.is_empty() || block.is_empty() {
            return;
        }

        let window = self.window_ns;

        // Binary search once for the first non-ref hit in the block
        let first_ts = block
            .iter()
            .find(|h| !(h.module == self.ref_module && h.channel == self.ref_channel))
            .map(|h| h.timestamp_ns);

        let mut scan_start = match first_ts {
            Some(ts) => sorted_triggers.partition_point(|&t| t < ts - window),
            None => return, // Block contains only ref channel hits
        };

        for hit in block {
            // Skip ref channel hits (regardless of energy gate — triggers already extracted)
            if hit.module == self.ref_module && hit.channel == self.ref_channel {
                continue;
            }

            let min_trig = hit.timestamp_ns - window;
            let max_trig = hit.timestamp_ns + window;

            // Advance scan_start linearly (hits within block are roughly time-ordered)
            while scan_start < sorted_triggers.len() && sorted_triggers[scan_start] < min_trig {
                scan_start += 1;
            }

            // Handle backward jumps: if scan_start is past our window, re-search
            // (can happen when blocks contain hits from different time regions)
            if scan_start < sorted_triggers.len() && sorted_triggers[scan_start] > max_trig {
                // Fallback to binary search for this hit
                let bs = sorted_triggers.partition_point(|&t| t < min_trig);
                for &trig_ts in &sorted_triggers[bs..] {
                    if trig_ts > max_trig {
                        break;
                    }
                    self.fill_histogram(hit.module, hit.channel, hit.timestamp_ns - trig_ts);
                }
                continue;
            }

            // We may have overshot — temporarily back up if needed
            // (scan_start points to first trigger >= min_trig, which is correct)
            for &trig_ts in &sorted_triggers[scan_start..] {
                if trig_ts > max_trig {
                    break;
                }
                self.fill_histogram(hit.module, hit.channel, hit.timestamp_ns - trig_ts);
            }
        }
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
    fn test_calibrator_low_stats_significant_peak() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_min_entries(100); // High threshold

        // Only a few hits but clear peak (zero background → infinite significance)
        let hits = vec![make_hit(0, 0, 1000.0), make_hit(0, 1, 1010.0)];

        cal.process_hits(&hits);

        let calib = cal.calculate_calibration();

        // Low stats but significant peak → rescued (offset ≈ 10 ns)
        let offset = calib.get_offset(0, 1);
        assert!(
            (offset - 10.0).abs() < 2.0,
            "Low-stats rescue should accept clear peak: {}",
            offset
        );
    }

    #[test]
    fn test_calibrator_no_significant_peak() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_min_entries(1000); // High threshold: 50 entries won't pass Tier 1

        // 1 trigger + 50 detectors at unique dt values → flat distribution (50 < 1000)
        // bg_mean ≈ 0.05, peak = 1 → significance ≈ 4.3 < 5σ → Tier 2 also fails
        let mut hits = vec![make_hit(0, 0, 1000.0)];
        for i in 0..50 {
            hits.push(make_hit(0, 1, 1000.0 + (i as f64 * 10.0 - 250.0)));
        }

        cal.process_hits(&hits);

        let calib = cal.calculate_calibration();

        // Low stats + no significant peak → offset = 0
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

    // --- Phase 1: Trigger-Index tests ---

    #[test]
    fn test_trigger_index_basic() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let hits = vec![
            make_hit(0, 0, 1000.0), // trigger
            make_hit(0, 1, 1010.0), // dt = +10
            make_hit(0, 0, 2000.0), // trigger
            make_hit(0, 1, 2008.0), // dt = +8
        ];

        cal.process_hits_by_trigger_index(&hits);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 2);
    }

    #[test]
    fn test_trigger_index_unsorted_hits() {
        // Hits are NOT sorted by timestamp — trigger-index should still work
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let hits = vec![
            make_hit(0, 1, 2008.0), // detector (before its trigger in array)
            make_hit(0, 0, 2000.0), // trigger
            make_hit(0, 1, 1010.0), // detector
            make_hit(0, 0, 1000.0), // trigger
        ];

        cal.process_hits_by_trigger_index(&hits);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 2);
    }

    #[test]
    fn test_trigger_index_matches_sorted() {
        // Verify trigger-index produces same results as sort + process_hits_sorted
        let mut hits = Vec::new();
        for i in 0..50 {
            let t = (i * 1000) as f64;
            hits.push(make_hit(0, 0, t));
            hits.push(make_hit(0, 1, t + 15.0));
            hits.push(make_hit(1, 0, t + 42.0));
        }

        // Method 1: sort + process_hits_sorted
        let mut cal_sorted = TimeCalibrator::new(0, 0, 500.0);
        let mut sorted_hits = hits.clone();
        sorted_hits.sort_unstable_by(|a, b| a.timestamp_ns.total_cmp(&b.timestamp_ns));
        cal_sorted.process_hits_sorted(&sorted_hits);

        // Method 2: trigger-index (no sort)
        let mut cal_idx = TimeCalibrator::new(0, 0, 500.0);
        cal_idx.process_hits_by_trigger_index(&hits);

        // Compare histogram entries for each channel
        let h1_sorted = cal_sorted.get_histogram(0, 1).unwrap();
        let h1_idx = cal_idx.get_histogram(0, 1).unwrap();
        assert_eq!(h1_sorted.entries(), h1_idx.entries());

        let h2_sorted = cal_sorted.get_histogram(1, 0).unwrap();
        let h2_idx = cal_idx.get_histogram(1, 0).unwrap();
        assert_eq!(h2_sorted.entries(), h2_idx.entries());
    }

    #[test]
    fn test_trigger_index_window() {
        let mut cal = TimeCalibrator::new(0, 0, 100.0); // ±100 ns window

        let hits = vec![
            make_hit(0, 0, 1000.0), // trigger
            make_hit(0, 1, 1050.0), // within window: +50
            make_hit(0, 2, 1200.0), // outside window: +200
        ];

        cal.process_hits_by_trigger_index(&hits);

        assert_eq!(cal.get_histogram(0, 1).unwrap().entries(), 1);
        assert!(
            cal.get_histogram(0, 2).is_none() || cal.get_histogram(0, 2).unwrap().entries() == 0
        );
    }

    #[test]
    fn test_trigger_index_no_triggers() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let hits = vec![make_hit(0, 1, 1010.0), make_hit(0, 2, 1020.0)];

        cal.process_hits_by_trigger_index(&hits);
        // No triggers → no histograms
        assert!(cal.get_histogram(0, 1).is_none());
    }

    // --- Phase 2: Block streaming tests ---

    #[test]
    fn test_block_streaming_basic() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let triggers = vec![1000.0, 2000.0, 3000.0];
        let block = vec![
            make_hit(0, 1, 1010.0), // dt = +10 from trigger 1000
            make_hit(0, 1, 2015.0), // dt = +15 from trigger 2000
        ];

        cal.process_block_with_sorted_triggers(&triggers, &block);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 2);
    }

    #[test]
    fn test_block_streaming_multiple_blocks() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let triggers = vec![1000.0, 2000.0, 3000.0];

        // Block from digitizer A (time-ordered within block)
        let block_a = vec![make_hit(0, 1, 1010.0), make_hit(0, 1, 1020.0)];
        // Block from digitizer B (different time range)
        let block_b = vec![make_hit(1, 0, 2005.0), make_hit(1, 0, 2050.0)];

        cal.process_block_with_sorted_triggers(&triggers, &block_a);
        cal.process_block_with_sorted_triggers(&triggers, &block_b);

        assert_eq!(cal.get_histogram(0, 1).unwrap().entries(), 2);
        assert_eq!(cal.get_histogram(1, 0).unwrap().entries(), 2);
    }

    #[test]
    fn test_block_streaming_matches_trigger_index() {
        // Verify block-streaming produces same results as trigger-index method
        let mut hits = Vec::new();
        for i in 0..50 {
            let t = (i * 1000) as f64;
            hits.push(make_hit(0, 0, t));
            hits.push(make_hit(0, 1, t + 15.0));
            hits.push(make_hit(1, 0, t + 42.0));
        }

        // Method 1: trigger-index
        let mut cal_idx = TimeCalibrator::new(0, 0, 500.0);
        cal_idx.process_hits_by_trigger_index(&hits);

        // Method 2: block-streaming (simulate blocks of 10 hits)
        let mut cal_blk = TimeCalibrator::new(0, 0, 500.0);
        let trigger_times: Vec<f64> = hits
            .iter()
            .filter(|h| h.module == 0 && h.channel == 0)
            .map(|h| h.timestamp_ns)
            .collect();
        // Already sorted since generated in order

        // Split non-trigger hits into blocks
        let det_hits: Vec<Hit> = hits
            .iter()
            .filter(|h| !(h.module == 0 && h.channel == 0))
            .cloned()
            .collect();
        for chunk in det_hits.chunks(10) {
            cal_blk.process_block_with_sorted_triggers(&trigger_times, chunk);
        }

        // Compare
        let h1_idx = cal_idx.get_histogram(0, 1).unwrap();
        let h1_blk = cal_blk.get_histogram(0, 1).unwrap();
        assert_eq!(h1_idx.entries(), h1_blk.entries());

        let h2_idx = cal_idx.get_histogram(1, 0).unwrap();
        let h2_blk = cal_blk.get_histogram(1, 0).unwrap();
        assert_eq!(h2_idx.entries(), h2_blk.entries());
    }

    #[test]
    fn test_block_streaming_backward_jump() {
        // Test that the fallback binary search handles non-monotonic blocks
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let triggers = vec![1000.0, 5000.0, 10000.0];

        // Block with a backward time jump (different digitizer interleaved)
        let block = vec![
            make_hit(0, 1, 10010.0), // near trigger 10000
            make_hit(0, 1, 1010.0),  // jump back near trigger 1000
        ];

        cal.process_block_with_sorted_triggers(&triggers, &block);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 2);
    }

    #[test]
    fn test_block_streaming_empty() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        // Empty triggers
        cal.process_block_with_sorted_triggers(&[], &[make_hit(0, 1, 1000.0)]);
        assert!(cal.get_histogram(0, 1).is_none());

        // Empty block
        cal.process_block_with_sorted_triggers(&[1000.0], &[]);
        assert!(cal.get_histogram(0, 1).is_none());
    }

    #[test]
    fn test_block_streaming_trigger_only_block() {
        // Block contains only trigger channel hits → should be skipped gracefully
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let triggers = vec![1000.0, 2000.0];
        let block = vec![
            make_hit(0, 0, 1000.0), // trigger channel hit
            make_hit(0, 0, 2000.0), // trigger channel hit
        ];

        cal.process_block_with_sorted_triggers(&triggers, &block);
        // No detector histograms should be created
        assert_eq!(cal.channels().count(), 0);
    }

    // --- Energy gate tests ---

    fn make_hit_energy(module: u8, channel: u8, ts: f64, energy: u16) -> Hit {
        Hit::new(module, channel, energy, 0, ts)
    }

    #[test]
    fn test_energy_gate_trigger_index() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_ref_energy_range(1000, 2000); // Only accept triggers with energy 1000-2000

        let hits = vec![
            make_hit_energy(0, 0, 1000.0, 1500), // trigger: energy in range → accepted
            make_hit_energy(0, 0, 2000.0, 500),  // trigger: energy below range → rejected
            make_hit_energy(0, 0, 3000.0, 3000), // trigger: energy above range → rejected
            make_hit_energy(0, 1, 1010.0, 800),  // detector
            make_hit_energy(0, 1, 2010.0, 800),  // detector (no matching trigger)
            make_hit_energy(0, 1, 3010.0, 800),  // detector (no matching trigger)
        ];

        cal.process_hits_by_trigger_index(&hits);

        let hist = cal.get_histogram(0, 1).unwrap();
        // Only 1 trigger accepted (energy=1500) → only hit at 1010 matches
        assert_eq!(hist.entries(), 1);
    }

    #[test]
    fn test_energy_gate_process_hits() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_ref_energy_range(1000, 2000);

        let hits = vec![
            make_hit_energy(0, 0, 1000.0, 1500), // accepted trigger
            make_hit_energy(0, 0, 2000.0, 500),  // rejected trigger
            make_hit_energy(0, 1, 1010.0, 800),
            make_hit_energy(0, 1, 2010.0, 800),
        ];

        cal.process_hits(&hits);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 1);
    }

    #[test]
    fn test_energy_gate_process_hits_sorted() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_ref_energy_range(1000, 2000);

        let hits = vec![
            make_hit_energy(0, 0, 1000.0, 1500), // accepted
            make_hit_energy(0, 1, 1010.0, 800),
            make_hit_energy(0, 0, 2000.0, 500), // rejected
            make_hit_energy(0, 1, 2010.0, 800),
        ];

        cal.process_hits_sorted(&hits);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 1);
    }

    #[test]
    fn test_energy_gate_default_accepts_all() {
        // Default energy range (0..65535) should accept all
        let mut cal = TimeCalibrator::new(0, 0, 500.0);

        let hits = vec![
            make_hit_energy(0, 0, 1000.0, 0),     // min energy
            make_hit_energy(0, 0, 2000.0, 65535), // max energy
            make_hit_energy(0, 1, 1010.0, 800),
            make_hit_energy(0, 1, 2010.0, 800),
        ];

        cal.process_hits_by_trigger_index(&hits);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 2); // Both triggers accepted
    }

    #[test]
    fn test_energy_gate_block_streaming() {
        let mut cal = TimeCalibrator::new(0, 0, 500.0);
        cal.set_ref_energy_range(1000, 2000);

        // Only trigger at t=1000 (energy=1500) passes the gate
        // Note: trigger filtering happens at collection time in event_builder.rs,
        // but the calibrator's is_ref_trigger also gates in process_hits* methods.
        // For block streaming, triggers are pre-collected, so this test
        // verifies the block processing itself works correctly with gated triggers.
        let triggers = vec![1000.0]; // Only the accepted trigger
        let block = vec![
            make_hit_energy(0, 1, 1010.0, 800),
            make_hit_energy(0, 1, 5010.0, 800), // far from trigger
        ];

        cal.process_block_with_sorted_triggers(&triggers, &block);

        let hist = cal.get_histogram(0, 1).unwrap();
        assert_eq!(hist.entries(), 1); // Only 1010 is within window of trigger 1000
    }
}

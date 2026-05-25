//! Slice Builder - Time Slice based event building
//!
//! Time Slice 方式でイベントを構築する。
//! 並列処理が可能で、メモリ使用量が予測可能。

use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use super::built_event::{BuiltEvent, EventHit};
use super::config::TimeCalibration;
use super::hit::Hit;
use super::time_slice::{create_slices, TimeSlice};

/// Slice-based event builder
///
/// Time Slice 方式でイベントを構築する。
/// 各スライスは独立して処理可能なため、rayon による並列化が容易。
pub struct SliceBuilder {
    /// Slice duration [ns] (default: 10 ms)
    slice_duration_ns: f64,
    /// Coincidence window [ns] (also used as overlap)
    coincidence_window_ns: f64,
    /// Trigger channels: (module, channel) -> priority (lower = higher)
    trigger_channels: HashMap<(u8, u8), u32>,
    /// AC pair mapping: detector (mod, ch) -> AC (mod, ch)
    ac_pairs: HashMap<(u8, u8), (u8, u8)>,
    /// Time calibration offsets
    time_calibration: TimeCalibration,
    /// Event ID counter (atomic for thread safety)
    next_event_id: AtomicU64,
}

impl SliceBuilder {
    /// Create a new slice builder
    ///
    /// # Arguments
    /// * `slice_duration_ns` - Duration of each time slice [ns]
    /// * `coincidence_window_ns` - Coincidence window [ns]
    pub fn new(slice_duration_ns: f64, coincidence_window_ns: f64) -> Self {
        Self {
            slice_duration_ns,
            coincidence_window_ns,
            trigger_channels: HashMap::new(),
            ac_pairs: HashMap::new(),
            time_calibration: TimeCalibration::default(),
            next_event_id: AtomicU64::new(0),
        }
    }

    /// Create with default parameters (10ms slice, 500ns window)
    pub fn default_params() -> Self {
        Self::new(10_000_000.0, 500.0) // 10 ms, 500 ns
    }

    /// Add a trigger channel
    pub fn add_trigger(&mut self, module: u8, channel: u8, priority: u32) {
        self.trigger_channels.insert((module, channel), priority);
    }

    /// Add an AC pair mapping
    pub fn add_ac_pair(&mut self, detector_mod: u8, detector_ch: u8, ac_mod: u8, ac_ch: u8) {
        self.ac_pairs
            .insert((detector_mod, detector_ch), (ac_mod, ac_ch));
    }

    /// Set time calibration
    pub fn set_time_calibration(&mut self, calib: TimeCalibration) {
        self.time_calibration = calib;
    }

    /// Check if a hit is a trigger
    #[inline]
    pub fn is_trigger(&self, hit: &Hit) -> bool {
        self.trigger_channels
            .contains_key(&(hit.module, hit.channel))
    }

    /// Get trigger priority (lower = higher priority)
    #[inline]
    pub fn get_priority(&self, hit: &Hit) -> u32 {
        self.trigger_channels
            .get(&(hit.module, hit.channel))
            .copied()
            .unwrap_or(u32::MAX)
    }

    /// Get coincidence window
    #[inline]
    pub fn coincidence_window_ns(&self) -> f64 {
        self.coincidence_window_ns
    }

    /// Get AC pair for a channel, if any
    #[inline]
    pub fn get_ac_pair(&self, module: u8, channel: u8) -> Option<&(u8, u8)> {
        self.ac_pairs.get(&(module, channel))
    }

    /// Apply time calibration to hits
    pub fn apply_calibration(&self, hits: &mut [Hit]) {
        for hit in hits.iter_mut() {
            let offset = self.time_calibration.get_offset(hit.module, hit.channel);
            hit.timestamp_ns -= offset;
        }
    }

    /// Build events from hits using Time Slice method (parallel)
    ///
    /// # Arguments
    /// * `hits` - Input hits (will be sorted and calibrated)
    ///
    /// # Returns
    /// Vector of built events
    pub fn build_events(&self, hits: Vec<Hit>) -> Vec<BuiltEvent> {
        if hits.is_empty() {
            return Vec::new();
        }

        // Apply calibration and sort
        let mut calibrated_hits = hits;
        self.apply_calibration(&mut calibrated_hits);
        calibrated_hits.sort_by(|a, b| {
            a.timestamp_ns
                .partial_cmp(&b.timestamp_ns)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Create time slices
        let slices = create_slices(
            &calibrated_hits,
            self.slice_duration_ns,
            self.coincidence_window_ns,
        );

        // Process slices in parallel
        let events: Vec<Vec<BuiltEvent>> = slices
            .par_iter()
            .map(|slice| self.process_slice(slice))
            .collect();

        // Flatten and assign event IDs
        let mut all_events: Vec<BuiltEvent> = events.into_iter().flatten().collect();

        // Sort by trigger time and assign sequential IDs
        all_events.sort_by(|a, b| {
            a.trigger_time
                .partial_cmp(&b.trigger_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for event in all_events.iter_mut() {
            event.event_id = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        }

        all_events
    }

    /// Process a single time slice
    ///
    /// Only triggers in the core region are processed.
    /// Triggers in the overlap region are skipped (processed in next slice).
    fn process_slice(&self, slice: &TimeSlice) -> Vec<BuiltEvent> {
        let mut events = Vec::new();

        for (idx, hit) in slice.hits.iter().enumerate() {
            // Skip non-trigger hits
            if !self.is_trigger(hit) {
                continue;
            }

            // Skip triggers in overlap region (processed in next slice)
            if slice.is_in_overlap(hit.timestamp_ns) {
                continue;
            }

            // Check for prior trigger in coincidence window (time priority)
            if self.has_prior_trigger(slice, idx) {
                continue;
            }

            // Build event from this trigger
            if let Some(event) = self.build_event_from_trigger(slice, idx) {
                events.push(event);
            }
        }

        events
    }

    /// Check if there's a prior trigger within coincidence window
    fn has_prior_trigger(&self, slice: &TimeSlice, trigger_idx: usize) -> bool {
        let trigger = &slice.hits[trigger_idx];
        let trigger_time = trigger.timestamp_ns;
        let trigger_priority = self.get_priority(trigger);
        let window_start = trigger_time - self.coincidence_window_ns;

        // Search backwards for prior triggers
        for i in (0..trigger_idx).rev() {
            let hit = &slice.hits[i];

            // Stop if outside window
            if hit.timestamp_ns < window_start {
                break;
            }

            // Check if this is a trigger
            if self.is_trigger(hit) {
                let other_priority = self.get_priority(hit);

                // Prior trigger found: skip if it has higher or equal priority
                // (lower priority value = higher priority)
                if other_priority <= trigger_priority {
                    return true;
                }
            }
        }

        false
    }

    /// Build an event from a trigger hit
    fn build_event_from_trigger(
        &self,
        slice: &TimeSlice,
        trigger_idx: usize,
    ) -> Option<BuiltEvent> {
        let trigger = &slice.hits[trigger_idx];
        let trigger_time = trigger.timestamp_ns;
        let window_start = trigger_time - self.coincidence_window_ns;
        let window_end = trigger_time + self.coincidence_window_ns;

        // Collect AC hits for with_ac determination
        let ac_hits: HashMap<(u8, u8), bool> = slice
            .hits
            .iter()
            .filter(|h| h.timestamp_ns >= window_start && h.timestamp_ns <= window_end)
            .map(|h| ((h.module, h.channel), true))
            .collect();

        // Collect coincident hits
        let mut event_hits = Vec::new();

        for hit in slice.hits.iter() {
            // Check if within coincidence window
            if hit.timestamp_ns < window_start {
                continue;
            }
            if hit.timestamp_ns > window_end {
                break; // Hits are time-sorted
            }

            // Determine with_ac flag
            let with_ac = self
                .ac_pairs
                .get(&(hit.module, hit.channel))
                .map(|ac| ac_hits.contains_key(ac))
                .unwrap_or(false);

            event_hits.push(EventHit {
                module: hit.module,
                channel: hit.channel,
                energy: hit.energy,
                energy_short: hit.energy_short,
                relative_time: hit.timestamp_ns - trigger_time,
                with_ac,
            });
        }

        if event_hits.is_empty() {
            return None;
        }

        Some(BuiltEvent {
            event_id: 0, // Assigned later
            trigger_time,
            trigger_module: trigger.module,
            trigger_channel: trigger.channel,
            hits: event_hits,
            counters: std::collections::HashMap::new(),
        })
    }

    /// Get statistics about the builder
    pub fn stats(&self) -> SliceBuilderStats {
        SliceBuilderStats {
            slice_duration_ns: self.slice_duration_ns,
            coincidence_window_ns: self.coincidence_window_ns,
            n_triggers: self.trigger_channels.len(),
            n_ac_pairs: self.ac_pairs.len(),
            events_built: self.next_event_id.load(Ordering::Relaxed),
        }
    }
}

/// Statistics from SliceBuilder
#[derive(Debug, Clone)]
pub struct SliceBuilderStats {
    pub slice_duration_ns: f64,
    pub coincidence_window_ns: f64,
    pub n_triggers: usize,
    pub n_ac_pairs: usize,
    pub events_built: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(module: u8, channel: u8, ts: f64) -> Hit {
        Hit::new(module, channel, 1000, 500, ts)
    }

    #[test]
    fn test_slice_builder_new() {
        let builder = SliceBuilder::new(10_000_000.0, 500.0);
        assert_eq!(builder.slice_duration_ns, 10_000_000.0);
        assert_eq!(builder.coincidence_window_ns, 500.0);
    }

    #[test]
    fn test_add_trigger() {
        let mut builder = SliceBuilder::new(10_000_000.0, 500.0);
        builder.add_trigger(0, 0, 0);
        builder.add_trigger(1, 2, 1);

        assert!(builder.is_trigger(&make_hit(0, 0, 100.0)));
        assert!(builder.is_trigger(&make_hit(1, 2, 100.0)));
        assert!(!builder.is_trigger(&make_hit(2, 0, 100.0)));
    }

    #[test]
    fn test_build_events_empty() {
        let builder = SliceBuilder::new(10_000_000.0, 500.0);
        let events = builder.build_events(vec![]);
        assert!(events.is_empty());
    }

    #[test]
    fn test_build_events_no_trigger() {
        let mut builder = SliceBuilder::new(10_000_000.0, 500.0);
        builder.add_trigger(0, 0, 0); // Only (0,0) is trigger

        let hits = vec![
            make_hit(1, 0, 100.0), // Not a trigger
            make_hit(1, 1, 200.0), // Not a trigger
        ];

        let events = builder.build_events(hits);
        assert!(events.is_empty());
    }

    #[test]
    fn test_build_events_single_trigger() {
        let mut builder = SliceBuilder::new(10_000_000.0, 500.0);
        builder.add_trigger(0, 0, 0);

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger
            make_hit(1, 0, 1100.0), // Coincident
            make_hit(1, 1, 1200.0), // Coincident
        ];

        let events = builder.build_events(hits);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger_module, 0);
        assert_eq!(events[0].trigger_channel, 0);
        assert_eq!(events[0].hits.len(), 3); // Trigger + 2 coincident
    }

    #[test]
    fn test_build_events_multiple_triggers() {
        let mut builder = SliceBuilder::new(10_000_000.0, 500.0);
        builder.add_trigger(0, 0, 0);

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger 1
            make_hit(1, 0, 1100.0), // Coincident with T1
            make_hit(0, 0, 5000.0), // Trigger 2 (well separated)
            make_hit(1, 1, 5100.0), // Coincident with T2
        ];

        let events = builder.build_events(hits);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_prior_trigger_skip() {
        let mut builder = SliceBuilder::new(10_000_000.0, 500.0);
        builder.add_trigger(0, 0, 0); // Priority 0 (highest)
        builder.add_trigger(0, 1, 1); // Priority 1 (lower)

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger (0,0) - highest priority
            make_hit(0, 1, 1200.0), // Trigger (0,1) - within window, lower priority -> skip
            make_hit(1, 0, 1100.0), // Coincident
        ];

        let events = builder.build_events(hits);

        // Only one event should be built (from highest priority trigger)
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger_module, 0);
        assert_eq!(events[0].trigger_channel, 0);
    }

    #[test]
    fn test_time_calibration_applied() {
        let mut builder = SliceBuilder::new(10_000_000.0, 500.0);
        builder.add_trigger(0, 0, 0);

        // Add calibration offset: channel (1,0) has +100ns offset
        let mut calib = TimeCalibration::new(0, 0);
        calib.set_offset(1, 0, 100.0);
        builder.set_time_calibration(calib);

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger at 1000ns
            make_hit(1, 0, 1150.0), // At 1150ns, after calibration: 1050ns
        ];

        let events = builder.build_events(hits);
        assert_eq!(events.len(), 1);

        // Check that the relative time reflects calibration
        // Original: 1150 - 1000 = 150ns
        // After calibration: (1150 - 100) - 1000 = 50ns
        let hit_1_0 = events[0]
            .hits
            .iter()
            .find(|h| h.module == 1 && h.channel == 0)
            .unwrap();
        assert!((hit_1_0.relative_time - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_ac_pair_detection() {
        let mut builder = SliceBuilder::new(10_000_000.0, 500.0);
        builder.add_trigger(0, 0, 0);
        builder.add_ac_pair(0, 0, 0, 1); // (0,0) has AC at (0,1)

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger (has AC pair)
            make_hit(0, 1, 1050.0), // AC hit (coincident)
        ];

        let events = builder.build_events(hits);
        assert_eq!(events.len(), 1);

        // Trigger hit should have with_ac = true
        let trigger_hit = events[0]
            .hits
            .iter()
            .find(|h| h.module == 0 && h.channel == 0)
            .unwrap();
        assert!(trigger_hit.with_ac);
    }

    #[test]
    fn test_parallel_consistency() {
        // Test that parallel processing gives consistent results
        let mut builder = SliceBuilder::new(1_000_000.0, 500.0); // 1ms slices
        builder.add_trigger(0, 0, 0);

        // Create many hits across multiple slices
        let mut hits = Vec::new();
        for i in 0..100 {
            let base_time = (i * 10000) as f64; // 10us apart
            hits.push(make_hit(0, 0, base_time)); // Trigger
            hits.push(make_hit(1, 0, base_time + 100.0)); // Coincident
        }

        let events = builder.build_events(hits);

        // Should have 100 events
        assert_eq!(events.len(), 100);

        // Event IDs should be sequential
        for (i, event) in events.iter().enumerate() {
            assert_eq!(event.event_id, i as u64);
        }
    }
}

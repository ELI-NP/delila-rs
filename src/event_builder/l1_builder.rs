//! L1 Event Builder - Coincidence detection
//!
//! Trigger ベースのコインシデンス検出。
//! ELIFANT-Event の L1EventBuilder に相当。

use super::built_event::BuiltEvent;
use super::config::{ChSettings, TimeCalibration};
use super::hit::Hit;
use super::time_sort::TimeSortBuffer;
use std::collections::{HashMap, VecDeque};

/// L1 Event Builder
///
/// Builds events from time-sorted hits using trigger-based coincidence detection.
pub struct L1Builder {
    /// Coincidence window in nanoseconds (±window from trigger)
    coincidence_window_ns: f64,
    /// Buffer delay for time sorting (stored for potential reconfiguration)
    #[allow(dead_code)]
    buffer_delay_ns: f64,
    /// Trigger channels: (module, channel) -> priority (lower = higher priority)
    trigger_channels: HashMap<(u8, u8), u32>,
    /// AC pair mapping: detector (mod, ch) -> AC (mod, ch)
    ac_pairs: HashMap<(u8, u8), (u8, u8)>,
    /// Time calibration offsets
    time_calibration: TimeCalibration,
    /// Time-sorted buffer
    time_buffer: TimeSortBuffer,
    /// Pending hits waiting for event building
    pending_hits: VecDeque<Hit>,
    /// Current event ID counter
    event_id: u64,
}

impl L1Builder {
    /// Create a new L1 builder
    pub fn new(coincidence_window_ns: f64, buffer_delay_ns: f64) -> Self {
        Self {
            coincidence_window_ns,
            buffer_delay_ns,
            trigger_channels: HashMap::new(),
            ac_pairs: HashMap::new(),
            time_calibration: TimeCalibration::default(),
            time_buffer: TimeSortBuffer::new(buffer_delay_ns),
            pending_hits: VecDeque::new(),
            event_id: 0,
        }
    }

    /// Add a trigger channel with priority
    ///
    /// Lower priority value = higher priority (0 is highest)
    pub fn add_trigger(&mut self, module: u8, channel: u8, priority: u32) {
        self.trigger_channels.insert((module, channel), priority);
    }

    /// Add an AC pair mapping
    ///
    /// When detector has a hit, check if AC also has hit in coincidence
    pub fn add_ac_pair(&mut self, det_mod: u8, det_ch: u8, ac_mod: u8, ac_ch: u8) {
        self.ac_pairs.insert((det_mod, det_ch), (ac_mod, ac_ch));
    }

    /// Set time calibration
    pub fn set_time_calibration(&mut self, calib: TimeCalibration) {
        self.time_calibration = calib;
    }

    /// Configure from channel settings
    pub fn configure_from_settings(&mut self, settings: &[ChSettings]) {
        for ch in settings {
            if ch.is_event_trigger {
                // Use ID as priority (could be configurable)
                self.add_trigger(ch.module, ch.channel, ch.id as u32);
            }
            if ch.has_ac && ch.ac_module != 128 {
                self.add_ac_pair(ch.module, ch.channel, ch.ac_module, ch.ac_channel);
            }
        }
    }

    /// Process a batch of hits
    ///
    /// Returns any complete events.
    pub fn process_hits(&mut self, hits: Vec<Hit>) -> Vec<BuiltEvent> {
        // Apply time calibration and insert into buffer
        for mut hit in hits {
            let offset = self.time_calibration.get_offset(hit.module, hit.channel);
            hit.apply_offset(offset);
            self.time_buffer.insert(hit);
        }

        // Get time-sorted ready hits
        let ready = self.time_buffer.drain_ready();
        self.pending_hits.extend(ready);

        // Build events from pending hits
        self.build_events()
    }

    /// Flush all remaining data and return final events
    pub fn flush(&mut self) -> Vec<BuiltEvent> {
        let remaining = self.time_buffer.flush();
        self.pending_hits.extend(remaining);
        self.build_events()
    }

    /// Build events from pending hits
    fn build_events(&mut self) -> Vec<BuiltEvent> {
        let mut events = Vec::new();

        while let Some(trigger) = self.find_next_trigger() {
            let event = self.build_single_event(trigger);
            events.push(event);
        }

        events
    }

    /// Find the next trigger hit in pending_hits
    ///
    /// Returns the trigger hit and removes it from pending_hits.
    /// Also removes any hits that are too early (before trigger - window).
    fn find_next_trigger(&mut self) -> Option<Hit> {
        // Find first trigger hit
        let trigger_idx = self
            .pending_hits
            .iter()
            .position(|h| self.trigger_channels.contains_key(&(h.module, h.channel)))?;

        // Remove any hits before the coincidence window starts
        let trigger_time = self.pending_hits[trigger_idx].timestamp_ns;
        let window_start = trigger_time - self.coincidence_window_ns;

        // Drain hits that are too early
        while let Some(front) = self.pending_hits.front() {
            if front.timestamp_ns < window_start {
                self.pending_hits.pop_front();
                // Adjust trigger_idx if needed
            } else {
                break;
            }
        }

        // Re-find trigger after draining
        let trigger_idx = self
            .pending_hits
            .iter()
            .position(|h| self.trigger_channels.contains_key(&(h.module, h.channel)))?;

        Some(
            self.pending_hits
                .remove(trigger_idx)
                .expect("trigger_idx came from .position() on this VecDeque just above"),
        )
    }

    /// Build a single event from a trigger hit
    fn build_single_event(&mut self, trigger: Hit) -> BuiltEvent {
        self.event_id += 1;
        let mut event = BuiltEvent::new(self.event_id, &trigger);

        let window_start = trigger.timestamp_ns - self.coincidence_window_ns;
        let window_end = trigger.timestamp_ns + self.coincidence_window_ns;

        // Collect coincident hits
        let mut to_remove = Vec::new();
        let mut ac_hits: HashMap<(u8, u8), bool> = HashMap::new();

        // First pass: identify AC hits
        for hit in self.pending_hits.iter() {
            if hit.timestamp_ns >= window_start && hit.timestamp_ns <= window_end {
                ac_hits.insert((hit.module, hit.channel), true);
            }
            if hit.timestamp_ns > window_end {
                break; // Hits are time-sorted, no need to check further
            }
        }

        // Second pass: collect coincident hits and mark AC coincidence
        for (idx, hit) in self.pending_hits.iter().enumerate() {
            if hit.timestamp_ns < window_start {
                continue;
            }
            if hit.timestamp_ns > window_end {
                break;
            }

            // Check if this is a higher priority trigger (skip it)
            if self.is_higher_priority_trigger(hit, &trigger) {
                continue;
            }

            let mut coincident_hit = hit.clone();

            // Check AC coincidence
            if let Some(&(ac_mod, ac_ch)) = self.ac_pairs.get(&(hit.module, hit.channel)) {
                if ac_hits.contains_key(&(ac_mod, ac_ch)) {
                    coincident_hit.with_ac = true;
                }
            }

            to_remove.push(idx);
            event.add_hit(&coincident_hit);
        }

        // Remove collected hits (in reverse order to preserve indices)
        for idx in to_remove.into_iter().rev() {
            self.pending_hits.remove(idx);
        }

        event.sort_hits_by_time();
        event
    }

    /// Check if hit is a higher priority trigger than the current trigger
    fn is_higher_priority_trigger(&self, hit: &Hit, trigger: &Hit) -> bool {
        let hit_priority = self.trigger_channels.get(&(hit.module, hit.channel));
        let trigger_priority = self
            .trigger_channels
            .get(&(trigger.module, trigger.channel));

        match (hit_priority, trigger_priority) {
            (Some(hp), Some(tp)) => *hp < *tp, // Lower number = higher priority
            _ => false,
        }
    }

    /// Get current event ID
    pub fn event_id(&self) -> u64 {
        self.event_id
    }

    /// Get number of pending hits
    pub fn pending_count(&self) -> usize {
        self.pending_hits.len() + self.time_buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(module: u8, channel: u8, ts: f64) -> Hit {
        Hit::new(module, channel, 1000, 500, ts)
    }

    #[test]
    fn test_new_builder() {
        let builder = L1Builder::new(500.0, 1000.0);
        assert_eq!(builder.event_id(), 0);
        assert_eq!(builder.pending_count(), 0);
    }

    #[test]
    fn test_add_trigger() {
        let mut builder = L1Builder::new(500.0, 1000.0);
        builder.add_trigger(0, 0, 0);
        builder.add_trigger(1, 0, 1);

        assert!(builder.trigger_channels.contains_key(&(0, 0)));
        assert!(builder.trigger_channels.contains_key(&(1, 0)));
    }

    #[test]
    fn test_simple_event_building() {
        let mut builder = L1Builder::new(500.0, 100.0);
        builder.add_trigger(0, 0, 0);

        // Create hits: trigger at 1000, coincident at 1100, 900
        let hits = vec![
            make_hit(0, 1, 900.0),  // Coincident
            make_hit(0, 0, 1000.0), // Trigger
            make_hit(0, 2, 1100.0), // Coincident
            make_hit(0, 3, 2000.0), // Advance time to flush
        ];

        let events = builder.process_hits(hits);

        // Should have 1 event with trigger + 2 coincident hits
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].multiplicity(), 3);
        assert_eq!(events[0].trigger_module, 0);
        assert_eq!(events[0].trigger_channel, 0);
    }

    #[test]
    fn test_coincidence_window() {
        let mut builder = L1Builder::new(200.0, 100.0); // ±200 ns window
        builder.add_trigger(0, 0, 0);

        let hits = vec![
            make_hit(0, 1, 700.0),  // Outside window (trigger - 300)
            make_hit(0, 2, 850.0),  // Inside window (trigger - 150)
            make_hit(0, 0, 1000.0), // Trigger
            make_hit(0, 3, 1150.0), // Inside window (trigger + 150)
            make_hit(0, 4, 1300.0), // Outside window (trigger + 300)
            make_hit(0, 5, 2000.0), // Advance time
        ];

        let events = builder.process_hits(hits);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].multiplicity(), 3); // Trigger + 2 coincident
    }

    #[test]
    fn test_multiple_events() {
        let mut builder = L1Builder::new(200.0, 100.0);
        builder.add_trigger(0, 0, 0);

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger 1
            make_hit(0, 1, 1050.0), // Coincident with trigger 1
            make_hit(0, 0, 2000.0), // Trigger 2
            make_hit(0, 2, 2100.0), // Coincident with trigger 2
            make_hit(0, 3, 5000.0), // Advance time
        ];

        let events = builder.process_hits(hits);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].trigger_time, 1000.0);
        assert_eq!(events[1].trigger_time, 2000.0);
    }

    #[test]
    fn test_trigger_priority() {
        let mut builder = L1Builder::new(500.0, 100.0);
        builder.add_trigger(0, 0, 0); // High priority
        builder.add_trigger(1, 0, 1); // Low priority

        // Low priority trigger at 1000, high priority at 1100
        let hits = vec![
            make_hit(1, 0, 1000.0), // Low priority trigger
            make_hit(0, 0, 1100.0), // High priority trigger (within window of low)
            make_hit(0, 1, 1050.0), // Coincident
            make_hit(0, 2, 2000.0), // Advance time
        ];

        let events = builder.process_hits(hits);

        // First event uses low priority trigger (comes first in time)
        // High priority trigger should NOT be included as coincident hit
        assert_eq!(events[0].trigger_channel, 0); // Low priority at ch 0... wait

        // Actually let me reconsider - (1, 0) is low priority trigger
        // The first trigger in time is used, regardless of priority
        assert_eq!(events[0].trigger_module, 1);
        assert_eq!(events[0].trigger_channel, 0);
    }

    #[test]
    fn test_ac_coincidence() {
        let mut builder = L1Builder::new(500.0, 100.0);
        builder.add_trigger(0, 0, 0);
        builder.add_ac_pair(0, 1, 0, 2); // Detector (0,1) has AC at (0,2)

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger
            make_hit(0, 1, 1050.0), // Detector hit
            make_hit(0, 2, 1055.0), // AC hit (in coincidence with detector)
            make_hit(0, 3, 2000.0), // Advance time
        ];

        let events = builder.process_hits(hits);

        assert_eq!(events.len(), 1);
        // Find the detector hit and check AC flag
        let det_hit = events[0]
            .hits
            .iter()
            .find(|h| h.module == 0 && h.channel == 1);
        assert!(det_hit.is_some());
        assert!(det_hit.unwrap().with_ac);
    }

    #[test]
    fn test_time_calibration() {
        let mut builder = L1Builder::new(200.0, 100.0);
        builder.add_trigger(0, 0, 0);

        let mut calib = TimeCalibration::new(0, 0);
        calib.set_offset(0, 1, 50.0); // Channel (0,1) is 50ns ahead
        builder.set_time_calibration(calib);

        // Hit at 1050 should become 1000 after calibration
        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger
            make_hit(0, 1, 1050.0), // Will be corrected to 1000
            make_hit(0, 2, 2000.0), // Advance time
        ];

        let events = builder.process_hits(hits);

        assert_eq!(events.len(), 1);
        // The calibrated hit should have relative_time near 0
        let hit = events[0]
            .hits
            .iter()
            .find(|h| h.module == 0 && h.channel == 1);
        assert!(hit.is_some());
        assert!((hit.unwrap().relative_time - 0.0).abs() < 1.0);
    }

    #[test]
    fn test_flush() {
        let mut builder = L1Builder::new(500.0, 1000.0); // Large buffer delay
        builder.add_trigger(0, 0, 0);

        let hits = vec![
            make_hit(0, 0, 1000.0), // Trigger
            make_hit(0, 1, 1050.0), // Coincident
        ];

        // Process won't build event yet (hits in buffer)
        let events = builder.process_hits(hits);
        assert!(events.is_empty());

        // Flush should build the event
        let events = builder.flush();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_configure_from_settings() {
        let mut builder = L1Builder::new(500.0, 100.0);

        let settings = vec![
            ChSettings {
                id: 0,
                module: 0,
                channel: 0,
                is_event_trigger: true,
                threshold_adc: 0,
                has_ac: false,
                ac_module: 128,
                ac_channel: 128,
                detector_type: "Si".to_string(),
                tags: vec![],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
            ChSettings {
                id: 1,
                module: 0,
                channel: 1,
                is_event_trigger: false,
                threshold_adc: 0,
                has_ac: true,
                ac_module: 0,
                ac_channel: 2,
                detector_type: "Si".to_string(),
                tags: vec![],
                p0: 0.0,
                p1: 1.0,
                p2: 0.0,
                p3: 0.0,
            },
        ];

        builder.configure_from_settings(&settings);

        assert!(builder.trigger_channels.contains_key(&(0, 0)));
        assert!(builder.ac_pairs.contains_key(&(0, 1)));
        assert_eq!(builder.ac_pairs.get(&(0, 1)), Some(&(0, 2)));
    }
}

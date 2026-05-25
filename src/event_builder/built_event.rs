//! Built event data structures
//!
//! Corresponds to ELIFANT-Event's EventData class

use super::hit::Hit;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A hit within a built event (with relative time)
///
/// Similar to the original Hit but with time expressed relative to the trigger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventHit {
    /// Module ID
    pub module: u8,
    /// Channel ID
    pub channel: u8,
    /// Energy (long gate integration, ADC units)
    pub energy: u16,
    /// Energy short (short gate, for PSD)
    pub energy_short: u16,
    /// Time relative to trigger [ns]
    /// Trigger hit has relative_time = 0.0
    pub relative_time: f64,
    /// AC coincidence flag
    pub with_ac: bool,
}

impl EventHit {
    /// Create from a Hit with relative time calculation
    #[inline]
    pub fn from_hit(hit: &Hit, trigger_time: f64) -> Self {
        Self {
            module: hit.module,
            channel: hit.channel,
            energy: hit.energy,
            energy_short: hit.energy_short,
            relative_time: hit.timestamp_ns - trigger_time,
            with_ac: hit.with_ac,
        }
    }

    /// Get channel key for lookup (module << 8 | channel)
    #[inline]
    pub fn channel_key(&self) -> u16 {
        ((self.module as u16) << 8) | (self.channel as u16)
    }
}

/// A fully built event containing trigger and coincident hits
///
/// This is the output of the L1 event builder.
/// Corresponds to ELIFANT-Event's EventData class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltEvent {
    /// Event sequential ID
    pub event_id: u64,
    /// Trigger timestamp (absolute, in ns)
    pub trigger_time: f64,
    /// Trigger module
    pub trigger_module: u8,
    /// Trigger channel
    pub trigger_channel: u8,
    /// All hits in this event (trigger is the first element)
    pub hits: Vec<EventHit>,
    /// L2 counter values annotated by `L2Filter::filter_and_annotate`.
    /// Keys are L2 `counter` op names; values are per-event hit counts.
    /// Empty when L2 has no counter ops or the event never went through L2
    /// (e.g. raw `chunk_builder` output before filtering). Wire-format-safe:
    /// older subscribers without this field round-trip cleanly via
    /// `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub counters: HashMap<String, i64>,
}

impl BuiltEvent {
    /// Create a new event from a trigger hit
    ///
    /// The trigger hit is automatically added as the first hit
    /// with relative_time = 0.0
    pub fn new(event_id: u64, trigger: &Hit) -> Self {
        let trigger_hit = EventHit {
            module: trigger.module,
            channel: trigger.channel,
            energy: trigger.energy,
            energy_short: trigger.energy_short,
            relative_time: 0.0,
            with_ac: trigger.with_ac,
        };

        Self {
            event_id,
            trigger_time: trigger.timestamp_ns,
            trigger_module: trigger.module,
            trigger_channel: trigger.channel,
            hits: vec![trigger_hit],
            counters: HashMap::new(),
        }
    }

    /// Add a coincident hit to the event
    ///
    /// The hit's relative time is calculated from the trigger time.
    pub fn add_hit(&mut self, hit: &Hit) {
        self.hits.push(EventHit::from_hit(hit, self.trigger_time));
    }

    /// Number of hits in the event (including trigger)
    #[inline]
    pub fn multiplicity(&self) -> usize {
        self.hits.len()
    }

    /// Get the trigger hit (first hit in the event)
    #[inline]
    pub fn trigger_hit(&self) -> Option<&EventHit> {
        self.hits.first()
    }

    /// Get all non-trigger hits
    pub fn coincident_hits(&self) -> &[EventHit] {
        if self.hits.len() > 1 {
            &self.hits[1..]
        } else {
            &[]
        }
    }

    /// Sort hits by relative time (trigger stays first)
    ///
    /// This is typically called after all hits have been added.
    pub fn sort_hits_by_time(&mut self) {
        if self.hits.len() > 1 {
            self.hits[1..].sort_by(|a, b| {
                a.relative_time
                    .partial_cmp(&b.relative_time)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    /// Check if any hit in the event has AC coincidence
    pub fn has_ac_coincidence(&self) -> bool {
        self.hits.iter().any(|h| h.with_ac)
    }

    /// Count hits with AC coincidence
    pub fn ac_hit_count(&self) -> usize {
        self.hits.iter().filter(|h| h.with_ac).count()
    }

    /// Get trigger channel key
    #[inline]
    pub fn trigger_channel_key(&self) -> u16 {
        ((self.trigger_module as u16) << 8) | (self.trigger_channel as u16)
    }
}

impl Default for BuiltEvent {
    fn default() -> Self {
        Self {
            event_id: 0,
            trigger_time: 0.0,
            trigger_module: 0,
            trigger_channel: 0,
            hits: Vec::new(),
            counters: HashMap::new(),
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
    fn test_built_event_new() {
        let trigger = make_hit(0, 0, 1000.0);
        let event = BuiltEvent::new(42, &trigger);

        assert_eq!(event.event_id, 42);
        assert_eq!(event.trigger_time, 1000.0);
        assert_eq!(event.trigger_module, 0);
        assert_eq!(event.trigger_channel, 0);
        assert_eq!(event.multiplicity(), 1);

        let trigger_hit = event.trigger_hit().unwrap();
        assert_eq!(trigger_hit.relative_time, 0.0);
    }

    #[test]
    fn test_built_event_add_hit() {
        let trigger = make_hit(0, 0, 1000.0);
        let mut event = BuiltEvent::new(0, &trigger);

        let hit1 = make_hit(1, 0, 1100.0);
        event.add_hit(&hit1);

        let hit2 = make_hit(1, 1, 900.0);
        event.add_hit(&hit2);

        assert_eq!(event.multiplicity(), 3);

        let coincident = event.coincident_hits();
        assert_eq!(coincident.len(), 2);
        assert_eq!(coincident[0].relative_time, 100.0); // 1100 - 1000
        assert_eq!(coincident[1].relative_time, -100.0); // 900 - 1000
    }

    #[test]
    fn test_built_event_sort_hits() {
        let trigger = make_hit(0, 0, 1000.0);
        let mut event = BuiltEvent::new(0, &trigger);

        event.add_hit(&make_hit(1, 0, 1200.0)); // +200
        event.add_hit(&make_hit(1, 1, 900.0)); // -100
        event.add_hit(&make_hit(1, 2, 1050.0)); // +50

        event.sort_hits_by_time();

        let hits = event.coincident_hits();
        assert_eq!(hits[0].relative_time, -100.0);
        assert_eq!(hits[1].relative_time, 50.0);
        assert_eq!(hits[2].relative_time, 200.0);
    }

    #[test]
    fn test_built_event_ac_coincidence() {
        let trigger = make_hit(0, 0, 1000.0);
        let mut event = BuiltEvent::new(0, &trigger);

        assert!(!event.has_ac_coincidence());
        assert_eq!(event.ac_hit_count(), 0);

        let mut hit_with_ac = make_hit(1, 0, 1100.0);
        hit_with_ac.with_ac = true;
        event.add_hit(&hit_with_ac);

        assert!(event.has_ac_coincidence());
        assert_eq!(event.ac_hit_count(), 1);
    }

    #[test]
    fn test_event_hit_channel_key() {
        let hit = EventHit {
            module: 5,
            channel: 10,
            energy: 0,
            energy_short: 0,
            relative_time: 0.0,
            with_ac: false,
        };
        assert_eq!(hit.channel_key(), (5 << 8) | 10);
    }

    #[test]
    fn counters_field_round_trips_via_json() {
        let mut ev = BuiltEvent::new(7, &make_hit(0, 0, 100.0));
        ev.counters.insert("HPGe_count".to_string(), 3);
        ev.counters.insert("Si_count".to_string(), 1);

        let json = serde_json::to_string(&ev).unwrap();
        let back: BuiltEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.counters.get("HPGe_count"), Some(&3));
        assert_eq!(back.counters.get("Si_count"), Some(&1));
    }

    #[test]
    fn legacy_payload_without_counters_field_deserializes_with_empty_map() {
        // Wire-format compat: subscribers running pre-counter-feature code
        // emitted BuiltEvent JSON / msgpack without the `counters` key. Newer
        // readers (this codebase) must accept that and default to an empty
        // map so the rest of the pipeline keeps working.
        let json = r#"{
            "event_id": 1,
            "trigger_time": 0.0,
            "trigger_module": 0,
            "trigger_channel": 0,
            "hits": []
        }"#;
        let ev: BuiltEvent = serde_json::from_str(json).unwrap();
        assert!(ev.counters.is_empty());
    }

    #[test]
    fn counters_field_skipped_in_json_when_empty() {
        // `skip_serializing_if` keeps the wire payload identical to the
        // pre-feature shape when L2 has no counter ops, so older subscribers
        // see no schema change.
        let ev = BuiltEvent::new(0, &make_hit(0, 0, 0.0));
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("counters"));
    }
}

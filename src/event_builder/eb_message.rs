//! Wire-format for the EB → EB-Monitor channel (SPEC § 9.3).
//!
//! Built-event batches are pushed to a ZMQ PUB endpoint as MessagePack
//! frames so any number of subscribers (EB Monitor, ad-hoc analysis
//! scripts, archival sinks) can observe the live stream without
//! coupling to the ROOT-writer pipeline.
//!
//! # Why MessagePack and not a fixed binary?
//!
//! The legacy `docs/event_bridge_wire_format.md` v1.0 chose a 14 B/hit
//! packed binary because the consumer was C++ and avoiding zero-copy
//! deserialization mattered. Now the consumer is also Rust + serde, so
//! MessagePack + `#[serde(default)]` gives schema evolution at near-zero
//! cost. See SPEC § 9.3.

use serde::{Deserialize, Serialize};

use super::built_event::BuiltEvent;

/// A batch of built events as emitted by one pipeline Worker thread.
///
/// `batch_id` is a per-run monotonically-increasing counter so the
/// subscriber can spot dropped batches (HWM=0 means buffered, not
/// dropped, but a sanity check is still cheap).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltEventBatch {
    pub run_number: u32,
    pub batch_id: u64,
    pub events: Vec<BuiltEvent>,
}

/// Top-level wire message on the EB PUB stream.
///
/// Layout mirrors the existing `crate::common::Message` (hit-level)
/// shape — `Events` / `EndOfStream` / `Heartbeat` — so subscribers can
/// reuse familiar dispatching logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EbMessage {
    /// One built-event batch from a worker thread.
    Events(BuiltEventBatch),
    /// Run shutting down — no more `Events` frames will arrive for
    /// `run_number`.
    EndOfStream { run_number: u32 },
    /// Periodic liveness ping so subscribers can detect a stuck PUB even
    /// during sparse data periods. `counter` increments per heartbeat.
    Heartbeat { run_number: u32, counter: u64 },
}

impl EbMessage {
    /// Serialize to MessagePack bytes via `rmp-serde`.
    pub fn to_msgpack(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(self)
    }

    /// Deserialize from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(bytes)
    }

    /// Is this message a `EndOfStream`?
    pub fn is_eos(&self) -> bool {
        matches!(self, Self::EndOfStream { .. })
    }

    /// Run number carried in the message envelope, if any.
    pub fn run_number(&self) -> u32 {
        match self {
            Self::Events(b) => b.run_number,
            Self::EndOfStream { run_number } => *run_number,
            Self::Heartbeat { run_number, .. } => *run_number,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_builder::built_event::EventHit;

    fn dummy_event(id: u64) -> BuiltEvent {
        BuiltEvent {
            event_id: id,
            trigger_time: 1234.0 + id as f64,
            trigger_module: 0,
            trigger_channel: 0,
            hits: vec![EventHit {
                module: 0,
                channel: 0,
                energy: 100,
                energy_short: 50,
                relative_time: 0.0,
                with_ac: false,
            }],
        }
    }

    #[test]
    fn events_round_trip() {
        let msg = EbMessage::Events(BuiltEventBatch {
            run_number: 42,
            batch_id: 7,
            events: vec![dummy_event(1), dummy_event(2)],
        });
        let bytes = msg.to_msgpack().unwrap();
        let back = EbMessage::from_msgpack(&bytes).unwrap();
        match back {
            EbMessage::Events(b) => {
                assert_eq!(b.run_number, 42);
                assert_eq!(b.batch_id, 7);
                assert_eq!(b.events.len(), 2);
                assert_eq!(b.events[0].event_id, 1);
                assert_eq!(b.events[1].trigger_time, 1236.0);
            }
            _ => panic!("expected Events"),
        }
    }

    #[test]
    fn eos_round_trip() {
        let msg = EbMessage::EndOfStream { run_number: 9 };
        let back = EbMessage::from_msgpack(&msg.to_msgpack().unwrap()).unwrap();
        assert!(back.is_eos());
        assert_eq!(back.run_number(), 9);
    }

    #[test]
    fn heartbeat_round_trip() {
        let msg = EbMessage::Heartbeat {
            run_number: 3,
            counter: 100,
        };
        let back = EbMessage::from_msgpack(&msg.to_msgpack().unwrap()).unwrap();
        match back {
            EbMessage::Heartbeat {
                run_number,
                counter,
            } => {
                assert_eq!(run_number, 3);
                assert_eq!(counter, 100);
            }
            _ => panic!("expected Heartbeat"),
        }
        assert!(!msg.is_eos());
    }
}

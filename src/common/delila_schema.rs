//! Self-describing schema for the `.delila` on-disk event records.
//!
//! The recorder embeds [`schema_json`] into the file header's
//! `metadata["event_schema"]` (format v3+). Downstream readers — notably the
//! C++ `TDelila` reader used by ROOT macros — learn the exact wire layout from
//! this descriptor instead of hardcoding field order, so adding or reordering a
//! field in [`EventData`]/[`Waveform`] no longer silently breaks them.
//!
//! # Wire encoding this describes
//!
//! `.delila` data blocks are `rmp_serde::to_vec` (compact) MessagePack: every
//! struct is a **positional array** in field-declaration order. This module's
//! field lists mirror that order exactly; the [`tests`] round-trip serializes a
//! real [`EventDataBatch`] and walks it against these lists, so the descriptor
//! cannot drift from the structs without a test failure.
//!
//! # Type-tag grammar
//!
//! - `u8`, `u16`, `u32`, `u64`, `i16`, `f64`, `bool` — MessagePack scalar
//! - `[T]`   — variable-length array of `T` (e.g. `[i16]`, `[EventData]`)
//! - `[T;N]` — fixed-length array of `N` `T`s (e.g. `[u64;4]`)
//! - `?T`    — optional: MessagePack `nil`, or a `T`
//! - `Name`  — nested record type resolvable via [`type_fields`]
//!
//! Note: `digital_probeN` are one `u8` (0 or 1) **per sample** — NOT bit-packed.

use serde_json::json;

/// Schema revision. Bump when the wire layout of any described type changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Root record type stored (length-prefixed) per data block.
pub const RECORD_TYPE: &str = "EventDataBatch";

/// One field of a record type: display name + wire type-tag (see module docs).
pub struct FieldDef {
    /// Field name (matches the Rust struct field).
    pub name: &'static str,
    /// Wire type-tag.
    pub tag: &'static str,
}

const fn f(name: &'static str, tag: &'static str) -> FieldDef {
    FieldDef { name, tag }
}

/// `EventDataBatch` wire layout (mirror of `src/common/mod.rs`).
pub const EVENT_DATA_BATCH: &[FieldDef] = &[
    f("source_id", "u32"),
    f("sequence_number", "u64"),
    f("timestamp", "u64"),
    f("events", "[EventData]"),
];

/// `EventData` wire layout (mirror of `src/common/mod.rs`).
pub const EVENT_DATA: &[FieldDef] = &[
    f("module", "u8"),
    f("channel", "u8"),
    f("energy", "u16"),
    f("energy_short", "u16"),
    f("timestamp_ns", "f64"),
    f("flags", "u64"),
    f("user_info", "[u64;4]"),
    f("waveform", "?Waveform"),
];

/// `Waveform` wire layout (mirror of `src/common/mod.rs`, 27 fields in order:
/// 3 analog + 16 digital probe vectors, 3 scalars, 3 signed flags, 2 type arrays).
pub const WAVEFORM: &[FieldDef] = &[
    f("analog_probe1", "[i16]"),
    f("analog_probe2", "[i16]"),
    f("analog_probe3", "[i16]"),
    f("digital_probe1", "[u8]"),
    f("digital_probe2", "[u8]"),
    f("digital_probe3", "[u8]"),
    f("digital_probe4", "[u8]"),
    f("digital_probe5", "[u8]"),
    f("digital_probe6", "[u8]"),
    f("digital_probe7", "[u8]"),
    f("digital_probe8", "[u8]"),
    f("digital_probe9", "[u8]"),
    f("digital_probe10", "[u8]"),
    f("digital_probe11", "[u8]"),
    f("digital_probe12", "[u8]"),
    f("digital_probe13", "[u8]"),
    f("digital_probe14", "[u8]"),
    f("digital_probe15", "[u8]"),
    f("digital_probe16", "[u8]"),
    f("time_resolution", "u8"),
    f("trigger_threshold", "u16"),
    f("ns_per_sample", "f64"),
    f("analog_probe1_is_signed", "bool"),
    f("analog_probe2_is_signed", "bool"),
    f("analog_probe3_is_signed", "bool"),
    f("analog_probe_type", "[u8;3]"),
    f("digital_probe_type", "[u8;16]"),
];

/// Resolve a record type's ordered field list by name.
pub fn type_fields(name: &str) -> Option<&'static [FieldDef]> {
    match name {
        "EventDataBatch" => Some(EVENT_DATA_BATCH),
        "EventData" => Some(EVENT_DATA),
        "Waveform" => Some(WAVEFORM),
        _ => None,
    }
}

fn fields_json(fields: &[FieldDef]) -> serde_json::Value {
    serde_json::Value::Array(
        fields
            .iter()
            .map(|fd| json!({ "name": fd.name, "type": fd.tag }))
            .collect(),
    )
}

/// Compact JSON descriptor embedded in `FileHeader.metadata["event_schema"]`.
pub fn schema_json() -> String {
    json!({
        "schema_version": SCHEMA_VERSION,
        "record": RECORD_TYPE,
        "types": {
            "EventDataBatch": fields_json(EVENT_DATA_BATCH),
            "EventData": fields_json(EVENT_DATA),
            "Waveform": fields_json(WAVEFORM),
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{EventData, EventDataBatch, Waveform};

    /// Walk one record (a serde_json array decoded from MessagePack) against its
    /// schema field list, returning `(name, value)` pairs. Trailing optional
    /// fields absent from the array (legacy v2 waveform-less events) are dropped,
    /// mirroring how `TDelila` walks positionally.
    fn walk<'a>(
        type_name: &str,
        val: &'a serde_json::Value,
    ) -> Vec<(&'static str, &'a serde_json::Value)> {
        let fields = type_fields(type_name).expect("known record type");
        let arr = val.as_array().expect("record is a MessagePack array");
        fields
            .iter()
            .enumerate()
            .filter_map(|(i, fd)| arr.get(i).map(|v| (fd.name, v)))
            .collect()
    }

    #[test]
    fn schema_json_is_wellformed() {
        let v: serde_json::Value = serde_json::from_str(&schema_json()).unwrap();
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert_eq!(v["record"], RECORD_TYPE);
        assert_eq!(v["types"]["EventData"].as_array().unwrap().len(), 8);
        assert_eq!(v["types"]["Waveform"].as_array().unwrap().len(), 27);
    }

    /// Drift guard: serialize real structs, decode dynamically, and confirm the
    /// schema describes the actual positional wire layout. Fails loudly if a
    /// struct field is added/reordered without updating this module.
    #[test]
    fn schema_matches_wire_layout() {
        let wf = Waveform {
            analog_probe1: vec![10, -20, 30],
            digital_probe1: vec![1, 0, 1],
            ns_per_sample: 2.0,
            ..Default::default()
        };
        let mut batch = EventDataBatch::new(7, 42);
        batch.push(EventData::new(3, 5, 800, 100, 123456.5, 0));
        batch.push(EventData::with_waveform(1, 2, 900, 0, 999.0, 0, wf));

        let bytes = batch.to_msgpack().unwrap();
        // rmp-serde is self-describing → decode into a dynamic serde_json tree
        // with no extra dependency.
        let dynamic: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();

        // EventDataBatch: 4 positional fields.
        let top = walk("EventDataBatch", &dynamic);
        assert_eq!(top.len(), EVENT_DATA_BATCH.len());
        assert_eq!(top[0].0, "source_id");
        assert_eq!(top[0].1.as_u64(), Some(7));
        assert_eq!(top[1].0, "sequence_number");
        assert_eq!(top[1].1.as_u64(), Some(42));
        assert_eq!(top[3].0, "events");
        let events = top[3].1.as_array().unwrap();
        assert_eq!(events.len(), 2);

        // Event 0: no waveform. With skip_serializing_if removed (v3) the
        // waveform is still present as nil → 8 elements.
        let e0 = walk("EventData", &events[0]);
        assert_eq!(e0.len(), EVENT_DATA.len());
        assert_eq!(e0[0].0, "module");
        assert_eq!(e0[0].1.as_u64(), Some(3));
        assert_eq!(e0[1].1.as_u64(), Some(5)); // channel
        assert_eq!(e0[2].1.as_u64(), Some(800)); // energy
        assert_eq!(e0[3].1.as_u64(), Some(100)); // energy_short
        assert_eq!(e0[4].1.as_f64(), Some(123456.5)); // timestamp_ns
        assert_eq!(e0[6].0, "user_info");
        assert_eq!(e0[6].1.as_array().unwrap().len(), 4);
        assert_eq!(e0[7].0, "waveform");
        assert!(e0[7].1.is_null());

        // Event 1: waveform present.
        let e1 = walk("EventData", &events[1]);
        assert_eq!(e1[0].1.as_u64(), Some(1)); // module
        let w = walk("Waveform", e1[7].1);
        assert_eq!(w.len(), WAVEFORM.len()); // 27
        assert_eq!(w[0].0, "analog_probe1");
        assert_eq!(w[0].1.as_array().unwrap().len(), 3);
        assert_eq!(w[0].1.as_array().unwrap()[1].as_i64(), Some(-20));
        assert_eq!(w[3].0, "digital_probe1");
        assert_eq!(w[3].1.as_array().unwrap().len(), 3);
        assert_eq!(w[21].0, "ns_per_sample");
        assert_eq!(w[21].1.as_f64(), Some(2.0));
        assert_eq!(w[25].0, "analog_probe_type");
        assert_eq!(w[25].1.as_array().unwrap().len(), 3);
        assert_eq!(w[26].0, "digital_probe_type");
        assert_eq!(w[26].1.as_array().unwrap().len(), 16);
    }
}

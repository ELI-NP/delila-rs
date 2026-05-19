//! End-to-end Event Builder integration tests.
//!
//! Wires the three config files (`eb_config.json` / `chSettings.json` /
//! `timeSettings.json`) through to `chunk_builder + L2Filter` on hand-crafted
//! synthetic hits, without going through ZMQ / ROOT / threads. The point is
//! to prove that the named-ops AST, the tree time-offsets, and the per-channel
//! tag map all line up with the hot-path code at the API boundary.
//!
//! See SPEC `TODO/event-builder/SPECIFICATION.md` §§ 4, 5, 6, 7.

use std::collections::HashMap;

use delila_rs::event_builder::chunk_builder::{build_events_from_chunk, SortedChunk};
use delila_rs::event_builder::time_offsets::TimeOffsetsFile;
use delila_rs::event_builder::{ChannelConfig, ChannelTagMap, EbRuntimeConfig, Hit, L2Filter};

fn build_tag_map(cfg: &ChannelConfig) -> ChannelTagMap {
    let mut m: ChannelTagMap = HashMap::new();
    for module_chs in cfg {
        for ch in module_chs {
            m.insert((ch.module, ch.channel), ch.tags.clone());
        }
    }
    m
}

const EB_CONFIG_JSON: &str = r#"
{
  "version": "1.0",
  "timing": {
    "coincidence_window_ns": 200.0,
    "buffer_delay_ns": 1.0e9,
    "slice_duration_ns": 1.0e7
  },
  "channels_file": "chSettings.json",
  "time_offsets_file": "timeSettings.json",
  "l1": {
    "definitions": [
      {"type": "channel",     "name": "HPGe0",      "module": 0, "channel": 0},
      {"type": "energy_gate", "name": "HPGe0_good", "source": "HPGe0",
       "min_adc": 1000, "max_adc": 50000}
    ],
    "trigger": "HPGe0_good"
  },
  "l2": [
    {"type": "counter", "name": "dE_count",  "tags": ["dE_Sector"]},
    {"type": "flag",    "name": "has_dE",    "monitor": "dE_count",
     "operator": ">", "value": 0},
    {"type": "min_hits","name": "atleast2",  "min": 2},
    {"type": "accept",  "name": "keep_with_dE",
     "monitor": ["has_dE", "atleast2"], "operator": "AND"}
  ],
  "output": {
    "events_per_file": 1000000,
    "directory": "/tmp",
    "zmq_pub_endpoint": "tcp://*:5610"
  }
}
"#;

// ELIFANT-Event compatible channel settings. Pre-Phase-J the schema still
// carries the legacy fields; this integration test exercises the L1/L2 ops
// instead of the channel-level trigger/AC flags so those fields can be
// dropped later without churning this test.
const CH_SETTINGS_JSON: &str = r#"
[
  [
    {
      "ID": 0,
      "Module": 0,
      "Channel": 0,
      "IsEventTrigger": false,
      "ThresholdADC": 100,
      "HasAC": false,
      "ACModule": 128,
      "ACChannel": 0,
      "DetectorType": "HPGe",
      "Tags": ["HPGe", "Trigger"]
    },
    {
      "ID": 1,
      "Module": 0,
      "Channel": 1,
      "IsEventTrigger": false,
      "ThresholdADC": 100,
      "HasAC": false,
      "ACModule": 128,
      "ACChannel": 0,
      "DetectorType": "Si",
      "Tags": ["dE_Sector"]
    },
    {
      "ID": 2,
      "Module": 0,
      "Channel": 2,
      "IsEventTrigger": false,
      "ThresholdADC": 100,
      "HasAC": false,
      "ACModule": 128,
      "ACChannel": 0,
      "DetectorType": "Si",
      "Tags": ["E_Sector"]
    }
  ]
]
"#;

const TIME_OFFSETS_JSON: &str = r#"
{
  "version": "1.0",
  "entries": [
    {"module": 0, "channel": 0, "ref": null,   "offset_ns": 0.0},
    {"module": 0, "channel": 1, "ref": [0, 0], "offset_ns": 5.0},
    {"module": 0, "channel": 2, "ref": [0, 0], "offset_ns": -3.0}
  ]
}
"#;

/// Apply per-channel time calibration to a hit (mirrors the sorter
/// behaviour at the pipeline boundary — see `pipeline.rs::sorter_thread`).
fn apply_offsets(hits: &mut [Hit], offsets: &impl Fn(u8, u8) -> f64) {
    for h in hits.iter_mut() {
        h.timestamp_ns -= offsets(h.module, h.channel);
    }
}

#[test]
fn three_config_files_drive_l1_l2_pipeline() {
    // --- Parse all three configs --------------------------------------------
    let eb_cfg: EbRuntimeConfig = serde_json::from_str(EB_CONFIG_JSON).unwrap();
    eb_cfg.validate().expect("eb_config validates");

    let ch_cfg: ChannelConfig = serde_json::from_str(CH_SETTINGS_JSON).unwrap();
    let tag_map = build_tag_map(&ch_cfg);

    let time_file: TimeOffsetsFile = serde_json::from_str(TIME_OFFSETS_JSON).unwrap();
    let time_resolved = time_file.resolve().expect("time tree resolves");

    // --- Derive hot-path structures -----------------------------------------
    let trigger_config = eb_cfg.build_trigger_config().expect("L1 adapter succeeds");
    let l2_filter = L2Filter::new(eb_cfg.l2.clone(), tag_map).expect("L2 filter builds");

    // Energy gate from `HPGe0_good` should have made it into the trigger config.
    assert_eq!(
        trigger_config.trigger_energy_gates.get(&(0, 0)),
        Some(&(1000, 50000))
    );

    // --- Synthesise hits ----------------------------------------------------
    // Event A: HPGe trigger with valid energy + a coincident dE hit + an
    //          E_Sector hit. multiplicity ≥ 2 AND dE present → KEEP.
    //
    // Event B: HPGe trigger below energy gate (1000 ADC threshold), so it
    //          never becomes a trigger anchor. The dE/E hits with no trigger
    //          → no event built at all.
    //
    // Event C: HPGe trigger with valid energy but NO dE hit alongside →
    //          dE_count is 0 → has_dE is false → L2 rejects.
    let mut hits = vec![
        // Event A trigger
        Hit::new(0, 0, 5000, 0, 1000.0),
        Hit::new(0, 1, 800, 0, 1020.0), // dE_Sector @ +20 ns from trigger (post-calib)
        Hit::new(0, 2, 1200, 0, 1100.0), // E_Sector @ +100 ns
        // Event B (sub-threshold trigger, should not anchor anything)
        Hit::new(0, 0, 500, 0, 5000.0), // below energy gate
        Hit::new(0, 1, 800, 0, 5020.0),
        // Event C (no dE → L2 rejects)
        Hit::new(0, 0, 5000, 0, 9000.0),
        Hit::new(0, 2, 1200, 0, 9050.0),
    ];

    // Apply per-channel offsets exactly like the pipeline does at ingress.
    let offsets = |m: u8, c: u8| time_resolved.get_or_zero(m, c);
    apply_offsets(&mut hits, &offsets);

    // chunk_builder requires sorted hits.
    hits.sort_by(|a, b| a.timestamp_ns.total_cmp(&b.timestamp_ns));

    let chunk = SortedChunk {
        hits,
        core_end: 1.0e6, // Safe-horizon far in the future — process everything.
    };

    // --- Run L1 (chunk_builder) ---------------------------------------------
    let l1_events = build_events_from_chunk(&chunk, &trigger_config);

    // Two anchors fire (Event A trigger and Event C trigger). Event B is
    // gated out by the L1 energy gate.
    assert_eq!(l1_events.len(), 2, "expected 2 L1 events (A and C)");

    // --- Run L2 -------------------------------------------------------------
    let kept_events: Vec<_> = l1_events.iter().filter(|ev| l2_filter.keeps(ev)).collect();

    // Only Event A survives L2 (multiplicity ≥ 2 AND dE present).
    assert_eq!(kept_events.len(), 1, "L2 should keep only Event A");
    let kept = kept_events[0];
    assert_eq!(kept.trigger_module, 0);
    assert_eq!(kept.trigger_channel, 0);
    assert!(
        kept.hits.iter().any(|h| h.module == 0 && h.channel == 1),
        "kept event must contain the dE coincidence"
    );

    // --- Verify time alignment ----------------------------------------------
    // The dE channel had a +5 ns offset; after aligning, its relative time
    // to the trigger should be (1020 - 5) - 1000 = 15 ns, not 20 ns.
    let de_hit = kept
        .hits
        .iter()
        .find(|h| h.channel == 1)
        .expect("dE hit present");
    assert!(
        (de_hit.relative_time - 15.0).abs() < 1e-9,
        "expected aligned dE relative_time ≈ 15 ns, got {}",
        de_hit.relative_time
    );
}

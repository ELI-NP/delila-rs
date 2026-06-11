//! L2 evaluator — filters built events through the named-ops chain
//! defined in `eb_config.json` § `l2` (SPEC § 7).
//!
//! Pipeline placement: runs in the Worker thread **after** `chunk_builder`
//! has produced `BuiltEvent`s; events for which no `Accept` op evaluates
//! to true are dropped before being sent to the Writers.
//!
//! # MVP variant set
//!
//! Implemented now:
//!
//! - `counter` — count hits whose channel tags intersect `tags`
//! - `flag`    — compare a `counter` result against a constant
//! - `accept`  — combine `flag` results with `AND` / `OR`; the event is
//!   kept if **any** `accept` op evaluates to true
//!
//! Declared in the SPEC but not yet evaluated (will error at filter
//! construction time so callers see a clear migration message rather than
//! silent passthrough):
//!
//! - `energy_gate`, `ac_veto`, `min_hits`

use std::collections::HashMap;
use std::sync::Arc;

use super::built_event::BuiltEvent;
#[cfg(test)]
use super::runtime_config::CmpOp;
use super::runtime_config::{L2Op, LogicOp};

/// Map of `(module, channel) → list of channel tags`, derived from
/// `chSettings.json`. Tags are case-sensitive, matched literally.
pub type ChannelTagMap = HashMap<(u8, u8), Vec<String>>;

/// Pre-validated L2 filter pipeline.
///
/// Construct once at startup with [`L2Filter::new`], then call
/// [`L2Filter::keeps`] on each built event from the hot path.
#[derive(Debug, Clone)]
pub struct L2Filter {
    ops: Vec<L2Op>,
    /// `(module, channel)` → tag set for counter evaluation.
    channel_tags: Arc<ChannelTagMap>,
}

#[derive(thiserror::Error, Debug)]
pub enum L2FilterError {
    #[error("L2 op `{name}` references unknown op `{target}`")]
    UnknownReference { name: String, target: String },
    #[error("L2 op `{name}` of type `{op_type}` is declared in the SPEC but not yet implemented")]
    Unimplemented { name: String, op_type: &'static str },
    #[error("L2 chain must contain at least one `accept` op")]
    NoAccept,
    #[error(
        "L2 counter name `{name}` is not a valid ROOT TTree branch name. \
         Counter names are written as ROOT branches in the output and must \
         match [A-Za-z_][A-Za-z0-9_]* (no `-`, `.`, spaces, leading digits). \
         Rename to something like `HPGe_count`."
    )]
    InvalidCounterName { name: String },
}

/// True if `s` is a valid ROOT TTree branch name (and a valid C identifier).
/// Used by [`L2Filter::new`] to reject counter `name`s that would otherwise
/// collide with ROOT operator characters (e.g. `-` is parsed as subtraction).
fn is_root_safe_branch_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl L2Filter {
    /// Construct the filter. Validates that every cross-reference points at
    /// a known op and that every op type is one the evaluator supports.
    pub fn new(ops: Vec<L2Op>, channel_tags: ChannelTagMap) -> Result<Self, L2FilterError> {
        let mut by_name: HashMap<String, usize> = HashMap::new();
        for (i, op) in ops.iter().enumerate() {
            by_name.insert(op.name().to_string(), i);
        }

        // Per-op validation
        let mut has_accept = false;
        for op in &ops {
            match op {
                L2Op::Counter { name, .. } => {
                    // Counter names become ROOT branches in the output, so
                    // reject anything ROOT can't accept (`-`, `.`, spaces,
                    // leading digit, ...). Same check is also in the JSON
                    // Schema for IDE feedback; this is the runtime safety net
                    // when validate-config or build is run on a file that
                    // bypassed the schema.
                    if !is_root_safe_branch_name(name) {
                        return Err(L2FilterError::InvalidCounterName { name: name.clone() });
                    }
                }
                L2Op::EnergyGate { .. } | L2Op::MinHits { .. } | L2Op::AcVeto { .. } => {
                    // Standalone ops (no inter-op references to resolve).
                }
                L2Op::Flag { name, monitor, .. } => {
                    if !by_name.contains_key(monitor) {
                        return Err(L2FilterError::UnknownReference {
                            name: name.clone(),
                            target: monitor.clone(),
                        });
                    }
                }
                L2Op::Accept { name, monitor, .. } => {
                    has_accept = true;
                    for m in monitor {
                        if !by_name.contains_key(m) {
                            return Err(L2FilterError::UnknownReference {
                                name: name.clone(),
                                target: m.clone(),
                            });
                        }
                    }
                }
            }
        }
        if !has_accept {
            return Err(L2FilterError::NoAccept);
        }

        Ok(Self {
            ops,
            channel_tags: Arc::new(channel_tags),
        })
    }

    /// Names of every `counter` L2 op in definition order. Used by the ROOT
    /// writer to declare one branch per counter so all events in a file share
    /// the same set of branches.
    pub fn counter_names(&self) -> Vec<String> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                L2Op::Counter { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// Filter events in-place: drop events that pass no `accept` op, and for
    /// the kept ones write each `counter` value into `event.counters`. This is
    /// the canonical path from the pipeline worker — combining filter + counter
    /// annotation in one pass avoids re-evaluating L2 ops for the writer.
    pub fn filter_and_annotate(&self, events: &mut Vec<BuiltEvent>) {
        events.retain_mut(|ev| {
            let (accepted, counters) = self.evaluate_with_counters(ev);
            if accepted.is_empty() {
                false
            } else {
                ev.counters = counters;
                true
            }
        });
    }

    /// Per-event check: returns the list of accepted op names (empty if
    /// the event should be **dropped**, non-empty if **kept**).
    ///
    /// Hot path: allocations are bounded — `counters` and `flags` HashMaps
    /// have size = number of L2 ops, typically a handful.
    pub fn evaluate(&self, event: &BuiltEvent) -> Vec<String> {
        self.evaluate_with_counters(event).0
    }

    /// Same as [`evaluate`] but also returns the per-counter snapshot keyed by
    /// counter op name. Used by [`filter_and_annotate`] and tests; the writer
    /// pulls these values out of `BuiltEvent.counters` afterward.
    pub fn evaluate_with_counters(
        &self,
        event: &BuiltEvent,
    ) -> (Vec<String>, HashMap<String, i64>) {
        let mut counters: HashMap<&str, i64> = HashMap::with_capacity(self.ops.len());
        let mut flags: HashMap<&str, bool> = HashMap::with_capacity(self.ops.len());
        let mut accepted: Vec<String> = Vec::new();

        for op in &self.ops {
            match op {
                L2Op::Counter { name, tags } => {
                    let mut count: i64 = 0;
                    for hit in &event.hits {
                        if let Some(channel_tags) =
                            self.channel_tags.get(&(hit.module, hit.channel))
                        {
                            if channel_tags.iter().any(|t| tags.contains(t)) {
                                count += 1;
                            }
                        }
                    }
                    counters.insert(name.as_str(), count);
                }
                L2Op::Flag {
                    name,
                    monitor,
                    operator,
                    value,
                } => {
                    let lhs = counters
                        .get(monitor.as_str())
                        .copied()
                        // monitor might also refer to another Flag (treated as 0/1).
                        .or_else(|| flags.get(monitor.as_str()).map(|b| if *b { 1 } else { 0 }))
                        .unwrap_or(0);
                    flags.insert(name.as_str(), operator.apply(lhs, *value));
                }
                L2Op::Accept {
                    name,
                    monitor,
                    operator,
                } => {
                    let inputs: Vec<bool> = monitor
                        .iter()
                        .map(|m| flags.get(m.as_str()).copied().unwrap_or(false))
                        .collect();
                    let result = match operator {
                        LogicOp::And => !inputs.is_empty() && inputs.iter().all(|b| *b),
                        LogicOp::Or => inputs.iter().any(|b| *b),
                    };
                    flags.insert(name.as_str(), result);
                    if result {
                        accepted.push(name.clone());
                    }
                }
                L2Op::EnergyGate {
                    name,
                    module,
                    channel,
                    min_adc,
                    max_adc,
                } => {
                    // True if at least one hit on (module, channel) has
                    // energy in [min_adc, max_adc]. Treated as a `flag`
                    // for downstream `accept` references.
                    let result = event.hits.iter().any(|h| {
                        h.module == *module
                            && h.channel == *channel
                            && h.energy >= *min_adc
                            && h.energy <= *max_adc
                    });
                    flags.insert(name.as_str(), result);
                }
                L2Op::MinHits { name, min } => {
                    flags.insert(name.as_str(), (event.hits.len() as u32) >= *min);
                }
                L2Op::AcVeto {
                    name,
                    trigger_channels,
                    veto_channels,
                    window_ns,
                } => {
                    // Veto fires if any trigger-channel hit has a
                    // veto-channel hit within ±window_ns.
                    //
                    // SPEC § 7.2: a true `flag` here means "this event
                    // SHOULD BE VETOED". Downstream Accept ops typically
                    // chain via `monitor: ["veto_flag", "good_flag"]`
                    // with operator AND of (NOT veto), so the eval here
                    // is "any pair within window → true". Users should
                    // compose a `flag` of `==0` on a `counter` named
                    // veto to invert (or extend the SPEC with a `not`
                    // op later).
                    let trigger_set: std::collections::HashSet<(u8, u8)> = trigger_channels
                        .iter()
                        .map(|c| (c.module, c.channel))
                        .collect();
                    let veto_set: std::collections::HashSet<(u8, u8)> = veto_channels
                        .iter()
                        .map(|c| (c.module, c.channel))
                        .collect();

                    let mut vetoed = false;
                    'outer: for ht in &event.hits {
                        if !trigger_set.contains(&(ht.module, ht.channel)) {
                            continue;
                        }
                        for hv in &event.hits {
                            if !veto_set.contains(&(hv.module, hv.channel)) {
                                continue;
                            }
                            if (ht.relative_time - hv.relative_time).abs() <= *window_ns {
                                vetoed = true;
                                break 'outer;
                            }
                        }
                    }
                    flags.insert(name.as_str(), vetoed);
                }
            }
        }

        // Snapshot the per-counter values into an owned-key map so the caller
        // (writer / filter_and_annotate) can store them in BuiltEvent.counters
        // beyond the lifetime of this evaluation.
        let counter_snapshot: HashMap<String, i64> = counters
            .iter()
            .map(|(&name, &value)| (name.to_string(), value))
            .collect();

        (accepted, counter_snapshot)
    }

    /// Hot-path bool check: `true` ↔ at least one `accept` op evaluated to true.
    #[inline]
    pub fn keeps(&self, event: &BuiltEvent) -> bool {
        !self.evaluate(event).is_empty()
    }

    /// Convenience for tests / introspection.
    pub fn op_count(&self) -> usize {
        self.ops.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_builder::built_event::EventHit;

    fn ev_with(hits: &[(u8, u8)]) -> BuiltEvent {
        let mut ev = BuiltEvent {
            trigger_module: hits[0].0,
            trigger_channel: hits[0].1,
            ..BuiltEvent::default()
        };
        for (m, c) in hits {
            ev.hits.push(EventHit {
                module: *m,
                channel: *c,
                energy: 0,
                energy_short: 0,
                relative_time: 0.0,
                with_ac: false,
            });
        }
        ev
    }

    fn tags(pairs: &[((u8, u8), &[&str])]) -> ChannelTagMap {
        pairs
            .iter()
            .map(|(k, v)| (*k, v.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn minimal_ops() -> Vec<L2Op> {
        vec![
            L2Op::Counter {
                name: "Si_E".to_string(),
                tags: vec!["E_Sector".to_string()],
            },
            L2Op::Flag {
                name: "Si_E_pos".to_string(),
                monitor: "Si_E".to_string(),
                operator: CmpOp::Gt,
                value: 0,
            },
            L2Op::Accept {
                name: "keep".to_string(),
                monitor: vec!["Si_E_pos".to_string()],
                operator: LogicOp::And,
            },
        ]
    }

    #[test]
    fn keeps_event_with_matching_tag() {
        let tags = tags(&[((0, 0), &["E_Sector"])]);
        let f = L2Filter::new(minimal_ops(), tags).unwrap();
        let ev = ev_with(&[(0, 0)]);
        assert!(f.keeps(&ev));
        assert_eq!(f.evaluate(&ev), vec!["keep".to_string()]);
    }

    #[test]
    fn drops_event_without_matching_tag() {
        let tags = tags(&[((0, 0), &["dE_Sector"])]);
        let f = L2Filter::new(minimal_ops(), tags).unwrap();
        let ev = ev_with(&[(0, 0)]);
        assert!(!f.keeps(&ev));
        assert!(f.evaluate(&ev).is_empty());
    }

    #[test]
    fn drops_event_with_no_known_channels() {
        let tags = tags(&[((9, 9), &["E_Sector"])]);
        let f = L2Filter::new(minimal_ops(), tags).unwrap();
        let ev = ev_with(&[(0, 0)]);
        assert!(!f.keeps(&ev));
    }

    #[test]
    fn counter_names_listed_in_definition_order() {
        let ops = vec![
            L2Op::Counter {
                name: "HPGe_count".into(),
                tags: vec!["HPGe".into()],
            },
            L2Op::Counter {
                name: "Si_count".into(),
                tags: vec!["Si".into()],
            },
            L2Op::Flag {
                name: "any".into(),
                monitor: "HPGe_count".into(),
                operator: CmpOp::Gt,
                value: 0,
            },
            L2Op::Accept {
                name: "keep".into(),
                monitor: vec!["any".into()],
                operator: LogicOp::And,
            },
        ];
        let f = L2Filter::new(ops, ChannelTagMap::new()).unwrap();
        assert_eq!(
            f.counter_names(),
            vec!["HPGe_count".to_string(), "Si_count".to_string()]
        );
    }

    #[test]
    fn filter_and_annotate_writes_counters_into_kept_events() {
        // Two channels with different tags. Counter sees both, flag selects
        // events with HPGe_count > 0.
        let tags = tags(&[((0, 0), &["HPGe"]), ((1, 0), &["Si"])]);
        let ops = vec![
            L2Op::Counter {
                name: "HPGe_count".into(),
                tags: vec!["HPGe".into()],
            },
            L2Op::Counter {
                name: "Si_count".into(),
                tags: vec!["Si".into()],
            },
            L2Op::Flag {
                name: "has_HPGe".into(),
                monitor: "HPGe_count".into(),
                operator: CmpOp::Gt,
                value: 0,
            },
            L2Op::Accept {
                name: "keep".into(),
                monitor: vec!["has_HPGe".into()],
                operator: LogicOp::And,
            },
        ];
        let f = L2Filter::new(ops, tags).unwrap();
        let mut events = vec![
            ev_with(&[(0, 0), (1, 0)]), // HPGe=1, Si=1 → kept
            ev_with(&[(1, 0)]),         // HPGe=0, Si=1 → dropped
            ev_with(&[(0, 0), (0, 0)]), // HPGe=2, Si=0 → kept
        ];
        f.filter_and_annotate(&mut events);
        assert_eq!(events.len(), 2, "dropped exactly one event");
        // First kept event has HPGe=1, Si=1
        assert_eq!(events[0].counters.get("HPGe_count"), Some(&1));
        assert_eq!(events[0].counters.get("Si_count"), Some(&1));
        // Second kept event has HPGe=2, Si=0
        assert_eq!(events[1].counters.get("HPGe_count"), Some(&2));
        assert_eq!(events[1].counters.get("Si_count"), Some(&0));
    }

    #[test]
    fn rejects_counter_name_with_hyphen() {
        let ops = vec![
            L2Op::Counter {
                name: "HPGe-count".into(),
                tags: vec!["HPGe".into()],
            },
            L2Op::Accept {
                name: "keep".into(),
                monitor: vec![],
                operator: LogicOp::Or,
            },
        ];
        let err = L2Filter::new(ops, ChannelTagMap::new()).unwrap_err();
        assert!(
            matches!(err, L2FilterError::InvalidCounterName { ref name } if name == "HPGe-count"),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_counter_name_with_dot_space_or_leading_digit() {
        for bad in ["HPGe.count", "HPGe count", "2HPGe", ""] {
            let ops = vec![
                L2Op::Counter {
                    name: bad.into(),
                    tags: vec!["HPGe".into()],
                },
                L2Op::Accept {
                    name: "keep".into(),
                    monitor: vec![],
                    operator: LogicOp::Or,
                },
            ];
            let err = L2Filter::new(ops, ChannelTagMap::new()).unwrap_err();
            assert!(
                matches!(err, L2FilterError::InvalidCounterName { .. }),
                "name `{bad}` should be rejected, got {err:?}"
            );
        }
    }

    #[test]
    fn accepts_counter_name_with_underscore_and_digits() {
        // ELIFANT-Event style names — all C-identifier-safe.
        for good in ["HPGe_count", "E_Sector_Counter", "_internal", "x42"] {
            let ops = vec![
                L2Op::Counter {
                    name: good.into(),
                    tags: vec!["HPGe".into()],
                },
                L2Op::Accept {
                    name: "keep".into(),
                    monitor: vec![],
                    operator: LogicOp::Or,
                },
            ];
            L2Filter::new(ops, ChannelTagMap::new())
                .unwrap_or_else(|e| panic!("name `{good}` should be accepted, got {e:?}"));
        }
    }

    #[test]
    fn elifant_si_both_pattern() {
        // Reproduces the ELIFANT-Event L2Settings.json example:
        //   Counter(E_Sector) → Flag(E_pos>0)
        //   Counter(dE_Sector) → Flag(dE_pos>0)
        //   Accept(Si_Both = E_pos AND dE_pos)
        let ops = vec![
            L2Op::Counter {
                name: "E_Sector_Counter".into(),
                tags: vec!["E_Sector".into()],
            },
            L2Op::Counter {
                name: "dE_Sector_Counter".into(),
                tags: vec!["dE_Sector".into()],
            },
            L2Op::Flag {
                name: "E_pos".into(),
                monitor: "E_Sector_Counter".into(),
                operator: CmpOp::Gt,
                value: 0,
            },
            L2Op::Flag {
                name: "dE_pos".into(),
                monitor: "dE_Sector_Counter".into(),
                operator: CmpOp::Gt,
                value: 0,
            },
            L2Op::Accept {
                name: "Si_Both".into(),
                monitor: vec!["E_pos".into(), "dE_pos".into()],
                operator: LogicOp::And,
            },
        ];
        let tag_map = tags(&[
            ((0, 0), &["E_Sector"]),
            ((0, 1), &["dE_Sector"]),
            ((1, 0), &["E_Sector"]),
        ]);
        let f = L2Filter::new(ops, tag_map).unwrap();

        // Two hits with both tags present → AND of flags is true → accepted.
        assert!(f.keeps(&ev_with(&[(0, 0), (0, 1)])));
        // Only E side present → dE_pos is false → AND false → dropped.
        assert!(!f.keeps(&ev_with(&[(0, 0), (1, 0)])));
        // Only dE side → E_pos false → dropped.
        assert!(!f.keeps(&ev_with(&[(0, 1)])));
    }

    #[test]
    fn or_accept_passes_with_any_flag_true() {
        let ops = vec![
            L2Op::Counter {
                name: "A".into(),
                tags: vec!["a".into()],
            },
            L2Op::Counter {
                name: "B".into(),
                tags: vec!["b".into()],
            },
            L2Op::Flag {
                name: "fA".into(),
                monitor: "A".into(),
                operator: CmpOp::Gt,
                value: 0,
            },
            L2Op::Flag {
                name: "fB".into(),
                monitor: "B".into(),
                operator: CmpOp::Gt,
                value: 0,
            },
            L2Op::Accept {
                name: "either".into(),
                monitor: vec!["fA".into(), "fB".into()],
                operator: LogicOp::Or,
            },
        ];
        let tag_map = tags(&[((0, 0), &["a"]), ((0, 1), &["b"])]);
        let f = L2Filter::new(ops, tag_map).unwrap();

        // Just A — OR accept is true.
        assert!(f.keeps(&ev_with(&[(0, 0)])));
        // Just B — also true.
        assert!(f.keeps(&ev_with(&[(0, 1)])));
        // Neither — false.
        assert!(!f.keeps(&ev_with(&[(9, 9)])));
    }

    #[test]
    fn min_hits_keeps_when_multiplicity_threshold_met() {
        let ops = vec![
            L2Op::MinHits {
                name: "at_least_2".into(),
                min: 2,
            },
            L2Op::Accept {
                name: "keep".into(),
                monitor: vec!["at_least_2".into()],
                operator: LogicOp::And,
            },
        ];
        let f = L2Filter::new(ops, HashMap::new()).unwrap();
        assert!(f.keeps(&ev_with(&[(0, 0), (0, 1)])));
        assert!(f.keeps(&ev_with(&[(0, 0), (0, 1), (0, 2)])));
        assert!(!f.keeps(&ev_with(&[(0, 0)])));
    }

    fn ev_with_energy(hits: &[(u8, u8, u16)]) -> BuiltEvent {
        let mut ev = BuiltEvent {
            trigger_module: hits[0].0,
            trigger_channel: hits[0].1,
            ..BuiltEvent::default()
        };
        for (m, c, e) in hits {
            ev.hits.push(EventHit {
                module: *m,
                channel: *c,
                energy: *e,
                energy_short: 0,
                relative_time: 0.0,
                with_ac: false,
            });
        }
        ev
    }

    #[test]
    fn energy_gate_filters_by_adc_range() {
        let ops = vec![
            L2Op::EnergyGate {
                name: "HPGe0_good".into(),
                module: 0,
                channel: 0,
                min_adc: 100,
                max_adc: 16000,
            },
            L2Op::Accept {
                name: "keep".into(),
                monitor: vec!["HPGe0_good".into()],
                operator: LogicOp::And,
            },
        ];
        let f = L2Filter::new(ops, HashMap::new()).unwrap();

        assert!(f.keeps(&ev_with_energy(&[(0, 0, 5000)])));
        assert!(!f.keeps(&ev_with_energy(&[(0, 0, 50)]))); // below min
        assert!(!f.keeps(&ev_with_energy(&[(0, 0, 20000)]))); // above max
        assert!(!f.keeps(&ev_with_energy(&[(1, 0, 5000)]))); // wrong channel
                                                             // OK if at least one hit on (0,0) is in range.
        assert!(f.keeps(&ev_with_energy(&[(0, 0, 50), (0, 0, 5000)])));
    }

    fn ev_with_times(hits: &[(u8, u8, f64)]) -> BuiltEvent {
        let mut ev = BuiltEvent {
            trigger_module: hits[0].0,
            trigger_channel: hits[0].1,
            trigger_time: 0.0,
            ..BuiltEvent::default()
        };
        for (m, c, rt) in hits {
            ev.hits.push(EventHit {
                module: *m,
                channel: *c,
                energy: 0,
                energy_short: 0,
                relative_time: *rt,
                with_ac: false,
            });
        }
        ev
    }

    #[test]
    fn ac_veto_flag_fires_inside_window_clears_outside() {
        use crate::event_builder::runtime_config::{ChannelRef, CmpOp};
        let ops = vec![
            L2Op::AcVeto {
                name: "vetoed".into(),
                trigger_channels: vec![ChannelRef {
                    module: 0,
                    channel: 0,
                }],
                veto_channels: vec![ChannelRef {
                    module: 0,
                    channel: 1,
                }],
                window_ns: 200.0,
            },
            L2Op::Flag {
                name: "vetoed_flag".into(),
                monitor: "vetoed".into(),
                operator: CmpOp::Eq,
                value: 1,
            },
            L2Op::Accept {
                name: "is_vetoed".into(),
                monitor: vec!["vetoed_flag".into()],
                operator: LogicOp::And,
            },
        ];
        let f = L2Filter::new(ops, HashMap::new()).unwrap();

        // veto channel fires inside window → vetoed = true.
        assert!(f.keeps(&ev_with_times(&[(0, 0, 0.0), (0, 1, 50.0)])));
        // veto channel fires outside window → vetoed = false.
        assert!(!f.keeps(&ev_with_times(&[(0, 0, 0.0), (0, 1, 500.0)])));
        // No veto channel in event → not vetoed.
        assert!(!f.keeps(&ev_with_times(&[(0, 0, 0.0)])));
    }

    #[test]
    fn keep_unless_vetoed_via_flag_negation() {
        use crate::event_builder::runtime_config::{ChannelRef, CmpOp};
        let ops = vec![
            L2Op::AcVeto {
                name: "vetoed".into(),
                trigger_channels: vec![ChannelRef {
                    module: 0,
                    channel: 0,
                }],
                veto_channels: vec![ChannelRef {
                    module: 0,
                    channel: 1,
                }],
                window_ns: 200.0,
            },
            L2Op::Flag {
                name: "not_vetoed".into(),
                monitor: "vetoed".into(),
                operator: CmpOp::Eq,
                value: 0,
            },
            L2Op::Accept {
                name: "keep".into(),
                monitor: vec!["not_vetoed".into()],
                operator: LogicOp::And,
            },
        ];
        let f = L2Filter::new(ops, HashMap::new()).unwrap();

        // Vetoed → dropped.
        assert!(!f.keeps(&ev_with_times(&[(0, 0, 0.0), (0, 1, 50.0)])));
        // Not vetoed (outside window) → kept.
        assert!(f.keeps(&ev_with_times(&[(0, 0, 0.0), (0, 1, 500.0)])));
        // No veto channel hit → kept.
        assert!(f.keeps(&ev_with_times(&[(0, 0, 0.0)])));
    }

    #[test]
    fn no_accept_rejected() {
        let ops = vec![L2Op::Counter {
            name: "A".into(),
            tags: vec![],
        }];
        assert!(matches!(
            L2Filter::new(ops, HashMap::new()),
            Err(L2FilterError::NoAccept)
        ));
    }

    #[test]
    fn unknown_reference_rejected() {
        let ops = vec![
            L2Op::Flag {
                name: "f".into(),
                monitor: "Ghost".into(),
                operator: CmpOp::Eq,
                value: 0,
            },
            L2Op::Accept {
                name: "a".into(),
                monitor: vec!["f".into()],
                operator: LogicOp::And,
            },
        ];
        let err = L2Filter::new(ops, HashMap::new()).unwrap_err();
        assert!(matches!(err, L2FilterError::UnknownReference { .. }));
    }
}

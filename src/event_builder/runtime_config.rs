//! Event Builder runtime configuration (`eb_config.json`).
//!
//! Implements the schema defined in
//! `TODO/event-builder/SPECIFICATION.md` §§ 4.1, 6, 7:
//!
//! - `timing` block (coincidence / buffer / slice windows)
//! - `channels_file` / `time_offsets_file` paths to the sibling JSON files
//! - `l1.definitions` — named-ops AST for trigger recognition
//! - `l2` — named-ops AST for built-event filtering (ELIFANT style)
//! - `output` — ROOT writer + ZMQ PUB endpoint
//!
//! This module **only** defines the data model + serde derive +
//! lightweight validation (cycle / unique-name / dangling-reference
//! checks). The actual L1/L2 evaluators land alongside the unified
//! pipeline refactor (SPEC § 11.4 Phase 4/5).
//!
//! # Variant coverage
//!
//! All L1 and L2 ops declared in SPEC v0.7 are wired up:
//!
//! | Layer | Implemented |
//! |-------|-------------|
//! | L1    | `Channel`, `Or`, `Multiplicity`, `And` |
//! | L2    | `Counter`, `Flag`, `Accept`, `EnergyGate`, `MinHits`, `AcVeto` |
//!
//! `And` and `Multiplicity` are stateful — they get lowered into a
//! [`MultiplicityTrigger`] (sliding-window check evaluated inside
//! `chunk_builder::build_events_from_chunk`).
//!
//! L1 `EnergyGate` was removed in v0.7 — pushing per-channel threshold
//! cuts into the EB violated the data-preservation principle in SPEC
//! § 1.4 for no measurable gain (p91Zr v3 run: 100 ADC gate dropped
//! < 0.1 % of events). Threshold cuts now live in analysis macros.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use thiserror::Error;

use crate::event_builder::chunk_builder::{MultiplicityTrigger, TriggerConfig};

/// Top-level runtime config: `eb_config.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EbRuntimeConfig {
    pub version: String,
    pub timing: TimingConfig,
    pub channels_file: String,
    pub time_offsets_file: String,
    pub l1: L1Config,
    pub l2: Vec<L2Op>,
    pub output: OutputConfig,
}

/// `timing` block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimingConfig {
    pub coincidence_window_ns: f64,
    pub buffer_delay_ns: f64,
    pub slice_duration_ns: f64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            coincidence_window_ns: 500.0,
            buffer_delay_ns: 1.0e9,
            slice_duration_ns: 1.0e7,
        }
    }
}

/// `l1` block: `definitions` (named-ops) + `trigger` (root name).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct L1Config {
    pub definitions: Vec<L1Op>,
    /// Name of the L1 op whose truth value triggers event construction.
    pub trigger: String,
}

/// One L1 op. `name` identifies it; downstream ops reference by name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum L1Op {
    /// "This (module, channel) hit qualifies as a trigger candidate."
    Channel {
        name: String,
        module: u8,
        channel: u8,
    },
    /// "Any hit in module `module` (channels `0..channels`) qualifies."
    /// Syntactic sugar — expands to the same set of `(module, c)` tuples a
    /// hand-rolled `or` over `channel` ops would produce. Useful when a
    /// digitizer has 16/32/64 channels and you'd otherwise write a wall of
    /// `channel` entries plus one `or`.
    Module {
        name: String,
        module: u8,
        channels: u8,
    },
    /// "Any input op is true (logical OR)."
    Or { name: String, inputs: Vec<String> },
    /// "All inputs fire within `window_ns` of each other."
    And {
        name: String,
        inputs: Vec<String>,
        window_ns: f64,
    },
    /// "`min` of the listed channels fire within `window_ns`."
    Multiplicity {
        name: String,
        channels: Vec<String>,
        min: u32,
        window_ns: f64,
    },
}

/// Discriminator string used in error messages.
fn op_type_str(op: &L1Op) -> &'static str {
    match op {
        L1Op::Channel { .. } => "channel",
        L1Op::Module { .. } => "module",
        L1Op::Or { .. } => "or",
        L1Op::And { .. } => "and",
        L1Op::Multiplicity { .. } => "multiplicity",
    }
}

impl L1Op {
    pub fn name(&self) -> &str {
        match self {
            Self::Channel { name, .. }
            | Self::Module { name, .. }
            | Self::Or { name, .. }
            | Self::And { name, .. }
            | Self::Multiplicity { name, .. } => name,
        }
    }

    /// Names this op references (used for topological / DAG validation).
    pub fn dependencies(&self) -> Vec<&str> {
        match self {
            Self::Channel { .. } | Self::Module { .. } => Vec::new(),
            Self::Or { inputs, .. } | Self::And { inputs, .. } => {
                inputs.iter().map(String::as_str).collect()
            }
            Self::Multiplicity { channels, .. } => channels.iter().map(String::as_str).collect(),
        }
    }
}

/// One L2 op. ELIFANT-Event style `Counter → Flag → Accept` chain plus
/// our additions (`EnergyGate`, `AcVeto`, `MinHits`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum L2Op {
    /// Count hits whose channel tags intersect `tags`. — MVP.
    Counter { name: String, tags: Vec<String> },
    /// Compare a counter result to `value` with `operator`. — MVP.
    Flag {
        name: String,
        monitor: String,
        operator: CmpOp,
        value: i64,
    },
    /// Combine flags via `operator`; result decides event acceptance. — MVP.
    Accept {
        name: String,
        monitor: Vec<String>,
        operator: LogicOp,
    },
    /// Per-(module, channel) energy gate at L2. — deferred.
    EnergyGate {
        name: String,
        module: u8,
        channel: u8,
        min_adc: u16,
        max_adc: u16,
    },
    /// Reject event if any `veto_channels` fired within `window_ns` of any
    /// `trigger_channels`. — deferred.
    AcVeto {
        name: String,
        trigger_channels: Vec<ChannelRef>,
        veto_channels: Vec<ChannelRef>,
        window_ns: f64,
    },
    /// "Event has at least `min` hits." — deferred.
    MinHits { name: String, min: u32 },
}

impl L2Op {
    pub fn name(&self) -> &str {
        match self {
            Self::Counter { name, .. }
            | Self::Flag { name, .. }
            | Self::Accept { name, .. }
            | Self::EnergyGate { name, .. }
            | Self::AcVeto { name, .. }
            | Self::MinHits { name, .. } => name,
        }
    }

    /// Names this op references.
    pub fn dependencies(&self) -> Vec<&str> {
        match self {
            Self::Counter { .. }
            | Self::EnergyGate { .. }
            | Self::AcVeto { .. }
            | Self::MinHits { .. } => Vec::new(),
            Self::Flag { monitor, .. } => vec![monitor.as_str()],
            Self::Accept { monitor, .. } => monitor.iter().map(String::as_str).collect(),
        }
    }

    /// Is this op an `Accept` (whose truth value gates event output)?
    pub fn is_accept(&self) -> bool {
        matches!(self, Self::Accept { .. })
    }
}

/// Comparison operator for `L2Op::Flag`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmpOp {
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
}

impl CmpOp {
    pub fn apply(self, lhs: i64, rhs: i64) -> bool {
        match self {
            Self::Eq => lhs == rhs,
            Self::Ne => lhs != rhs,
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            Self::Gt => lhs > rhs,
            Self::Ge => lhs >= rhs,
        }
    }
}

/// Logical operator for `L2Op::Accept`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LogicOp {
    #[serde(rename = "AND")]
    And,
    #[serde(rename = "OR")]
    Or,
}

/// Channel reference shared by several L2 ops.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ChannelRef {
    pub module: u8,
    pub channel: u8,
}

/// `output` block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputConfig {
    pub events_per_file: u64,
    pub directory: String,
    /// Optional ZMQ PUB endpoint for the live BuiltEvent stream
    /// (SPEC § 9.3). `None` / `null` / missing all disable the PUB
    /// thread; the value is consumed by `online_event_builder` and
    /// ignored by the offline `event_builder build`.
    #[serde(default)]
    pub zmq_pub_endpoint: Option<String>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            events_per_file: 1_000_000,
            directory: "./eb_output".to_string(),
            zmq_pub_endpoint: Some("tcp://*:5610".to_string()),
        }
    }
}

/// Error during runtime-config load / validation.
#[derive(Error, Debug)]
pub enum RuntimeConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("validation: {0}")]
    Invalid(String),
}

impl EbRuntimeConfig {
    /// Build a [`TriggerConfig`] for the existing `chunk_builder` pipeline
    /// from this runtime config (MVP: `Channel` ops only).
    ///
    /// Walks the L1 op graph starting from `l1.trigger` and collects every
    /// underlying `Channel` op into the resulting trigger set. Priorities
    /// are assigned by the order in which channels are first reached
    /// (depth-first), so a `multiplicity` / `or` op (once implemented)
    /// will deterministically derive priorities from the JSON ordering.
    ///
    /// `ac_pairs` is **always empty**. AC veto lives in L2 (`L2Op::AcVeto`)
    /// and is applied by the L2 evaluator on built events, not by
    /// `chunk_builder`. See SPEC § 5 (2-layer threshold model).
    ///
    /// Returns an error if:
    /// - the `trigger` name does not exist in `l1.definitions`
    /// - the resolved root op (or any descendant) is not yet supported
    pub fn build_trigger_config(&self) -> Result<TriggerConfig, RuntimeConfigError> {
        let by_name: HashMap<&str, &L1Op> = self
            .l1
            .definitions
            .iter()
            .map(|op| (op.name(), op))
            .collect();

        let mut triggers: HashSet<(u8, u8)> = HashSet::new();
        let mut priorities: HashMap<(u8, u8), u32> = HashMap::new();
        let mut multiplicities: Vec<MultiplicityTrigger> = Vec::new();
        let mut visiting: HashSet<&str> = HashSet::new();

        self.collect_trigger_channels(
            &self.l1.trigger,
            &by_name,
            &mut triggers,
            &mut priorities,
            &mut multiplicities,
            &mut visiting,
        )?;

        Ok(TriggerConfig {
            triggers,
            priorities,
            ac_pairs: HashMap::new(),
            coincidence_window_ns: self.timing.coincidence_window_ns,
            multiplicity_triggers: multiplicities,
        })
    }

    /// Resolve an `L1Op` name expected to refer to channels (a `channel` leaf
    /// or a `module` group), returning every concrete `(module, channel)` it
    /// covers. Used by `and` / `multiplicity` resolution. `module` expands
    /// to `0..channels` so `multiplicity` over a `module` op gives "any N of
    /// M's channels within window".
    fn resolve_leaf_channels<'a>(
        name: &'a str,
        by_name: &HashMap<&'a str, &'a L1Op>,
        parent_op: &str,
    ) -> Result<Vec<(u8, u8)>, RuntimeConfigError> {
        match by_name.get(name) {
            Some(L1Op::Channel {
                module, channel, ..
            }) => Ok(vec![(*module, *channel)]),
            Some(L1Op::Module {
                module, channels, ..
            }) => Ok((0..*channels).map(|c| (*module, c)).collect()),
            Some(other) => Err(RuntimeConfigError::Invalid(format!(
                "L1 op `{parent_op}` expects a `channel` or `module` input, but `{name}` is `{}`",
                op_type_str(other)
            ))),
            None => Err(RuntimeConfigError::Invalid(format!(
                "L1 op `{parent_op}` references unknown name `{name}`"
            ))),
        }
    }

    /// DFS over the L1 op graph that resolves `name` down to a set of
    /// `Channel` ops. Concrete results are accumulated into the
    /// out-parameter maps.
    fn collect_trigger_channels<'a>(
        &'a self,
        name: &'a str,
        by_name: &HashMap<&'a str, &'a L1Op>,
        triggers: &mut HashSet<(u8, u8)>,
        priorities: &mut HashMap<(u8, u8), u32>,
        multiplicities: &mut Vec<MultiplicityTrigger>,
        visiting: &mut HashSet<&'a str>,
    ) -> Result<(), RuntimeConfigError> {
        let op = *by_name.get(name).ok_or_else(|| {
            RuntimeConfigError::Invalid(format!("L1 trigger `{name}` is not defined"))
        })?;

        if !visiting.insert(op.name()) {
            return Err(RuntimeConfigError::Invalid(format!(
                "L1 cycle re-entered at `{}` while building trigger config",
                op.name()
            )));
        }

        match op {
            L1Op::Channel {
                module, channel, ..
            } => {
                let key = (*module, *channel);
                if triggers.insert(key) {
                    // First time we see this channel — assign priority by insertion order.
                    let prio = priorities.len() as u32;
                    priorities.insert(key, prio);
                }
            }
            L1Op::Module {
                module, channels, ..
            } => {
                for c in 0..*channels {
                    let key = (*module, c);
                    if triggers.insert(key) {
                        let prio = priorities.len() as u32;
                        priorities.insert(key, prio);
                    }
                }
            }
            L1Op::Or { inputs, .. } => {
                for child in inputs {
                    self.collect_trigger_channels(
                        child,
                        by_name,
                        triggers,
                        priorities,
                        multiplicities,
                        visiting,
                    )?;
                }
            }
            // `and` lowers to a Multiplicity with min = |inputs| (must all
            // fire). `multiplicity` keeps its own min. Both require their
            // inputs / channels to resolve to leaf `channel` ops in the
            // MVP — nested combinators inside a stateful trigger are not
            // supported yet.
            L1Op::And {
                inputs, window_ns, ..
            } => {
                let mut channel_set: HashSet<(u8, u8)> = HashSet::new();
                for child in inputs {
                    for ch in Self::resolve_leaf_channels(child, by_name, op.name())? {
                        channel_set.insert(ch);
                    }
                }
                let min = channel_set.len() as u32;
                multiplicities.push(MultiplicityTrigger {
                    channels: channel_set,
                    min,
                    window_ns: *window_ns,
                });
            }
            L1Op::Multiplicity {
                channels,
                min,
                window_ns,
                ..
            } => {
                let mut channel_set: HashSet<(u8, u8)> = HashSet::new();
                for child in channels {
                    for ch in Self::resolve_leaf_channels(child, by_name, op.name())? {
                        channel_set.insert(ch);
                    }
                }
                if *min == 0 {
                    return Err(RuntimeConfigError::Invalid(format!(
                        "L1 op `{}`: multiplicity `min` must be > 0",
                        op.name()
                    )));
                }
                if (*min as usize) > channel_set.len() {
                    return Err(RuntimeConfigError::Invalid(format!(
                        "L1 op `{}`: multiplicity `min` ({}) exceeds the number of channels ({})",
                        op.name(),
                        min,
                        channel_set.len()
                    )));
                }
                multiplicities.push(MultiplicityTrigger {
                    channels: channel_set,
                    min: *min,
                    window_ns: *window_ns,
                });
            }
        }
        visiting.remove(op.name());
        Ok(())
    }

    /// Load from a JSON file and validate.
    pub fn load(path: &Path) -> Result<Self, RuntimeConfigError> {
        let content = std::fs::read_to_string(path)?;
        let cfg: Self = serde_json::from_str(&content)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Save to a JSON file (pretty-printed).
    pub fn save(&self, path: &Path) -> Result<(), RuntimeConfigError> {
        self.validate()?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Run all static validation checks (called from `load` and `save`).
    pub fn validate(&self) -> Result<(), RuntimeConfigError> {
        self.validate_timing()?;
        self.validate_l1()?;
        self.validate_l2()?;
        Ok(())
    }

    fn validate_timing(&self) -> Result<(), RuntimeConfigError> {
        let t = &self.timing;
        // Reject NaN / non-finite / non-positive in one shot.
        if !t.coincidence_window_ns.is_finite() || t.coincidence_window_ns <= 0.0 {
            return Err(RuntimeConfigError::Invalid(format!(
                "coincidence_window_ns must be > 0 (got {})",
                t.coincidence_window_ns
            )));
        }
        if !t.buffer_delay_ns.is_finite() || t.buffer_delay_ns <= 0.0 {
            return Err(RuntimeConfigError::Invalid(format!(
                "buffer_delay_ns must be > 0 (got {})",
                t.buffer_delay_ns
            )));
        }
        if !t.slice_duration_ns.is_finite() || t.slice_duration_ns <= 0.0 {
            return Err(RuntimeConfigError::Invalid(format!(
                "slice_duration_ns must be > 0 (got {})",
                t.slice_duration_ns
            )));
        }
        if t.slice_duration_ns <= t.coincidence_window_ns {
            return Err(RuntimeConfigError::Invalid(format!(
                "slice_duration_ns ({}) must be > coincidence_window_ns ({}) — overlap region would equal or exceed the core",
                t.slice_duration_ns, t.coincidence_window_ns
            )));
        }
        Ok(())
    }

    fn validate_l1(&self) -> Result<(), RuntimeConfigError> {
        let names = check_unique_names(self.l1.definitions.iter().map(L1Op::name))
            .map_err(|n| RuntimeConfigError::Invalid(format!("L1 duplicate name: {n}")))?;

        for op in &self.l1.definitions {
            if let L1Op::Module { name, channels, .. } = op {
                if *channels == 0 {
                    return Err(RuntimeConfigError::Invalid(format!(
                        "L1 op `{name}`: module `channels` must be >= 1"
                    )));
                }
            }
            for dep in op.dependencies() {
                if !names.contains(dep) {
                    return Err(RuntimeConfigError::Invalid(format!(
                        "L1 op `{}` references unknown name `{}`",
                        op.name(),
                        dep
                    )));
                }
            }
        }

        if !names.contains(self.l1.trigger.as_str()) {
            return Err(RuntimeConfigError::Invalid(format!(
                "L1 trigger `{}` is not defined",
                self.l1.trigger
            )));
        }

        detect_cycle(
            self.l1.definitions.iter(),
            |op| op.name(),
            |op| op.dependencies(),
        )
        .map_err(|c| RuntimeConfigError::Invalid(format!("L1 cycle: {c}")))?;

        Ok(())
    }

    fn validate_l2(&self) -> Result<(), RuntimeConfigError> {
        let names = check_unique_names(self.l2.iter().map(L2Op::name))
            .map_err(|n| RuntimeConfigError::Invalid(format!("L2 duplicate name: {n}")))?;

        for op in &self.l2 {
            for dep in op.dependencies() {
                if !names.contains(dep) {
                    return Err(RuntimeConfigError::Invalid(format!(
                        "L2 op `{}` references unknown name `{}`",
                        op.name(),
                        dep
                    )));
                }
            }
        }

        detect_cycle(self.l2.iter(), |op| op.name(), |op| op.dependencies())
            .map_err(|c| RuntimeConfigError::Invalid(format!("L2 cycle: {c}")))?;

        if !self.l2.iter().any(L2Op::is_accept) {
            return Err(RuntimeConfigError::Invalid(
                "L2 chain must contain at least one `accept` op to gate event output".to_string(),
            ));
        }

        Ok(())
    }
}

fn check_unique_names<'a, I>(iter: I) -> Result<HashSet<&'a str>, &'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut seen: HashSet<&str> = HashSet::new();
    for name in iter {
        if !seen.insert(name) {
            return Err(name);
        }
    }
    Ok(seen)
}

/// Detect a cycle by attempting a topological sort (Kahn's algorithm).
/// Returns `Err(name)` on the first node found to be part of a cycle.
fn detect_cycle<'a, T, F, G>(
    items: impl Iterator<Item = &'a T>,
    name: F,
    deps: G,
) -> Result<(), String>
where
    T: 'a,
    F: Fn(&'a T) -> &'a str,
    G: Fn(&'a T) -> Vec<&'a str>,
{
    use std::collections::HashMap;

    let items: Vec<&'a T> = items.collect();
    let by_name: HashMap<&str, &T> = items.iter().map(|t| (name(t), *t)).collect();

    // Count in-edges per node (an "in-edge" here means: another node lists this one
    // as a dependency).
    let mut in_count: HashMap<&str, usize> = HashMap::new();
    for t in &items {
        in_count.entry(name(t)).or_insert(0);
    }
    for t in &items {
        for d in deps(t) {
            if by_name.contains_key(d) {
                *in_count.entry(d).or_insert(0) += 1;
            }
        }
    }

    // Standard Kahn: pull off nodes with zero in-edges, decrement their depends-on.
    // If any node remains in_count > 0 at the end, the rest is a cycle.
    let mut zero: Vec<&str> = in_count
        .iter()
        .filter_map(|(k, v)| if *v == 0 { Some(*k) } else { None })
        .collect();
    let mut visited = 0usize;
    while let Some(n) = zero.pop() {
        visited += 1;
        if let Some(t) = by_name.get(n) {
            for d in deps(t) {
                if let Some(c) = in_count.get_mut(d) {
                    *c -= 1;
                    if *c == 0 {
                        zero.push(d);
                    }
                }
            }
        }
    }
    if visited != items.len() {
        let stuck: Vec<&str> = in_count
            .iter()
            .filter_map(|(k, v)| if *v > 0 { Some(*k) } else { None })
            .collect();
        return Err(format!("cycle involves: {stuck:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_valid_config() -> EbRuntimeConfig {
        EbRuntimeConfig {
            version: "1.0".to_string(),
            timing: TimingConfig::default(),
            channels_file: "chSettings.json".to_string(),
            time_offsets_file: "timeSettings.json".to_string(),
            l1: L1Config {
                definitions: vec![L1Op::Channel {
                    name: "HPGe0".to_string(),
                    module: 0,
                    channel: 0,
                }],
                trigger: "HPGe0".to_string(),
            },
            l2: vec![
                L2Op::Counter {
                    name: "HPGe_counter".to_string(),
                    tags: vec!["HPGe".to_string()],
                },
                L2Op::Flag {
                    name: "HPGe_present".to_string(),
                    monitor: "HPGe_counter".to_string(),
                    operator: CmpOp::Gt,
                    value: 0,
                },
                L2Op::Accept {
                    name: "keep".to_string(),
                    monitor: vec!["HPGe_present".to_string()],
                    operator: LogicOp::And,
                },
            ],
            output: OutputConfig::default(),
        }
    }

    #[test]
    fn minimal_config_validates() {
        minimal_valid_config().validate().unwrap();
    }

    #[test]
    fn cmpop_apply() {
        assert!(CmpOp::Eq.apply(5, 5));
        assert!(!CmpOp::Eq.apply(5, 6));
        assert!(CmpOp::Gt.apply(7, 5));
        assert!(!CmpOp::Gt.apply(5, 7));
        assert!(CmpOp::Le.apply(5, 5));
        assert!(CmpOp::Le.apply(4, 5));
        assert!(!CmpOp::Le.apply(6, 5));
    }

    #[test]
    fn duplicate_l1_name_rejected() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.push(L1Op::Channel {
            name: "HPGe0".to_string(),
            module: 0,
            channel: 1,
        });
        let e = cfg.validate().unwrap_err();
        assert!(matches!(e, RuntimeConfigError::Invalid(_)), "got {e:?}");
        assert!(e.to_string().contains("duplicate"), "got {e}");
    }

    #[test]
    fn dangling_l1_reference_rejected() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.push(L1Op::Or {
            name: "combo".to_string(),
            inputs: vec!["HPGe0".to_string(), "DoesNotExist".to_string()],
        });
        cfg.l1.trigger = "combo".to_string();
        let e = cfg.validate().unwrap_err();
        assert!(e.to_string().contains("DoesNotExist"), "got {e}");
    }

    #[test]
    fn unknown_l1_trigger_rejected() {
        let mut cfg = minimal_valid_config();
        cfg.l1.trigger = "Ghost".to_string();
        let e = cfg.validate().unwrap_err();
        assert!(e.to_string().contains("Ghost"), "got {e}");
    }

    #[test]
    fn l2_missing_accept_rejected() {
        let mut cfg = minimal_valid_config();
        cfg.l2.retain(|op| !op.is_accept());
        let e = cfg.validate().unwrap_err();
        assert!(e.to_string().contains("accept"), "got {e}");
    }

    #[test]
    fn l2_cycle_detected() {
        // a → b → a
        let cfg = EbRuntimeConfig {
            version: "1.0".into(),
            timing: TimingConfig::default(),
            channels_file: "x".into(),
            time_offsets_file: "y".into(),
            l1: L1Config {
                definitions: vec![L1Op::Channel {
                    name: "T".into(),
                    module: 0,
                    channel: 0,
                }],
                trigger: "T".into(),
            },
            l2: vec![
                L2Op::Accept {
                    name: "a".into(),
                    monitor: vec!["b".into()],
                    operator: LogicOp::And,
                },
                L2Op::Accept {
                    name: "b".into(),
                    monitor: vec!["a".into()],
                    operator: LogicOp::And,
                },
            ],
            output: OutputConfig::default(),
        };
        let e = cfg.validate().unwrap_err();
        assert!(e.to_string().contains("cycle"), "got {e}");
    }

    #[test]
    fn coincidence_window_must_be_smaller_than_slice() {
        let mut cfg = minimal_valid_config();
        cfg.timing.slice_duration_ns = cfg.timing.coincidence_window_ns;
        let e = cfg.validate().unwrap_err();
        assert!(e.to_string().contains("slice_duration_ns"), "got {e}");
    }

    #[test]
    fn full_roundtrip_via_json_string() {
        let cfg = minimal_valid_config();
        let s = serde_json::to_string_pretty(&cfg).unwrap();
        let back: EbRuntimeConfig = serde_json::from_str(&s).unwrap();
        back.validate().unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn parses_full_example_with_deferred_ops() {
        let json = r#"
        {
          "version": "1.0",
          "timing": {
            "coincidence_window_ns": 500.0,
            "buffer_delay_ns": 1.0e9,
            "slice_duration_ns": 1.0e7
          },
          "channels_file": "chSettings.json",
          "time_offsets_file": "timeSettings.json",
          "l1": {
            "definitions": [
              {"type": "channel", "name": "HPGe0", "module": 0, "channel": 0},
              {"type": "channel", "name": "HPGe1", "module": 0, "channel": 1},
              {"type": "or", "name": "HPGe_any", "inputs": ["HPGe0", "HPGe1"]}
            ],
            "trigger": "HPGe_any"
          },
          "l2": [
            {"type": "counter", "name": "E_Sector_Counter", "tags": ["E_Sector"]},
            {"type": "counter", "name": "dE_Sector_Counter", "tags": ["dE_Sector"]},
            {"type": "flag", "name": "E_pos", "monitor": "E_Sector_Counter",
             "operator": ">", "value": 0},
            {"type": "flag", "name": "dE_pos", "monitor": "dE_Sector_Counter",
             "operator": ">", "value": 0},
            {"type": "accept", "name": "Si_Both",
             "monitor": ["E_pos", "dE_pos"], "operator": "AND"},
            {"type": "ac_veto", "name": "VetoedHPGe",
             "trigger_channels": [{"module": 0, "channel": 0}],
             "veto_channels": [{"module": 0, "channel": 1}],
             "window_ns": 200.0},
            {"type": "min_hits", "name": "atleast2", "min": 2}
          ],
          "output": {
            "events_per_file": 1000000,
            "directory": "./eb_output",
            "zmq_pub_endpoint": "tcp://*:5610"
          }
        }
        "#;
        let cfg: EbRuntimeConfig = serde_json::from_str(json).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.l1.definitions.len(), 3);
        assert_eq!(cfg.l2.len(), 7);
        // Operator round-trips:
        if let L2Op::Flag { operator, .. } = &cfg.l2[2] {
            assert_eq!(*operator, CmpOp::Gt);
        } else {
            panic!("expected Flag at index 2");
        }
        if let L2Op::Accept { operator, .. } = &cfg.l2[4] {
            assert_eq!(*operator, LogicOp::And);
        } else {
            panic!("expected Accept at index 4");
        }
    }

    #[test]
    fn build_trigger_config_single_channel() {
        let cfg = minimal_valid_config();
        let tc = cfg.build_trigger_config().unwrap();
        assert_eq!(tc.triggers.len(), 1);
        assert!(tc.triggers.contains(&(0, 0)));
        assert_eq!(tc.priorities.get(&(0, 0)), Some(&0));
        assert!(tc.ac_pairs.is_empty(), "AC pairs live in L2 ac_veto now");
        assert_eq!(tc.coincidence_window_ns, 500.0);
    }

    #[test]
    fn build_trigger_config_or_collects_all_channels() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.push(L1Op::Channel {
            name: "HPGe1".to_string(),
            module: 0,
            channel: 1,
        });
        cfg.l1.definitions.push(L1Op::Or {
            name: "any".to_string(),
            inputs: vec!["HPGe0".to_string(), "HPGe1".to_string()],
        });
        cfg.l1.trigger = "any".to_string();
        let tc = cfg.build_trigger_config().unwrap();
        assert_eq!(tc.triggers.len(), 2);
        assert!(tc.triggers.contains(&(0, 0)));
        assert!(tc.triggers.contains(&(0, 1)));
        // Priorities assigned by visitation order (DFS).
        let p0 = tc.priorities.get(&(0, 0)).copied().unwrap();
        let p1 = tc.priorities.get(&(0, 1)).copied().unwrap();
        assert!(
            p0 < p1,
            "first-visited channel should get lower priority value"
        );
    }

    #[test]
    fn multiplicity_lowers_to_trigger_config() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.push(L1Op::Channel {
            name: "HPGe1".into(),
            module: 0,
            channel: 1,
        });
        cfg.l1.definitions.push(L1Op::Multiplicity {
            name: "mult".into(),
            channels: vec!["HPGe0".into(), "HPGe1".into()],
            min: 2,
            window_ns: 100.0,
        });
        cfg.l1.trigger = "mult".into();
        let tc = cfg.build_trigger_config().unwrap();
        assert_eq!(tc.multiplicity_triggers.len(), 1);
        let mt = &tc.multiplicity_triggers[0];
        assert_eq!(mt.min, 2);
        assert!((mt.window_ns - 100.0).abs() < 1e-9);
        assert!(mt.channels.contains(&(0, 0)));
        assert!(mt.channels.contains(&(0, 1)));
        // Multiplicity channels are NOT added to `triggers`: a single hit
        // on (0, 0) alone shouldn't fire — only the stateful sliding-window
        // check inside chunk_builder decides anchor-ness.
        assert!(tc.triggers.is_empty(), "got {:?}", tc.triggers);
    }

    #[test]
    fn and_lowers_to_multiplicity_with_min_equal_inputs() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.push(L1Op::Channel {
            name: "HPGe1".into(),
            module: 0,
            channel: 1,
        });
        cfg.l1.definitions.push(L1Op::And {
            name: "both".into(),
            inputs: vec!["HPGe0".into(), "HPGe1".into()],
            window_ns: 50.0,
        });
        cfg.l1.trigger = "both".into();
        let tc = cfg.build_trigger_config().unwrap();
        assert_eq!(tc.multiplicity_triggers.len(), 1);
        let mt = &tc.multiplicity_triggers[0];
        assert_eq!(mt.min, 2, "and: min must equal number of channels");
        assert_eq!(mt.channels.len(), 2);
    }

    #[test]
    fn multiplicity_min_must_not_exceed_channel_count() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.push(L1Op::Multiplicity {
            name: "mult".into(),
            channels: vec!["HPGe0".into()],
            min: 2, // exceeds 1 channel
            window_ns: 100.0,
        });
        cfg.l1.trigger = "mult".into();
        let e = cfg.build_trigger_config().unwrap_err();
        assert!(e.to_string().contains("exceeds"), "got {e}");
    }

    #[test]
    fn multiplicity_input_must_be_channel_op() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.push(L1Op::Or {
            name: "any_HPGe".into(),
            inputs: vec!["HPGe0".into()],
        });
        cfg.l1.definitions.push(L1Op::Multiplicity {
            name: "mult".into(),
            channels: vec!["any_HPGe".into()], // OR op, not a channel op
            min: 1,
            window_ns: 100.0,
        });
        cfg.l1.trigger = "mult".into();
        let e = cfg.build_trigger_config().unwrap_err();
        assert!(
            e.to_string().contains("expects a `channel` or `module` input"),
            "got {e}"
        );
    }

    // L1 `energy_gate` was removed in v0.7 — pushing threshold cuts at
    // the EB layer caused permanent data loss for negligible benefit (see
    // p91Zr v3 run: 100 ADC gate dropped <0.1 % of events). The matching
    // tests have been deleted; threshold cuts now live in analysis macros.

    #[test]
    fn module_op_at_top_level_expands_to_all_channels() {
        // A 16-ch `module` op used directly as the trigger root should
        // register all 16 channels as triggers, just like an `or` over 16
        // explicit `channel` ops would.
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.clear();
        cfg.l1.definitions.push(L1Op::Module {
            name: "any_M0".into(),
            module: 0,
            channels: 16,
        });
        cfg.l1.trigger = "any_M0".into();
        let tc = cfg.build_trigger_config().unwrap();
        assert_eq!(tc.triggers.len(), 16);
        for c in 0..16u8 {
            assert!(tc.triggers.contains(&(0, c)), "missing (0, {c})");
        }
    }

    #[test]
    fn module_op_inside_or() {
        // Two modules ORed — the trigger set should be the union.
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.clear();
        cfg.l1.definitions.push(L1Op::Module {
            name: "any_M0".into(),
            module: 0,
            channels: 4,
        });
        cfg.l1.definitions.push(L1Op::Module {
            name: "any_M4".into(),
            module: 4,
            channels: 4,
        });
        cfg.l1.definitions.push(L1Op::Or {
            name: "any".into(),
            inputs: vec!["any_M0".into(), "any_M4".into()],
        });
        cfg.l1.trigger = "any".into();
        let tc = cfg.build_trigger_config().unwrap();
        assert_eq!(tc.triggers.len(), 8);
        assert!(tc.triggers.contains(&(0, 0)));
        assert!(tc.triggers.contains(&(0, 3)));
        assert!(tc.triggers.contains(&(4, 0)));
        assert!(tc.triggers.contains(&(4, 3)));
    }

    #[test]
    fn module_op_inside_multiplicity() {
        // "Any 2 of module 0's 8 channels within 200 ns" — the typical HPGe
        // pair-coincidence shortcut. The expanded multiplicity should cover
        // all 8 channels.
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.clear();
        cfg.l1.definitions.push(L1Op::Module {
            name: "any_HPGe".into(),
            module: 0,
            channels: 8,
        });
        cfg.l1.definitions.push(L1Op::Multiplicity {
            name: "HPGe_pair".into(),
            channels: vec!["any_HPGe".into()],
            min: 2,
            window_ns: 200.0,
        });
        cfg.l1.trigger = "HPGe_pair".into();
        let tc = cfg.build_trigger_config().unwrap();
        assert_eq!(tc.multiplicity_triggers.len(), 1);
        let m = &tc.multiplicity_triggers[0];
        assert_eq!(m.min, 2);
        assert_eq!(m.channels.len(), 8);
        for c in 0..8u8 {
            assert!(m.channels.contains(&(0, c)), "missing (0, {c})");
        }
    }

    #[test]
    fn module_op_with_zero_channels_is_rejected() {
        let mut cfg = minimal_valid_config();
        cfg.l1.definitions.clear();
        cfg.l1.definitions.push(L1Op::Module {
            name: "empty".into(),
            module: 0,
            channels: 0,
        });
        cfg.l1.trigger = "empty".into();
        let e = cfg.validate().unwrap_err();
        assert!(
            e.to_string().contains("`channels` must be >= 1"),
            "got {e}"
        );
    }

    #[test]
    fn build_trigger_config_rejects_unknown_trigger() {
        let mut cfg = minimal_valid_config();
        // Sneak past validate() by clearing l1 entirely
        cfg.l1.definitions.clear();
        cfg.l1.trigger = "Ghost".to_string();
        let e = cfg.build_trigger_config().unwrap_err();
        assert!(e.to_string().contains("Ghost"), "got {e}");
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = std::env::temp_dir().join(format!("eb_config_test_{}.json", std::process::id()));
        let cfg = minimal_valid_config();
        cfg.save(&path).unwrap();
        let loaded = EbRuntimeConfig::load(&path).unwrap();
        assert_eq!(cfg, loaded);
        let _ = std::fs::remove_file(&path);
    }
}

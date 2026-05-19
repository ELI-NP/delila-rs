//! Tree-based per-channel time offsets (`timeSettings.json`).
//!
//! Implements SPEC `TODO/event-builder/SPECIFICATION.md` § 4.3:
//!
//! - JSON entries form a forest: each entry has a `ref` pointing at its
//!   parent `(module, channel)`, or `null` if the entry is a root.
//! - At load time the forest is **resolved** (DFS walk per root) into a
//!   flat `HashMap<(mod, ch), f64>` of absolute offsets, plus auxiliary
//!   per-channel info (root, depth).
//! - Convention: `aligned_ts = raw_ts - offset_ns`.
//! - Multiple roots are allowed (multi-domain), with a warn at load time.
//!
//! Validation:
//!
//! | Severity | Condition |
//! |----------|-----------|
//! | Error    | cycle, dangling `ref`, duplicate `(module, channel)` |
//! | Warn     | multiple roots, tree depth > 5 |
//!
//! Hot path: `ResolvedTimeOffsets::get(module, channel)` is a `HashMap`
//! lookup → O(1). Channels missing from `timeSettings` resolve to `0.0`
//! and are warned at load (see SPEC § 4.3.3).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use thiserror::Error;

/// A `(module, channel)` reference used by [`TimeOffsetEntry::parent`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(from = "RefRepr", into = "RefRepr")]
pub struct ParentRef {
    pub module: u8,
    pub channel: u8,
}

/// JSON wire form: `[module, channel]` (a 2-element array).
#[derive(Serialize, Deserialize)]
struct RefRepr([u8; 2]);

impl From<RefRepr> for ParentRef {
    fn from(r: RefRepr) -> Self {
        Self {
            module: r.0[0],
            channel: r.0[1],
        }
    }
}

impl From<ParentRef> for RefRepr {
    fn from(p: ParentRef) -> Self {
        RefRepr([p.module, p.channel])
    }
}

/// One entry in `timeSettings.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeOffsetEntry {
    pub module: u8,
    pub channel: u8,
    /// Parent `(module, channel)` in the offset tree, or `None` for a root.
    #[serde(rename = "ref")]
    pub parent: Option<ParentRef>,
    pub offset_ns: f64,
}

/// Top-level structure: `timeSettings.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeOffsetsFile {
    pub version: String,
    pub entries: Vec<TimeOffsetEntry>,
}

/// Result of resolving the offset tree into flat lookup tables.
#[derive(Debug, Clone, Default)]
pub struct ResolvedTimeOffsets {
    offsets: HashMap<(u8, u8), f64>,
    roots: HashMap<(u8, u8), (u8, u8)>,
    depths: HashMap<(u8, u8), u32>,
    /// Warnings collected during resolve (multi-root, depth-too-deep).
    /// The loader caller is expected to forward these to `tracing::warn!`.
    pub warnings: Vec<String>,
}

impl ResolvedTimeOffsets {
    /// Absolute offset for a given `(module, channel)`.
    ///
    /// Returns `Some(0.0)` for entries explicitly declared with offset 0
    /// (e.g. roots); `None` for channels not present in the resolved set.
    /// Callers may treat `None` as "0 with a warn" — see SPEC § 4.3.3.
    #[inline]
    pub fn get(&self, module: u8, channel: u8) -> Option<f64> {
        self.offsets.get(&(module, channel)).copied()
    }

    /// Same as [`get`] but defaults to `0.0` if the channel is missing.
    /// Use this on the hot path; warn separately at load time.
    #[inline]
    pub fn get_or_zero(&self, module: u8, channel: u8) -> f64 {
        self.offsets.get(&(module, channel)).copied().unwrap_or(0.0)
    }

    /// The root `(mod, ch)` of the timing domain this channel belongs to.
    #[inline]
    pub fn root_of(&self, module: u8, channel: u8) -> Option<(u8, u8)> {
        self.roots.get(&(module, channel)).copied()
    }

    /// Depth from the root (root itself is 0).
    #[inline]
    pub fn depth_of(&self, module: u8, channel: u8) -> Option<u32> {
        self.depths.get(&(module, channel)).copied()
    }

    /// All resolved channels with their full per-channel info.
    /// Used by the `eb-offsets` CLI to dump a flat table.
    pub fn iter(&self) -> impl Iterator<Item = ResolvedRow> + '_ {
        self.offsets.iter().map(move |(&(m, c), &abs)| ResolvedRow {
            module: m,
            channel: c,
            absolute_offset_ns: abs,
            depth: self.depths.get(&(m, c)).copied().unwrap_or(0),
            root: self.roots.get(&(m, c)).copied().unwrap_or((m, c)),
        })
    }

    /// Number of distinct roots (`> 1` means multiple disconnected timing
    /// domains — already warned at load time).
    pub fn root_count(&self) -> usize {
        self.roots.values().copied().collect::<HashSet<_>>().len()
    }

    /// Convert into the legacy [`TimeCalibration`] shape so existing pipeline
    /// stages (which expect `time_calibration.get_offset(mod, ch)`) keep
    /// working unchanged.
    ///
    /// The reference channel is picked deterministically — if the forest has
    /// exactly one root, that root becomes the reference; otherwise we pick
    /// the lexicographically smallest `(module, channel)` root (the multi-root
    /// case is already warned at resolve time).
    ///
    /// This is meant as a temporary adapter while the rest of the pipeline
    /// is migrated to consume `ResolvedTimeOffsets` directly (SPEC § 4.3).
    pub fn into_time_calibration(self) -> crate::event_builder::config::TimeCalibration {
        use crate::event_builder::config::TimeCalibration;

        let ref_root = {
            let mut roots: Vec<(u8, u8)> = self
                .roots
                .values()
                .copied()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            roots.sort();
            roots.into_iter().next().unwrap_or((0, 0))
        };
        let mut tc = TimeCalibration::new(ref_root.0, ref_root.1);
        for ((m, c), off) in self.offsets {
            tc.set_offset(m, c, off);
        }
        tc
    }
}

/// One row of the resolved flat table (for CLI dump / debug).
#[derive(Debug, Clone, Copy)]
pub struct ResolvedRow {
    pub module: u8,
    pub channel: u8,
    pub absolute_offset_ns: f64,
    pub depth: u32,
    pub root: (u8, u8),
}

#[derive(Error, Debug)]
pub enum TimeOffsetsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("duplicate entry for (module {0}, channel {1})")]
    Duplicate(u8, u8),

    #[error("entry (module {0}, channel {1}) references missing parent (module {2}, channel {3})")]
    DanglingParent(u8, u8, u8, u8),

    #[error("cycle detected involving (module {0}, channel {1})")]
    Cycle(u8, u8),
}

/// Threshold above which a tree-depth warning is emitted.
pub const DEPTH_WARN_THRESHOLD: u32 = 5;

impl TimeOffsetsFile {
    /// Load from a JSON file.
    pub fn load(path: &Path) -> Result<Self, TimeOffsetsError> {
        let content = std::fs::read_to_string(path)?;
        let f: Self = serde_json::from_str(&content)?;
        Ok(f)
    }

    /// Save to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), TimeOffsetsError> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Resolve the forest of entries into flat lookup tables.
    ///
    /// Performs structural validation (errors on cycle / dangling /
    /// duplicate). Soft conditions (multi-root, deep tree) are collected
    /// into [`ResolvedTimeOffsets::warnings`].
    pub fn resolve(&self) -> Result<ResolvedTimeOffsets, TimeOffsetsError> {
        // Map (mod, ch) -> &entry, rejecting duplicates.
        let mut by_key: HashMap<(u8, u8), &TimeOffsetEntry> = HashMap::new();
        for e in &self.entries {
            if by_key.insert((e.module, e.channel), e).is_some() {
                return Err(TimeOffsetsError::Duplicate(e.module, e.channel));
            }
        }

        // Validate parent references exist (or are None).
        for e in &self.entries {
            if let Some(p) = e.parent {
                if !by_key.contains_key(&(p.module, p.channel)) {
                    return Err(TimeOffsetsError::DanglingParent(
                        e.module, e.channel, p.module, p.channel,
                    ));
                }
            }
        }

        // DFS from each entry, walking parents until a root is reached or a
        // cycle is detected. We memoize the per-(mod, ch) absolute offset and
        // root once computed.
        let mut offsets: HashMap<(u8, u8), f64> = HashMap::new();
        let mut roots: HashMap<(u8, u8), (u8, u8)> = HashMap::new();
        let mut depths: HashMap<(u8, u8), u32> = HashMap::new();

        for &start in by_key.keys() {
            if offsets.contains_key(&start) {
                continue;
            }
            // Walk up until we hit something memoized or a root, recording
            // the chain so we can fill in everything afterwards.
            let mut chain: Vec<(u8, u8)> = Vec::new();
            let mut seen: HashSet<(u8, u8)> = HashSet::new();
            let mut cur = start;
            let (base_offset, base_root, base_depth) = loop {
                if let (Some(&abs), Some(&r), Some(&d)) =
                    (offsets.get(&cur), roots.get(&cur), depths.get(&cur))
                {
                    break (abs, r, d);
                }
                if !seen.insert(cur) {
                    return Err(TimeOffsetsError::Cycle(cur.0, cur.1));
                }
                chain.push(cur);
                let entry = by_key[&cur];
                match entry.parent {
                    None => {
                        // Root. Its offset is its declared own offset_ns,
                        // its root is itself, depth = 0.
                        offsets.insert(cur, entry.offset_ns);
                        roots.insert(cur, cur);
                        depths.insert(cur, 0);
                        break (entry.offset_ns, cur, 0);
                    }
                    Some(p) => {
                        cur = (p.module, p.channel);
                    }
                }
            };

            // Fill in offsets for the chain (skipping the last element if
            // it was memoized — that's `cur` after the loop with the
            // base values already in the maps).
            let mut acc = base_offset;
            let mut depth = base_depth;
            // chain is ordered child-to-ancestor; iterate in reverse to
            // accumulate from ancestor down.
            for &node in chain.iter().rev() {
                let entry = by_key[&node];
                if entry.parent.is_some() {
                    depth += 1;
                    acc += entry.offset_ns;
                }
                offsets.entry(node).or_insert(acc);
                roots.entry(node).or_insert(base_root);
                depths.entry(node).or_insert(depth);
            }
        }

        let mut warnings: Vec<String> = Vec::new();

        let root_set: HashSet<(u8, u8)> = roots.values().copied().collect();
        if root_set.len() > 1 {
            warnings.push(format!(
                "timeSettings: {} disconnected timing domains found (roots: {:?})",
                root_set.len(),
                {
                    let mut v: Vec<_> = root_set.iter().copied().collect();
                    v.sort();
                    v
                }
            ));
        }

        let max_depth = depths.values().copied().max().unwrap_or(0);
        if max_depth > DEPTH_WARN_THRESHOLD {
            warnings.push(format!(
                "timeSettings: tree depth {max_depth} exceeds soft limit \
                 {DEPTH_WARN_THRESHOLD} — drift accumulation along long chains \
                 may be significant"
            ));
        }

        Ok(ResolvedTimeOffsets {
            offsets,
            roots,
            depths,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(module: u8, channel: u8, parent: Option<(u8, u8)>, offset_ns: f64) -> TimeOffsetEntry {
        TimeOffsetEntry {
            module,
            channel,
            parent: parent.map(|(m, c)| ParentRef {
                module: m,
                channel: c,
            }),
            offset_ns,
        }
    }

    #[test]
    fn single_root_resolves() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![
                entry(9, 0, None, 0.0),
                entry(9, 1, Some((9, 0)), 0.05),
                entry(0, 0, Some((9, 0)), 46.75),
                entry(5, 0, Some((0, 0)), 12.3),
            ],
        };
        let r = f.resolve().unwrap();

        assert_eq!(r.get(9, 0), Some(0.0));
        assert_eq!(r.get(9, 1), Some(0.05));
        assert_eq!(r.get(0, 0), Some(46.75));
        // 5/0 absolute = 0 + 46.75 + 12.3
        let abs = r.get(5, 0).unwrap();
        assert!((abs - 59.05).abs() < 1e-9, "got {abs}");
        assert_eq!(r.root_count(), 1);
        assert_eq!(r.root_of(5, 0), Some((9, 0)));
        assert_eq!(r.depth_of(5, 0), Some(2));
        assert_eq!(r.depth_of(9, 0), Some(0));
        assert!(r.warnings.is_empty(), "no warnings expected");
    }

    #[test]
    fn missing_channel_returns_none() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![entry(0, 0, None, 0.0)],
        };
        let r = f.resolve().unwrap();
        assert_eq!(r.get(7, 7), None);
        assert_eq!(r.get_or_zero(7, 7), 0.0);
    }

    #[test]
    fn duplicate_entries_rejected() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![entry(0, 0, None, 0.0), entry(0, 0, None, 1.0)],
        };
        let e = f.resolve().unwrap_err();
        assert!(matches!(e, TimeOffsetsError::Duplicate(0, 0)), "got {e:?}");
    }

    #[test]
    fn dangling_parent_rejected() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![
                entry(0, 0, None, 0.0),
                entry(1, 0, Some((9, 9)), 1.0), // 9/9 does not exist
            ],
        };
        let e = f.resolve().unwrap_err();
        assert!(
            matches!(e, TimeOffsetsError::DanglingParent(1, 0, 9, 9)),
            "got {e:?}"
        );
    }

    #[test]
    fn cycle_rejected() {
        // 0/0 -> 1/0 -> 0/0
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![
                entry(0, 0, Some((1, 0)), 1.0),
                entry(1, 0, Some((0, 0)), 1.0),
            ],
        };
        let e = f.resolve().unwrap_err();
        assert!(matches!(e, TimeOffsetsError::Cycle(_, _)), "got {e:?}");
    }

    #[test]
    fn multi_root_warns() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![
                entry(0, 0, None, 0.0),
                entry(0, 1, Some((0, 0)), 1.0),
                entry(5, 0, None, 0.0),
                entry(5, 1, Some((5, 0)), 2.0),
            ],
        };
        let r = f.resolve().unwrap();
        assert_eq!(r.root_count(), 2);
        assert!(
            r.warnings
                .iter()
                .any(|w| w.contains("disconnected timing domains")),
            "warnings: {:?}",
            r.warnings
        );
    }

    #[test]
    fn deep_tree_warns() {
        // Build a chain 0/0 -> 0/1 -> 0/2 -> ... -> 0/7 (depth 7)
        let mut entries = vec![entry(0, 0, None, 0.0)];
        for i in 1..=7u8 {
            entries.push(entry(0, i, Some((0, i - 1)), 1.0));
        }
        let r = TimeOffsetsFile {
            version: "1.0".into(),
            entries,
        }
        .resolve()
        .unwrap();
        assert_eq!(r.depth_of(0, 7), Some(7));
        assert!(
            r.warnings.iter().any(|w| w.contains("tree depth")),
            "warnings: {:?}",
            r.warnings
        );
    }

    #[test]
    fn parses_full_example() {
        let json = r#"
        {
          "version": "1.0",
          "entries": [
            {"module": 9, "channel": 0, "ref": null,   "offset_ns": 0.0},
            {"module": 9, "channel": 1, "ref": [9, 0], "offset_ns": 0.05},
            {"module": 0, "channel": 0, "ref": [9, 0], "offset_ns": 46.75},
            {"module": 5, "channel": 0, "ref": [0, 0], "offset_ns": 12.3}
          ]
        }
        "#;
        let f: TimeOffsetsFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.entries.len(), 4);
        let r = f.resolve().unwrap();
        assert!((r.get(5, 0).unwrap() - 59.05).abs() < 1e-9);
    }

    #[test]
    fn root_offset_is_applied() {
        // A non-zero offset_ns on a root means "the root's own timestamp
        // needs to be shifted by this amount". This is unusual (typically
        // root offset is 0) but should still resolve correctly.
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![entry(0, 0, None, 100.0), entry(0, 1, Some((0, 0)), 5.0)],
        };
        let r = f.resolve().unwrap();
        assert_eq!(r.get(0, 0), Some(100.0));
        assert_eq!(r.get(0, 1), Some(105.0));
    }

    #[test]
    fn iter_yields_all_entries() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![
                entry(0, 0, None, 0.0),
                entry(0, 1, Some((0, 0)), 5.0),
                entry(0, 2, Some((0, 0)), 10.0),
            ],
        };
        let r = f.resolve().unwrap();
        let rows: Vec<_> = r.iter().collect();
        assert_eq!(rows.len(), 3);
        for row in rows {
            assert_eq!(row.root, (0, 0));
            assert!(row.absolute_offset_ns >= 0.0);
        }
    }

    #[test]
    fn into_time_calibration_carries_offsets() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![
                entry(9, 0, None, 0.0),
                entry(9, 1, Some((9, 0)), 0.05),
                entry(0, 0, Some((9, 0)), 46.75),
                entry(5, 0, Some((0, 0)), 12.3),
            ],
        };
        let r = f.resolve().unwrap();
        let tc = r.into_time_calibration();
        // Ref = single root (9, 0).
        assert_eq!(tc.ref_module, 9);
        assert_eq!(tc.ref_channel, 0);
        assert_eq!(tc.get_offset(9, 0), 0.0);
        assert!((tc.get_offset(9, 1) - 0.05).abs() < 1e-9);
        assert!((tc.get_offset(0, 0) - 46.75).abs() < 1e-9);
        // 5/0 absolute = 0 + 46.75 + 12.3 = 59.05
        assert!((tc.get_offset(5, 0) - 59.05).abs() < 1e-9);
        // Unknown channel defaults to 0.
        assert_eq!(tc.get_offset(7, 7), 0.0);
    }

    #[test]
    fn into_time_calibration_picks_min_root_for_multi_root() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![
                entry(5, 0, None, 0.0),
                entry(0, 0, None, 0.0),
                entry(0, 1, Some((0, 0)), 1.0),
            ],
        };
        let r = f.resolve().unwrap();
        let tc = r.into_time_calibration();
        // The smaller (module, channel) tuple wins → (0, 0)
        assert_eq!((tc.ref_module, tc.ref_channel), (0, 0));
    }

    #[test]
    fn save_and_load_round_trip() {
        let f = TimeOffsetsFile {
            version: "1.0".into(),
            entries: vec![entry(9, 0, None, 0.0), entry(9, 1, Some((9, 0)), 0.05)],
        };
        let path =
            std::env::temp_dir().join(format!("time_offsets_test_{}.json", std::process::id()));
        f.save(&path).unwrap();
        let loaded = TimeOffsetsFile::load(&path).unwrap();
        assert_eq!(f, loaded);
        let _ = std::fs::remove_file(&path);
    }
}

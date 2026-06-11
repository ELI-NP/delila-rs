//! Skeleton generators for EB config files.
//!
//! Lets `event_builder init …` produce a structurally-correct chSettings.json
//! or timeSettings.json from a handful of CLI flags. The goal is to remove
//! the "open editor and write 200 channels by hand" pain — users fill in
//! detector types, tags, and calibrations afterwards.

use crate::event_builder::{
    time_offsets::{ParentRef, TimeOffsetEntry, TimeOffsetsFile},
    ChSettings, ChannelConfig,
};
use anyhow::{anyhow, Context, Result};

/// Per-module override parsed from `"M:DetectorType:tag1,tag2,..."`.
///
/// The tag list is optional — `"3:HPGe"` parses to `Tags = []`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleTypeOverride {
    pub module: u8,
    pub detector_type: String,
    pub tags: Vec<String>,
}

/// Parse a `--module-type` CLI arg.
///
/// Forms accepted:
/// - `"0:HPGe"` → DetectorType=HPGe, Tags=[]
/// - `"4:Si:E_Sector"` → DetectorType=Si, Tags=["E_Sector"]
/// - `"4:Si:E_Sector,Si"` → DetectorType=Si, Tags=["E_Sector", "Si"]
pub fn parse_module_type_spec(s: &str) -> Result<ModuleTypeOverride> {
    let mut parts = s.splitn(3, ':');
    let module_s = parts.next().ok_or_else(|| anyhow!("empty spec"))?;
    let detector_type = parts.next().ok_or_else(|| {
        anyhow!("missing DetectorType in '{s}' — expected 'M:DetectorType[:tag1,tag2,...]'")
    })?;
    let tags_s = parts.next().unwrap_or("");

    let module: u8 = module_s
        .parse()
        .with_context(|| format!("module number must be 0–255: '{module_s}'"))?;
    let tags: Vec<String> = if tags_s.is_empty() {
        Vec::new()
    } else {
        tags_s.split(',').map(|t| t.trim().to_string()).collect()
    };

    Ok(ModuleTypeOverride {
        module,
        detector_type: detector_type.to_string(),
        tags,
    })
}

/// Generate a `chSettings.json` skeleton with `modules × channels` entries.
///
/// Channels covered by a `--module-type` override inherit that module's
/// DetectorType and Tags; everyone else gets `"Unknown"` / `[]`.
/// Calibration is identity (`p0=0, p1=1`).
pub fn build_chsettings_skeleton(
    modules: u8,
    channels: u8,
    overrides: &[ModuleTypeOverride],
) -> ChannelConfig {
    (0..modules)
        .map(|m| {
            let ovr = overrides.iter().find(|o| o.module == m);
            let (detector_type, tags) = match ovr {
                Some(o) => (o.detector_type.clone(), o.tags.clone()),
                None => ("Unknown".to_string(), Vec::new()),
            };
            (0..channels)
                .map(|c| ChSettings {
                    module: m,
                    channel: c,
                    detector_type: detector_type.clone(),
                    tags: tags.clone(),
                    p0: 0.0,
                    p1: 1.0,
                    p2: 0.0,
                    p3: 0.0,
                })
                .collect()
        })
        .collect()
}

/// Generate a flat-tree `timeSettings.json` where every channel points at
/// `root` with `offset_ns = 0`. Real offsets come from `event_builder
/// time-calib` afterwards — this just produces a valid placeholder.
///
/// `channels` is the explicit list of `(module, channel)` that should appear
/// in the file. The root entry itself is included with `parent = None`.
pub fn build_timesettings_skeleton(
    channels: &[(u8, u8)],
    root: (u8, u8),
) -> Result<TimeOffsetsFile> {
    if !channels.contains(&root) {
        return Err(anyhow!(
            "root channel ({}, {}) not in the supplied channel list",
            root.0,
            root.1
        ));
    }

    let entries = channels
        .iter()
        .map(|&(m, c)| TimeOffsetEntry {
            module: m,
            channel: c,
            parent: if (m, c) == root {
                None
            } else {
                Some(ParentRef {
                    module: root.0,
                    channel: root.1,
                })
            },
            offset_ns: 0.0,
        })
        .collect();

    Ok(TimeOffsetsFile {
        version: "1.0".to_string(),
        entries,
    })
}

/// Convenience: enumerate `(module, channel)` pairs from a `ChannelConfig`.
pub fn channels_from_chsettings(cfg: &ChannelConfig) -> Vec<(u8, u8)> {
    cfg.iter()
        .flat_map(|m| m.iter().map(|c| (c.module, c.channel)))
        .collect()
}

/// Parse `"M:C"` into `(module, channel)`. Used by both `init timesettings`
/// and a handful of existing flags (`--trigger M:C`), but kept here so the
/// init module is self-contained.
pub fn parse_module_channel(s: &str) -> Result<(u8, u8)> {
    let (m, c) = s
        .split_once(':')
        .ok_or_else(|| anyhow!("expected 'M:C', got '{s}'"))?;
    let module: u8 = m.parse().with_context(|| format!("bad module: '{m}'"))?;
    let channel: u8 = c.parse().with_context(|| format!("bad channel: '{c}'"))?;
    Ok((module, channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_module_type_spec_minimal() {
        let o = parse_module_type_spec("0:HPGe").unwrap();
        assert_eq!(o.module, 0);
        assert_eq!(o.detector_type, "HPGe");
        assert!(o.tags.is_empty());
    }

    #[test]
    fn parse_module_type_spec_with_tags() {
        let o = parse_module_type_spec("4:Si:E_Sector,Si").unwrap();
        assert_eq!(o.module, 4);
        assert_eq!(o.detector_type, "Si");
        assert_eq!(o.tags, vec!["E_Sector", "Si"]);
    }

    #[test]
    fn parse_module_type_spec_trims_tag_whitespace() {
        let o = parse_module_type_spec("4:Si:E_Sector, Si , Trigger").unwrap();
        assert_eq!(o.tags, vec!["E_Sector", "Si", "Trigger"]);
    }

    #[test]
    fn parse_module_type_spec_rejects_missing_detector_type() {
        assert!(parse_module_type_spec("4").is_err());
    }

    #[test]
    fn chsettings_skeleton_uniform() {
        let cfg = build_chsettings_skeleton(3, 16, &[]);
        assert_eq!(cfg.len(), 3);
        assert!(cfg.iter().all(|m| m.len() == 16));
        // Module/channel numbering is dense and zero-based.
        assert_eq!(cfg[1][5].module, 1);
        assert_eq!(cfg[1][5].channel, 5);
        // Defaults.
        assert_eq!(cfg[0][0].detector_type, "Unknown");
        assert!(cfg[0][0].tags.is_empty());
        assert_eq!(cfg[0][0].p1, 1.0);
    }

    #[test]
    fn chsettings_skeleton_applies_overrides() {
        let overrides = vec![
            ModuleTypeOverride {
                module: 0,
                detector_type: "HPGe".to_string(),
                tags: vec!["HPGe".to_string(), "Trigger".to_string()],
            },
            ModuleTypeOverride {
                module: 4,
                detector_type: "Si".to_string(),
                tags: vec!["E_Sector".to_string()],
            },
        ];
        let cfg = build_chsettings_skeleton(8, 16, &overrides);
        // Modules 0 and 4 inherit override.
        assert_eq!(cfg[0][0].detector_type, "HPGe");
        assert_eq!(cfg[0][15].tags, vec!["HPGe", "Trigger"]);
        assert_eq!(cfg[4][0].detector_type, "Si");
        assert_eq!(cfg[4][0].tags, vec!["E_Sector"]);
        // Untouched module falls back to Unknown.
        assert_eq!(cfg[1][0].detector_type, "Unknown");
    }

    #[test]
    fn timesettings_skeleton_flat_tree() {
        let channels = vec![(0, 0), (0, 1), (1, 0), (1, 1)];
        let file = build_timesettings_skeleton(&channels, (0, 0)).unwrap();
        assert_eq!(file.entries.len(), 4);
        // Root has no parent.
        let root_entry = file
            .entries
            .iter()
            .find(|e| (e.module, e.channel) == (0, 0))
            .unwrap();
        assert!(root_entry.parent.is_none());
        assert_eq!(root_entry.offset_ns, 0.0);
        // Non-root points at root.
        let leaf = file
            .entries
            .iter()
            .find(|e| (e.module, e.channel) == (1, 1))
            .unwrap();
        let p = leaf.parent.unwrap();
        assert_eq!((p.module, p.channel), (0, 0));
        assert_eq!(leaf.offset_ns, 0.0);
    }

    #[test]
    fn timesettings_skeleton_resolves_cleanly() {
        // The generated file must pass the same resolve() the loader uses.
        let channels = vec![(0, 0), (0, 1), (1, 0)];
        let file = build_timesettings_skeleton(&channels, (0, 0)).unwrap();
        let resolved = file.resolve().unwrap();
        assert_eq!(resolved.root_count(), 1);
    }

    #[test]
    fn timesettings_skeleton_rejects_root_not_in_channels() {
        let channels = vec![(0, 0), (0, 1)];
        assert!(build_timesettings_skeleton(&channels, (5, 5)).is_err());
    }

    #[test]
    fn channels_from_chsettings_flattens_in_order() {
        let cfg = build_chsettings_skeleton(2, 3, &[]);
        let ch = channels_from_chsettings(&cfg);
        assert_eq!(ch, vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)]);
    }

    #[test]
    fn parse_module_channel_works() {
        assert_eq!(parse_module_channel("3:7").unwrap(), (3, 7));
        assert!(parse_module_channel("3").is_err());
        assert!(parse_module_channel("3:abc").is_err());
    }
}

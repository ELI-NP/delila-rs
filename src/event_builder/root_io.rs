//! ROOT TTree I/O for event building
//!
//! oxyroot を使用した ROOT ファイルの読み書き。
//! ELIFANT-Event 互換の TTree フォーマットをサポート。

#[cfg(feature = "root")]
use oxyroot::{RootFile, WriterTree};

use super::built_event::BuiltEvent;
use super::hit::Hit;
use std::path::Path;
use thiserror::Error;

/// ROOT I/O errors
#[derive(Error, Debug)]
pub enum RootError {
    #[error("Failed to open ROOT file: {0}")]
    OpenError(String),

    #[error("Failed to create ROOT file: {0}")]
    CreateError(String),

    #[error("Tree not found: {0}")]
    TreeNotFound(String),

    #[error("Branch not found: {0}")]
    BranchNotFound(String),

    #[error("Write error: {0}")]
    WriteError(String),

    #[error("ROOT feature not enabled")]
    FeatureNotEnabled,
}

/// Read hits from ELIFANT-Event ROOT TTree
///
/// Expected branch structure (ELIADE_Tree):
/// - Mod: u8 (module ID)
/// - Ch: u8 (channel ID)
/// - FineTS: f64 (timestamp in **picoseconds**)
/// - ChargeLong: u16 (energy long gate)
/// - ChargeShort: u16 (energy short gate)
///
/// Note: FineTS is converted from picoseconds to nanoseconds internally.
#[cfg(feature = "root")]
pub fn read_hits_from_root(path: &Path, tree_name: &str) -> Result<Vec<Hit>, RootError> {
    let mut file = RootFile::open(path.to_str().unwrap_or(""))
        .map_err(|e| RootError::OpenError(format!("{:?}", e)))?;

    let tree = file
        .get_tree(tree_name)
        .map_err(|e| RootError::TreeNotFound(format!("{}: {:?}", tree_name, e)))?;

    // Get branch iterators
    let mod_iter = tree
        .branch("Mod")
        .ok_or_else(|| RootError::BranchNotFound("Mod".to_string()))?
        .as_iter::<u8>()
        .map_err(|e| RootError::BranchNotFound(format!("Mod: {:?}", e)))?;

    let ch_iter = tree
        .branch("Ch")
        .ok_or_else(|| RootError::BranchNotFound("Ch".to_string()))?
        .as_iter::<u8>()
        .map_err(|e| RootError::BranchNotFound(format!("Ch: {:?}", e)))?;

    let ts_iter = tree
        .branch("FineTS")
        .ok_or_else(|| RootError::BranchNotFound("FineTS".to_string()))?
        .as_iter::<f64>()
        .map_err(|e| RootError::BranchNotFound(format!("FineTS: {:?}", e)))?;

    let energy_iter = tree
        .branch("ChargeLong")
        .ok_or_else(|| RootError::BranchNotFound("ChargeLong".to_string()))?
        .as_iter::<u16>()
        .map_err(|e| RootError::BranchNotFound(format!("ChargeLong: {:?}", e)))?;

    let energy_short_iter = tree
        .branch("ChargeShort")
        .ok_or_else(|| RootError::BranchNotFound("ChargeShort".to_string()))?
        .as_iter::<u16>()
        .map_err(|e| RootError::BranchNotFound(format!("ChargeShort: {:?}", e)))?;

    // Zip all iterators and collect hits
    // Note: FineTS is in picoseconds, convert to nanoseconds
    let hits: Vec<Hit> = mod_iter
        .zip(ch_iter)
        .zip(ts_iter)
        .zip(energy_iter)
        .zip(energy_short_iter)
        .map(|((((module, channel), ts_ps), energy), energy_short)| {
            let ts_ns = ts_ps / 1000.0; // Convert ps to ns
            Hit::new(module, channel, energy, energy_short, ts_ns)
        })
        .collect();

    Ok(hits)
}

#[cfg(not(feature = "root"))]
pub fn read_hits_from_root(_path: &Path, _tree_name: &str) -> Result<Vec<Hit>, RootError> {
    Err(RootError::FeatureNotEnabled)
}

/// Write built events to ROOT TTree
///
/// Output branch structure:
/// - EventID: u64
/// - TriggerTime: f64
/// - TriggerMod: u8
/// - TriggerCh: u8
/// - Multiplicity: u32
/// - Mod: Vec<u8>
/// - Ch: Vec<u8>
/// - Energy: Vec<u16>
/// - EnergyShort: Vec<u16>
/// - RelTime: Vec<f64>
/// - WithAC: Vec<u8> (0 or 1)
#[cfg(feature = "root")]
pub fn write_events_to_root(
    path: &Path,
    tree_name: &str,
    events: &[BuiltEvent],
) -> Result<(), RootError> {
    // Flatten events into per-entry vectors for TTree branches
    let event_ids: Vec<u64> = events.iter().map(|e| e.event_id).collect();
    let trigger_times: Vec<f64> = events.iter().map(|e| e.trigger_time).collect();
    let trigger_mods: Vec<u8> = events.iter().map(|e| e.trigger_module).collect();
    let trigger_chs: Vec<u8> = events.iter().map(|e| e.trigger_channel).collect();
    let multiplicities: Vec<u32> = events.iter().map(|e| e.multiplicity() as u32).collect();

    // Hit arrays per event
    let mods: Vec<Vec<u8>> = events
        .iter()
        .map(|e| e.hits.iter().map(|h| h.module).collect())
        .collect();
    let chs: Vec<Vec<u8>> = events
        .iter()
        .map(|e| e.hits.iter().map(|h| h.channel).collect())
        .collect();
    let energies: Vec<Vec<u16>> = events
        .iter()
        .map(|e| e.hits.iter().map(|h| h.energy).collect())
        .collect();
    let energy_shorts: Vec<Vec<u16>> = events
        .iter()
        .map(|e| e.hits.iter().map(|h| h.energy_short).collect())
        .collect();
    let rel_times: Vec<Vec<f64>> = events
        .iter()
        .map(|e| e.hits.iter().map(|h| h.relative_time).collect())
        .collect();
    let with_acs: Vec<Vec<u8>> = events
        .iter()
        .map(|e| {
            e.hits
                .iter()
                .map(|h| if h.with_ac { 1u8 } else { 0u8 })
                .collect()
        })
        .collect();

    let mut file = RootFile::create(path.to_str().unwrap_or(""))
        .map_err(|e| RootError::CreateError(format!("{:?}", e)))?;

    let mut tree = WriterTree::new(tree_name);

    tree.new_branch("EventID", event_ids.into_iter());
    tree.new_branch("TriggerTime", trigger_times.into_iter());
    tree.new_branch("TriggerMod", trigger_mods.into_iter());
    tree.new_branch("TriggerCh", trigger_chs.into_iter());
    tree.new_branch("Multiplicity", multiplicities.into_iter());
    tree.new_branch("Mod", mods.into_iter());
    tree.new_branch("Ch", chs.into_iter());
    tree.new_branch("Energy", energies.into_iter());
    tree.new_branch("EnergyShort", energy_shorts.into_iter());
    tree.new_branch("RelTime", rel_times.into_iter());
    tree.new_branch("WithAC", with_acs.into_iter());

    tree.write(&mut file)
        .map_err(|e| RootError::WriteError(format!("{:?}", e)))?;

    file.close()
        .map_err(|e| RootError::WriteError(format!("{:?}", e)))?;

    Ok(())
}

#[cfg(not(feature = "root"))]
pub fn write_events_to_root(
    _path: &Path,
    _tree_name: &str,
    _events: &[BuiltEvent],
) -> Result<(), RootError> {
    Err(RootError::FeatureNotEnabled)
}

/// Write raw hits to ROOT TTree (for time calibration input)
///
/// Output branch structure matches ELIFANT input format:
/// - Mod: u8
/// - Ch: u8
/// - FineTS: f64 (in **picoseconds** for ELIFANT compatibility)
/// - ChargeLong: u16
/// - ChargeShort: u16
///
/// Note: timestamp_ns is converted to picoseconds for output.
#[cfg(feature = "root")]
pub fn write_hits_to_root(path: &Path, tree_name: &str, hits: &[Hit]) -> Result<(), RootError> {
    let mods: Vec<u8> = hits.iter().map(|h| h.module).collect();
    let chs: Vec<u8> = hits.iter().map(|h| h.channel).collect();
    // Convert ns to ps for ELIFANT compatibility
    let timestamps: Vec<f64> = hits.iter().map(|h| h.timestamp_ns * 1000.0).collect();
    let energies: Vec<u16> = hits.iter().map(|h| h.energy).collect();
    let energy_shorts: Vec<u16> = hits.iter().map(|h| h.energy_short).collect();

    let mut file = RootFile::create(path.to_str().unwrap_or(""))
        .map_err(|e| RootError::CreateError(format!("{:?}", e)))?;

    let mut tree = WriterTree::new(tree_name);

    tree.new_branch("Mod", mods.into_iter());
    tree.new_branch("Ch", chs.into_iter());
    tree.new_branch("FineTS", timestamps.into_iter());
    tree.new_branch("ChargeLong", energies.into_iter());
    tree.new_branch("ChargeShort", energy_shorts.into_iter());

    tree.write(&mut file)
        .map_err(|e| RootError::WriteError(format!("{:?}", e)))?;

    file.close()
        .map_err(|e| RootError::WriteError(format!("{:?}", e)))?;

    Ok(())
}

#[cfg(not(feature = "root"))]
pub fn write_hits_to_root(_path: &Path, _tree_name: &str, _hits: &[Hit]) -> Result<(), RootError> {
    Err(RootError::FeatureNotEnabled)
}

#[cfg(all(test, feature = "root"))]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_hit(module: u8, channel: u8, ts: f64) -> Hit {
        Hit::new(module, channel, 1000, 500, ts)
    }

    #[test]
    fn test_write_and_read_hits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_hits.root");

        // Create test hits
        let hits = vec![
            make_hit(0, 0, 1000.0),
            make_hit(0, 1, 1100.0),
            make_hit(1, 0, 1200.0),
        ];

        // Write to ROOT file
        write_hits_to_root(&path, "test_tree", &hits).unwrap();

        // Read back
        let read_hits = read_hits_from_root(&path, "test_tree").unwrap();

        assert_eq!(read_hits.len(), 3);
        assert_eq!(read_hits[0].module, 0);
        assert_eq!(read_hits[0].channel, 0);
        assert_eq!(read_hits[0].timestamp_ns, 1000.0);
        assert_eq!(read_hits[1].module, 0);
        assert_eq!(read_hits[1].channel, 1);
        assert_eq!(read_hits[2].module, 1);
        assert_eq!(read_hits[2].channel, 0);
    }

    #[test]
    fn test_write_events() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_events.root");

        // Create test event
        let trigger = make_hit(0, 0, 1000.0);
        let mut event = BuiltEvent::new(1, &trigger);
        event.add_hit(&make_hit(0, 1, 1050.0));
        event.add_hit(&make_hit(1, 0, 1100.0));

        let events = vec![event];

        // Write to ROOT file
        write_events_to_root(&path, "events", &events).unwrap();

        // Verify file exists
        assert!(path.exists());
    }
}

#[cfg(all(test, not(feature = "root")))]
mod tests_no_root {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_feature_not_enabled() {
        let path = PathBuf::from("dummy.root");
        let result = read_hits_from_root(&path, "tree");
        assert!(matches!(result, Err(RootError::FeatureNotEnabled)));
    }
}

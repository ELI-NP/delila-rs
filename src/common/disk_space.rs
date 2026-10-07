//! Free-space probe for writer output directories (TODO 69 Phase 2).
//!
//! A full disk used to be noticed only when a write failed — by then the run
//! had already lost data. The Recorder reports the free space of its
//! `output_dir` filesystem in [`crate::common::ComponentMetrics`] and grades
//! it against soft/hard watermarks; the Operator's run guard answers the hard
//! level with a drain-first stop (the disk is still writable, so the backlog
//! is written and the file closed with its footer) and refuses Start below it.

use std::path::Path;

/// Bytes available to an unprivileged writer on the filesystem holding
/// `path` (`statvfs`: `f_bavail × f_frsize` — root-reserved blocks excluded,
/// which is what a non-root Recorder can actually use).
///
/// The output directory is created lazily at the first file of a run, so a
/// missing `path` is resolved to its nearest existing ancestor (same
/// filesystem in every sane layout).
pub fn free_bytes(path: &Path) -> std::io::Result<u64> {
    let mut probe = path;
    while !probe.exists() {
        match probe.parent() {
            Some(p) if !p.as_os_str().is_empty() => probe = p,
            // A bare relative name ("data") lives in the working directory.
            _ => {
                probe = Path::new(".");
                break;
            }
        }
    }
    statvfs_free(probe)
}

#[cfg(unix)]
fn statvfs_free(path: &Path) -> std::io::Result<u64> {
    let st = rustix::fs::statvfs(path)?;
    Ok(st.f_bavail.saturating_mul(st.f_frsize))
}

#[cfg(not(unix))]
fn statvfs_free(_path: &Path) -> std::io::Result<u64> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "free-space probe needs statvfs (unix only)",
    ))
}

/// Grade free space against the watermarks:
///
/// - `0` — OK (at/above soft, or soft disabled)
/// - `1` — below the soft watermark (warn; run keeps going)
/// - `2` — below the hard watermark (drain-first stop, Start refused)
///
/// A watermark of `0` disables it. A misconfigured `hard > soft` still
/// reports `2` below hard — the more severe verdict wins (mirrors
/// `queue_accounting::backlog_level`).
pub fn disk_level(free_bytes: u64, soft_free_bytes: u64, hard_free_bytes: u64) -> u8 {
    if hard_free_bytes > 0 && free_bytes < hard_free_bytes {
        2
    } else if soft_free_bytes > 0 && free_bytes < soft_free_bytes {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_level_boundaries() {
        let (soft, hard) = (200, 100);
        assert_eq!(disk_level(u64::MAX, soft, hard), 0);
        assert_eq!(disk_level(200, soft, hard), 0, "at soft = still OK");
        assert_eq!(disk_level(199, soft, hard), 1);
        assert_eq!(disk_level(100, soft, hard), 1, "at hard = still writable");
        assert_eq!(disk_level(99, soft, hard), 2);
        assert_eq!(disk_level(0, soft, hard), 2);
    }

    #[test]
    fn disk_level_zero_disables_a_watermark() {
        assert_eq!(disk_level(0, 0, 0), 0, "both disabled");
        assert_eq!(disk_level(50, 0, 100), 2, "soft disabled, hard live");
        assert_eq!(disk_level(150, 0, 100), 0, "soft disabled above hard");
        assert_eq!(disk_level(0, 100, 0), 1, "hard disabled caps at 1");
    }

    #[test]
    fn disk_level_hard_above_soft_still_reports_hard() {
        assert_eq!(disk_level(150, 100, 200), 2);
    }

    #[cfg(unix)]
    #[test]
    fn free_bytes_of_existing_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let here = free_bytes(dir.path()).unwrap();
        assert!(here > 0, "a temp dir has some free space");
        // Not created yet (the Recorder creates output_dir lazily): resolves
        // to the nearest existing ancestor on the same filesystem.
        let missing = dir.path().join("not/yet/created");
        let there = free_bytes(&missing).unwrap();
        let tolerance = 64 * 1024 * 1024; // other writers on the host
        assert!(here.abs_diff(there) < tolerance, "{here} vs {there}");
    }
}

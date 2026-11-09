//! Clock (PLL) health of a digitizer (TODO 85), polled by the RAW read loop.
//!
//! A digitizer whose PLL is not locked does not take valid data. On pf-qaq
//! (2026-11-08) a loose cable in the V1725 clock daisy chain took mod2-5 down
//! for 5 h while the run went on. The Reader reports the status in its
//! metrics; the Operator stops the run when it stays bad
//! (`operator::digitizer_guard`).
//!
//! Sources:
//! - DIG1 (x725/x730, PSD1/PHA1): `/par/plllocked`, "PLL locked", TRUE/FALSE
//!   (DevTree, `docs/devtree_examples/dt5730b_pha1_sn990.json`). The register
//!   behind it is Acquisition Status 0x8104 bit 7, PLL Unlock Detect,
//!   UM5678 rev5 p.45: 0 = an unlock happened since the last read access
//!   (sticky). The same page notes a temporary loss of lock during
//!   daisy-chain initialization, so one FALSE is not a fault by itself.
//! - DIG2 RAW (x27xx, PSD2/PHA2): `/par/errorflags`, "x2730 DPP-PSD Supported
//!   Parameters" (CUP 2024092000, `legacy/documentation_2024092000-2/a00103.html`):
//!   bit 2 si5341_unlock, 3 si5395_unlock, 4 LMK04832_unlock, 5 jesd_unlock,
//!   15 clkin_miss. Bit 15 counts only when the board takes an external
//!   clock (`/par/clocksource` not `Internal`).

use crate::common::PllStatus;

use super::caen::CaenError;

/// How often the read loop reads the PLL status.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// DIG1 parameter (DevTree name, lowercase as in the parameter cache).
pub const DIG1_PARAM: &str = "plllocked";
/// DIG2 parameter (DevTree name, lowercase as in the parameter cache).
pub const DIG2_PARAM: &str = "errorflags";

/// DIG2 `ErrorFlags` bits that mean the clock is not usable whatever the
/// clock source: the three PLL chips and the JESD link to the ADCs.
pub const DIG2_UNLOCK_MASK: u32 = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5);
/// DIG2 `ErrorFlags` bit 15 clkin_miss: no clock on CLK-IN.
pub const DIG2_CLKIN_MISS: u32 = 1 << 15;

/// Status from a DIG1 `/par/plllocked` read.
pub fn dig1_status(reading: &Result<String, CaenError>) -> PllStatus {
    match reading {
        Ok(v) => match v.trim().to_ascii_uppercase().as_str() {
            "TRUE" | "1" => PllStatus::Locked,
            "FALSE" | "0" => PllStatus::Unlocked,
            _ => PllStatus::Unreadable,
        },
        Err(_) => PllStatus::Unreadable,
    }
}

/// Parse a DIG2 32-bit status word ("0", "32772" or "0x8004").
pub fn parse_word(value: &str) -> Option<u32> {
    let v = value.trim();
    match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => v.parse::<u32>().ok(),
    }
}

/// Whether a DIG2 `/par/clocksource` value is an external clock.
pub fn is_external_clock(clocksource: &str) -> bool {
    !clocksource.trim().eq_ignore_ascii_case("internal")
}

/// Status from a DIG2 `/par/errorflags` read. `external_clock` is consulted
/// only when bit 15 (clkin_miss) is set.
pub fn dig2_status(
    reading: &Result<String, CaenError>,
    external_clock: impl FnOnce() -> bool,
) -> PllStatus {
    let Some(flags) = reading.as_ref().ok().and_then(|v| parse_word(v)) else {
        return PllStatus::Unreadable;
    };
    if flags & DIG2_UNLOCK_MASK != 0 || (flags & DIG2_CLKIN_MISS != 0 && external_clock()) {
        PllStatus::Unlocked
    } else {
        PllStatus::Locked
    }
}

/// Remembers the last status so the read loop logs each change once.
#[derive(Debug, Default)]
pub struct PllWatch {
    last: Option<Option<PllStatus>>,
}

impl PllWatch {
    /// Record a new reading. Returns `Some(new)` when it differs from the
    /// previous one (the caller logs it), `None` when unchanged.
    pub fn changed(&mut self, status: Option<PllStatus>) -> Option<Option<PllStatus>> {
        if self.last == Some(status) {
            return None;
        }
        self.last = Some(status);
        Some(status)
    }
}

/// Read the PLL status of a connected digitizer. `None` when its firmware
/// has no status parameter (not in the DevTree, or a firmware family this
/// module does not know). Without a parameter cache the read is attempted.
pub(crate) fn read(
    conn: &super::DeviceConnection,
    firmware: crate::config::digitizer::FirmwareType,
) -> Option<PllStatus> {
    use crate::config::digitizer::FirmwareType;
    let has = |name: &str| {
        conn.param_cache
            .as_ref()
            .is_none_or(|c| c.contains_key(name))
    };
    if firmware.is_dig1() {
        if !has(DIG1_PARAM) {
            return None;
        }
        return Some(dig1_status(&conn.handle.get_value("/par/plllocked")));
    }
    if matches!(firmware, FirmwareType::PSD2 | FirmwareType::PHA2) {
        if !has(DIG2_PARAM) {
            return None;
        }
        let reading = conn.handle.get_value("/par/errorflags");
        return Some(dig2_status(&reading, || {
            // Unreadable source → assume external: a clkin_miss we cannot
            // explain must not pass as a locked clock.
            conn.handle
                .get_value("/par/clocksource")
                .map(|v| is_external_clock(&v))
                .unwrap_or(true)
        }));
    }
    None
}

/// Log a status change. Unlocked and Unreadable are warnings (silent-failure
/// rule): the board takes no valid data in either state.
pub fn log_change(status: Option<PllStatus>, connected: bool) {
    use tracing::{info, warn};
    match status {
        Some(PllStatus::Locked) => info!("PLL locked (digitizer clock OK)"),
        Some(PllStatus::Unlocked) => warn!(
            "PLL NOT locked — the digitizer takes no valid data; check the clock \
             (CLK-IN cable of the daisy chain, clock source). The Operator stops a run \
             when this persists"
        ),
        Some(PllStatus::Unreadable) if connected => {
            warn!("PLL status unreadable — the digitizer does not answer (lost clock or link?)")
        }
        Some(PllStatus::Unreadable) => {
            warn!("PLL status unknown — digitizer not connected")
        }
        None => info!("No PLL status available for this firmware"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(v: &str) -> Result<String, CaenError> {
        Ok(v.to_string())
    }

    fn err() -> Result<String, CaenError> {
        Err(CaenError {
            code: -6,
            name: "CommandError".into(),
            description: "Command error".into(),
        })
    }

    #[test]
    fn dig1_reads_the_devtree_values() {
        assert_eq!(dig1_status(&ok("TRUE")), PllStatus::Locked);
        assert_eq!(dig1_status(&ok("true")), PllStatus::Locked);
        assert_eq!(dig1_status(&ok("FALSE")), PllStatus::Unlocked);
        assert_eq!(dig1_status(&ok(" 1 ")), PllStatus::Locked);
        assert_eq!(dig1_status(&ok("0")), PllStatus::Unlocked);
    }

    #[test]
    fn dig1_unexpected_value_or_error_is_unreadable_not_locked() {
        assert_eq!(dig1_status(&ok("MAYBE")), PllStatus::Unreadable);
        assert_eq!(dig1_status(&ok("")), PllStatus::Unreadable);
        assert_eq!(dig1_status(&err()), PllStatus::Unreadable);
    }

    #[test]
    fn dig2_status_word_parses_decimal_and_hex() {
        assert_eq!(parse_word("0"), Some(0));
        assert_eq!(parse_word("32772"), Some(0x8004));
        assert_eq!(parse_word("0x8004"), Some(0x8004));
        assert_eq!(parse_word(" 8 "), Some(8));
        assert_eq!(parse_word("x"), None);
    }

    #[test]
    fn dig2_pll_and_jesd_bits_mean_unlocked() {
        let never = || panic!("clock source must not be read for these bits");
        assert_eq!(dig2_status(&ok("0"), never), PllStatus::Locked);
        for bit in [2, 3, 4, 5] {
            let word = (1u32 << bit).to_string();
            assert_eq!(
                dig2_status(&ok(&word), || false),
                PllStatus::Unlocked,
                "bit {bit}"
            );
        }
        // Over-temperature (bit 11) is not a clock problem.
        assert_eq!(dig2_status(&ok("2048"), || false), PllStatus::Locked);
    }

    #[test]
    fn dig2_clkin_miss_counts_only_with_an_external_clock() {
        let word = DIG2_CLKIN_MISS.to_string();
        assert_eq!(dig2_status(&ok(&word), || false), PllStatus::Locked);
        assert_eq!(dig2_status(&ok(&word), || true), PllStatus::Unlocked);
        assert!(!is_external_clock("Internal"));
        assert!(!is_external_clock("internal"));
        assert!(is_external_clock("FPClkIn"));
    }

    #[test]
    fn dig2_error_or_garbage_is_unreadable() {
        assert_eq!(dig2_status(&err(), || false), PllStatus::Unreadable);
        assert_eq!(dig2_status(&ok("n/a"), || false), PllStatus::Unreadable);
    }

    #[test]
    fn watch_reports_each_change_once() {
        let mut w = PllWatch::default();
        assert_eq!(
            w.changed(Some(PllStatus::Locked)),
            Some(Some(PllStatus::Locked))
        );
        assert_eq!(w.changed(Some(PllStatus::Locked)), None);
        assert_eq!(
            w.changed(Some(PllStatus::Unreadable)),
            Some(Some(PllStatus::Unreadable))
        );
        assert_eq!(w.changed(None), Some(None));
        assert_eq!(w.changed(None), None);
    }
}

//! Clock (PLL) and board health of a digitizer (TODO 85), polled by the RAW
//! read loop in every state ([`PllPoller`]).
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
//!   Parameters" (CUP 2024092000, `legacy/documentation_2024092000-2/a00103.html`),
//!   bits 0-16 named in [`DIG2_FLAG_NAMES`]. Not only the clock: on
//!   2026-11-10 the VX2730 (clock master, internal oscillator) failed a
//!   Configure-time reset and reported a fault that was shown as "PLL not
//!   locked" although its clock is its own (TODO 85 §5.3). So a DIG2 fault
//!   names its bits ("jesd_unlock", ...), the bits that count are
//!   [`DIG2_FAULT_MASK`], and bit 15 clkin_miss counts only with an external
//!   clock (`/par/clocksource` not `Internal`); a clock source that cannot be
//!   read makes the status unreadable, not "unlocked".

use crate::common::PllStatus;

use super::caen::CaenError;

/// How often the read loop reads the status.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// DIG1 parameter (DevTree name, lowercase as in the parameter cache).
pub const DIG1_PARAM: &str = "plllocked";
/// DIG2 parameter (DevTree name, lowercase as in the parameter cache).
pub const DIG2_PARAM: &str = "errorflags";

/// DIG2 `ErrorFlags` bit names, bit 0 to 16 (CUP 2024092000, a00103.html).
pub const DIG2_FLAG_NAMES: [&str; 17] = [
    "power_fail",
    "board_init_fault",
    "si5341_unlock",
    "si5395_unlock",
    "LMK04832_unlock",
    "jesd_unlock",
    "ddr_pl_bank0_calib_fail",
    "ddr_pl_bank1_calib_fail",
    "ddr_ps_calib_fail",
    "fpga_config_fail",
    "bic_error",
    "adc_overtemp",
    "air_overtemp",
    "fpga_overtemp",
    "dcdc_overtemp",
    "clkin_miss",
    "adc_shutdown",
];

/// DIG2 `ErrorFlags` bits that mean the board takes no valid data whatever
/// its clock source (user, 2026-11-10): power_fail, board_init_fault, the
/// three PLL chips, the JESD link to the ADCs, the three DDR calibrations,
/// fpga_config_fail and adc_shutdown. Not counted, only logged: bic_error
/// (meaning not documented) and the four over-temperature warnings
/// (adc_shutdown is what follows them).
pub const DIG2_FAULT_MASK: u32 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 4)
    | (1 << 5)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 16);
/// DIG2 `ErrorFlags` bit 15 clkin_miss: no clock on CLK-IN.
pub const DIG2_CLKIN_MISS: u32 = 1 << 15;

/// "jesd_unlock, clkin_miss" for the bits set in `bits`; bits past the table
/// as "bit17".
pub fn flag_names(bits: u32) -> String {
    (0..32)
        .filter(|b| bits & (1 << b) != 0)
        .map(|b| match DIG2_FLAG_NAMES.get(b as usize) {
            Some(name) => (*name).to_string(),
            None => format!("bit{b}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One reading of a digitizer's clock / board status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PllReading {
    pub status: PllStatus,
    /// DIG2: the `ErrorFlags` bits that make it a fault (0 when none). DIG1: 0.
    pub fault_bits: u32,
    /// DIG2: the whole `ErrorFlags` word as read. `None` for DIG1, or when
    /// it could not be read.
    pub raw: Option<u32>,
}

impl PllReading {
    /// A status without any flags (DIG1, or nothing read).
    pub fn plain(status: PllStatus) -> Self {
        Self {
            status,
            fault_bits: 0,
            raw: None,
        }
    }
}

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

/// A DIG2 `/par/errorflags` read. `external_clock` (`None` = the clock
/// source cannot be read) is consulted only when bit 15 clkin_miss is set.
pub fn dig2_reading(
    reading: &Result<String, CaenError>,
    external_clock: impl FnOnce() -> Option<bool>,
) -> PllReading {
    let Some(raw) = reading.as_ref().ok().and_then(|v| parse_word(v)) else {
        return PllReading::plain(PllStatus::Unreadable);
    };
    let mut fault_bits = raw & DIG2_FAULT_MASK;
    let mut clock_unknown = false;
    if raw & DIG2_CLKIN_MISS != 0 {
        match external_clock() {
            Some(true) => fault_bits |= DIG2_CLKIN_MISS,
            Some(false) => {}
            None => clock_unknown = true,
        }
    }
    let status = if fault_bits != 0 {
        PllStatus::Unlocked
    } else if clock_unknown {
        PllStatus::Unreadable
    } else {
        PllStatus::Locked
    };
    PllReading {
        status,
        fault_bits,
        raw: Some(raw),
    }
}

/// Remembers the last status and fault bits so the read loop logs each
/// change once (jesd_unlock turning into si5341_unlock is a change).
#[derive(Debug, Default)]
pub struct PllWatch {
    last: Option<Option<(PllStatus, u32)>>,
}

impl PllWatch {
    /// Record a new reading; true when it differs from the previous one.
    pub fn changed(&mut self, reading: Option<PllReading>) -> bool {
        let key = reading.map(|r| (r.status, r.fault_bits));
        if self.last == Some(key) {
            return false;
        }
        self.last = Some(key);
        true
    }
}

/// The read loop's poll: every [`POLL_INTERVAL`], in every state (Idle,
/// Configured, Armed, Running), connected or not. Before 2026-11-10 it ran
/// only during a run, so a board on a dead clock chain showed nothing in the
/// UI until Start (pf-qaq, the DAQ restart of 2026-11-10).
#[derive(Debug)]
pub struct PllPoller {
    watch: PllWatch,
    /// False once a connected board showed it has no status parameter, so a
    /// disconnected board of that kind reports nothing rather than "unreadable".
    supported: bool,
    last: Option<std::time::Instant>,
}

impl Default for PllPoller {
    fn default() -> Self {
        Self {
            watch: PllWatch::default(),
            supported: true,
            last: None,
        }
    }
}

impl PllPoller {
    /// When a poll is due: read the status (`read` is called only when
    /// `connected`), log a change, and return the reading for the metrics.
    /// `None` when no poll was due.
    pub fn poll(
        &mut self,
        now: std::time::Instant,
        connected: bool,
        read: impl FnOnce() -> Option<PllReading>,
    ) -> Option<Option<PllReading>> {
        if self
            .last
            .is_some_and(|t| now.saturating_duration_since(t) < POLL_INTERVAL)
        {
            return None;
        }
        self.last = Some(now);
        let reading = if connected {
            let r = read();
            self.supported = r.is_some();
            r
        } else if self.supported {
            Some(PllReading::plain(PllStatus::Unreadable))
        } else {
            None
        };
        if self.watch.changed(reading) {
            log_change(reading, connected);
        }
        Some(reading)
    }
}

/// Read the status of a connected digitizer. `None` when its firmware has no
/// status parameter (not in the DevTree, or a firmware family this module
/// does not know). Without a parameter cache the read is attempted.
pub(crate) fn read(
    conn: &super::DeviceConnection,
    firmware: crate::config::digitizer::FirmwareType,
) -> Option<PllReading> {
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
        return Some(PllReading::plain(dig1_status(
            &conn.handle.get_value("/par/plllocked"),
        )));
    }
    if matches!(firmware, FirmwareType::PSD2 | FirmwareType::PHA2) {
        if !has(DIG2_PARAM) {
            return None;
        }
        let reading = conn.handle.get_value("/par/errorflags");
        return Some(dig2_reading(&reading, || {
            conn.handle
                .get_value("/par/clocksource")
                .ok()
                .map(|v| is_external_clock(&v))
        }));
    }
    None
}

/// The log line for a change of reading: (is a warning, text).
pub fn change_message(reading: Option<PllReading>, connected: bool) -> (bool, String) {
    let Some(r) = reading else {
        return (
            false,
            "No PLL status available for this firmware".to_string(),
        );
    };
    match (r.status, r.raw) {
        (PllStatus::Locked, None) => (false, "PLL locked (digitizer clock OK)".to_string()),
        (PllStatus::Locked, Some(0)) => (
            false,
            "Digitizer OK (errorflags 0: PLLs locked, no board error)".to_string(),
        ),
        (PllStatus::Locked, Some(raw)) => (
            false,
            format!(
                "Digitizer OK; errorflags {raw:#x} ({}) not counted as a fault",
                flag_names(raw)
            ),
        ),
        (PllStatus::Unlocked, None) => (
            true,
            "PLL NOT locked — the digitizer takes no valid data; check the clock \
             (CLK-IN cable of the daisy chain, clock source). The Operator stops a run \
             when this persists"
                .to_string(),
        ),
        (PllStatus::Unlocked, Some(raw)) => (
            true,
            format!(
                "Digitizer error: {} (errorflags {raw:#x}: {}) — the digitizer takes no \
                 valid data. The Operator stops a run when this persists; a DAQ restart \
                 usually brings the board back",
                flag_names(r.fault_bits),
                flag_names(raw)
            ),
        ),
        (PllStatus::Unreadable, Some(raw)) => (
            true,
            format!(
                "Digitizer status unclear: errorflags {raw:#x} ({}) has clkin_miss but the \
                 clock source cannot be read — the digitizer does not answer properly",
                flag_names(raw)
            ),
        ),
        (PllStatus::Unreadable, None) if connected => (
            true,
            "PLL status unreadable — the digitizer does not answer (lost clock or link?)"
                .to_string(),
        ),
        (PllStatus::Unreadable, None) => (
            true,
            "PLL status unknown — digitizer not connected".to_string(),
        ),
    }
}

/// Log a change. Every bad status is a warning (silent-failure rule): the
/// board takes no valid data.
pub fn log_change(reading: Option<PllReading>, connected: bool) {
    let (warning, text) = change_message(reading, connected);
    if warning {
        tracing::warn!("{text}");
    } else {
        tracing::info!("{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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

    fn locked() -> Option<PllReading> {
        Some(PllReading::plain(PllStatus::Locked))
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
    fn flag_names_follow_the_parameter_list() {
        assert_eq!(flag_names(0), "");
        assert_eq!(flag_names(1 << 5), "jesd_unlock");
        assert_eq!(
            flag_names((1 << 1) | (1 << 15)),
            "board_init_fault, clkin_miss"
        );
        assert_eq!(flag_names(1 << 16), "adc_shutdown");
        assert_eq!(flag_names(1 << 20), "bit20");
    }

    #[test]
    fn dig2_fault_bits_mean_no_valid_data_and_are_named() {
        let never = || panic!("clock source must not be read without clkin_miss");
        let r = dig2_reading(&ok("0"), never);
        assert_eq!(
            (r.status, r.fault_bits, r.raw),
            (PllStatus::Locked, 0, Some(0))
        );
        // The three PLL chips, the JESD link, and (user, 2026-11-10) power,
        // init, the DDR calibrations, the FPGA configuration, the ADC shutdown.
        for bit in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 16] {
            let r = dig2_reading(&ok(&(1u32 << bit).to_string()), || Some(false));
            assert_eq!(r.status, PllStatus::Unlocked, "bit {bit}");
            assert_eq!(r.fault_bits, 1 << bit, "bit {bit}");
        }
        // A master on its internal clock that failed its init (VX2730, 2026-11-10).
        let r = dig2_reading(&ok("0x8020"), || Some(false));
        assert_eq!(r.status, PllStatus::Unlocked);
        assert_eq!(flag_names(r.fault_bits), "jesd_unlock");
        assert_eq!(r.raw, Some(0x8020));
    }

    #[test]
    fn dig2_warnings_are_logged_but_not_faults() {
        // bic_error and the four over-temperature bits.
        for bit in [10, 11, 12, 13, 14] {
            let r = dig2_reading(&ok(&(1u32 << bit).to_string()), || Some(false));
            assert_eq!(r.status, PllStatus::Locked, "bit {bit}");
            assert_eq!(r.fault_bits, 0, "bit {bit}");
            assert_eq!(r.raw, Some(1 << bit));
        }
    }

    #[test]
    fn dig2_clkin_miss_counts_only_with_an_external_clock() {
        let word = DIG2_CLKIN_MISS.to_string();
        assert_eq!(
            dig2_reading(&ok(&word), || Some(false)).status,
            PllStatus::Locked
        );
        let r = dig2_reading(&ok(&word), || Some(true));
        assert_eq!(r.status, PllStatus::Unlocked);
        assert_eq!(flag_names(r.fault_bits), "clkin_miss");
        assert!(!is_external_clock("Internal"));
        assert!(!is_external_clock("internal"));
        assert!(is_external_clock("FPClkIn"));
    }

    #[test]
    fn dig2_clkin_miss_with_an_unreadable_clock_source_is_unreadable_not_unlocked() {
        let r = dig2_reading(&ok(&DIG2_CLKIN_MISS.to_string()), || None);
        assert_eq!(r.status, PllStatus::Unreadable);
        assert_eq!(r.fault_bits, 0);
        // A real fault bit still wins.
        let r = dig2_reading(&ok("0x8020"), || None);
        assert_eq!(r.status, PllStatus::Unlocked);
        assert_eq!(flag_names(r.fault_bits), "jesd_unlock");
    }

    #[test]
    fn dig2_error_or_garbage_is_unreadable() {
        assert_eq!(
            dig2_reading(&err(), || Some(false)),
            PllReading::plain(PllStatus::Unreadable)
        );
        assert_eq!(
            dig2_reading(&ok("n/a"), || Some(false)).status,
            PllStatus::Unreadable
        );
    }

    #[test]
    fn the_log_names_the_bits_and_the_whole_word() {
        let r = dig2_reading(&ok("0x8820"), || Some(false));
        let (warning, text) = change_message(Some(r), true);
        assert!(warning);
        assert!(
            text.starts_with("Digitizer error: jesd_unlock (errorflags 0x8820: "),
            "{text}"
        );
        assert!(
            text.contains("jesd_unlock, adc_overtemp, clkin_miss"),
            "{text}"
        );
        let (warning, text) = change_message(Some(PllReading::plain(PllStatus::Unlocked)), true);
        assert!(warning);
        assert!(text.starts_with("PLL NOT locked"), "{text}");
        let (warning, text) = change_message(dig2_reading(&ok("0x800"), || None).into(), true);
        assert!(!warning, "{text}");
        assert!(text.contains("adc_overtemp) not counted"), "{text}");
    }

    #[test]
    fn poller_reads_at_once_then_once_per_interval() {
        let mut p = PllPoller::default();
        let t0 = Instant::now();
        assert_eq!(p.poll(t0, true, locked), Some(locked()));
        assert_eq!(
            p.poll(t0 + POLL_INTERVAL / 2, true, || panic!(
                "read before the interval"
            )),
            None
        );
        let unlocked = Some(PllReading::plain(PllStatus::Unlocked));
        assert_eq!(
            p.poll(t0 + POLL_INTERVAL, true, || unlocked),
            Some(unlocked)
        );
    }

    #[test]
    fn poller_reports_a_disconnected_board_as_unreadable_without_reading() {
        let mut p = PllPoller::default();
        assert_eq!(
            p.poll(Instant::now(), false, || panic!(
                "nothing to read without a connection"
            )),
            Some(Some(PllReading::plain(PllStatus::Unreadable)))
        );
    }

    #[test]
    fn poller_firmware_without_a_status_stays_silent_when_disconnected() {
        let mut p = PllPoller::default();
        let t0 = Instant::now();
        assert_eq!(p.poll(t0, true, || None), Some(None));
        assert_eq!(
            p.poll(t0 + POLL_INTERVAL, false, || panic!("not connected")),
            Some(None)
        );
        // Reconnected to a board that has a status: a later loss shows again.
        assert_eq!(p.poll(t0 + 2 * POLL_INTERVAL, true, locked), Some(locked()));
        assert_eq!(
            p.poll(t0 + 3 * POLL_INTERVAL, false, || panic!("not connected")),
            Some(Some(PllReading::plain(PllStatus::Unreadable)))
        );
    }

    #[test]
    fn watch_reports_each_change_once_including_a_change_of_bits() {
        let mut w = PllWatch::default();
        assert!(w.changed(locked()));
        assert!(!w.changed(locked()));
        let jesd = dig2_reading(&ok("0x20"), || Some(false));
        let si = dig2_reading(&ok("0x4"), || Some(false));
        assert!(w.changed(Some(jesd)));
        assert!(!w.changed(Some(jesd)));
        assert!(w.changed(Some(si)), "another fault is a change");
        assert!(w.changed(None));
        assert!(!w.changed(None));
    }
}

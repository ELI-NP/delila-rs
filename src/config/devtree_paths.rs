//! FELib DevTree path string constants.
//!
//! The CAEN FELib exposes the digitizer as a JSON-RPC tree of nodes addressed
//! by string paths (`/cmd/armacquisition`, `/par/startmode`, etc.). Pre-Phase-1
//! these strings were hand-written at every call site, which made typos a
//! silent failure mode — case-insensitive `param_cache` (commit `e45e0ec`,
//! 2026-05-04) was the most recent example: a CamelCase typo fell through
//! cache lookup and the apply path silently dropped the write.
//!
//! Centralizing the strings here means:
//! 1. `cargo build` catches typos at compile time
//! 2. `grep` finds every consumer of a given path
//! 3. spec/page references can sit on the constant rather than be repeated
//!
//! # Coverage
//!
//! Phase 1 lifts only the **frequently-used** paths in the production read
//! loops (`reader::mod`) and the FELib handle (`reader::caen::handle`). Less
//! frequent paths in `config::digitizer` (per-channel parameter tables) and
//! dev binaries stay as literals for now — Phase 2 (R-C1) collapses
//! `add_channel_params()` into a table-driven design and will route those
//! through the same constants.
//!
//! # Source
//!
//! CAEN FELib User Guide GD9764, sections "Command set" and "Parameter set".

/// Top-level commands sent via `handle.send_command(path)`.
pub mod cmd {
    /// Arm the acquisition. Hardware moves Idle/Configured → Armed.
    /// DIG2 (FELib) accepts this directly; DIG1 with `START_MODE_SW` defers
    /// the actual arm to start (`armacquisition` is sent at start instead).
    pub const ARM_ACQUISITION: &str = "/cmd/armacquisition";

    /// Disarm the acquisition. Hardware moves Armed/Running → Configured.
    pub const DISARM_ACQUISITION: &str = "/cmd/disarmacquisition";

    /// Start acquisition by software command (after Arm).
    /// DIG1: when `startmode == START_MODE_SW`, the implementation actually
    /// sends `armacquisition` at this point (arm = start fused in DIG1 SW
    /// mode). See `send_start_command` in `reader::mod`.
    pub const SW_START_ACQUISITION: &str = "/cmd/swstartacquisition";

    /// Clear the FELib internal data buffers without changing acquisition
    /// state. Used post-Stop to drop residual Armed-window data so the next
    /// Start sees a clean buffer.
    pub const CLEAR_DATA: &str = "/cmd/cleardata";

    /// Soft reset the digitizer. **Important**: on DIG2 this also invalidates
    /// the active endpoint, so callers must re-`configure_endpoint` before
    /// any further `read_data`. See `read_loop_opendpp` for the recovery
    /// pattern (commit `aaa fix DIG1 S_IN time correlation`).
    pub const RESET: &str = "/cmd/reset";

    /// Run the X743 / DIG2 SAM-chip ADC calibration sweep. Takes ~700ms on
    /// VX2730; must be called before Arm in the Configure phase, otherwise
    /// the per-board startup latency desyncs S_OUT/S_IN time correlation.
    pub const CALIBRATE_ADC: &str = "/cmd/calibrateadc";
}

/// Board-level parameters (`handle.get_value(path)` / `set_value(path, v)`).
pub mod par {
    /// Active endpoint type — `"RAW"` for DIG1 (PSD1/PHA1) and PSD2/PHA2
    /// raw aggregate streaming, `"OpenDPP"` for AMax (pre-decoded events).
    /// Wire format depends on this field; switching mid-run is not safe.
    pub const ACTIVE_ENDPOINT: &str = "/par/activeendpoint";

    /// Hardware start mode. Common values: `"START_MODE_SW"` (software
    /// trigger initiates Run), `"START_MODE_S_IN"` (level-triggered by S_IN),
    /// `"START_MODE_FIRST_TRG"` (first physics trigger arms the run).
    pub const START_MODE: &str = "/par/startmode";

    /// Source of the start signal — orthogonal to `START_MODE` on DIG2.
    /// Examples: `"SWcmd"`, `"SIN"`, `"GPIO"`.
    pub const START_SOURCE: &str = "/par/startsource";

    /// Output to drive on TRG_OUT (Lemo). Examples: `"Run"`,
    /// `"GlobalTrigger"`, `"BusyOut"`.
    pub const TRG_OUT_SOURCE: &str = "/par/trgoutsource";

    /// What the SIN input maps to. Example: `"SIN"`, `"GlobalTrigger"`.
    pub const SIN_SOURCE: &str = "/par/sinsource";

    /// Source of the global trigger. Examples: `"ITLA"` (internal trigger
    /// logic), `"SWtrigger"`, `"External"`.
    pub const GLOBAL_TRIGGER_SOURCE: &str = "/par/globaltriggersource";

    /// GPIO mode override on PSD2/PHA2. Example: `"Run"`.
    pub const GPIO_MODE: &str = "/par/gpiomode";

    /// I/O level (NIM vs TTL). Example: `"NIM"`, `"TTL"`.
    pub const IO_LEVEL: &str = "/par/iolevel";

    /// Per-board waveform recording master switch. Set to `"true"` to enable.
    pub const WAVEFORMS: &str = "/par/waveforms";

    /// Per-board record length in samples (DIG1: ns / time_step).
    pub const REC_LEN: &str = "/par/reclen";

    /// Number of events per aggregate (DIG1 only). Higher value = more
    /// events per readout, lower interrupt rate.
    pub const EVENT_AGGR: &str = "/par/eventaggr";

    /// Per-board extras-word select (DIG1 only). Encodes which extras
    /// fields ride alongside the event word (timestamp_ext, fine_ts, etc.).
    pub const EXTRAS: &str = "/par/extras";

    /// Test pulse generator: high level (DAC counts).
    pub const TEST_PULSE_HIGH_LEVEL: &str = "/par/testpulsehighlevel";
    /// Test pulse generator: low level (DAC counts).
    pub const TEST_PULSE_LOW_LEVEL: &str = "/par/testpulselowlevel";
    /// Test pulse generator: period (ns).
    pub const TEST_PULSE_PERIOD: &str = "/par/testpulseperiod";
    /// Test pulse generator: width (ns).
    pub const TEST_PULSE_WIDTH: &str = "/par/testpulsewidth";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the constants must be the exact strings the FELib expects.
    /// A typo here would silently break every consumer; explicit `assert_eq!`
    /// against the literal pins the spec.
    #[test]
    fn cmd_paths_match_spec_strings() {
        assert_eq!(cmd::ARM_ACQUISITION, "/cmd/armacquisition");
        assert_eq!(cmd::DISARM_ACQUISITION, "/cmd/disarmacquisition");
        assert_eq!(cmd::SW_START_ACQUISITION, "/cmd/swstartacquisition");
        assert_eq!(cmd::CLEAR_DATA, "/cmd/cleardata");
        assert_eq!(cmd::RESET, "/cmd/reset");
        assert_eq!(cmd::CALIBRATE_ADC, "/cmd/calibrateadc");
    }

    #[test]
    fn par_paths_match_spec_strings() {
        assert_eq!(par::ACTIVE_ENDPOINT, "/par/activeendpoint");
        assert_eq!(par::START_MODE, "/par/startmode");
        assert_eq!(par::START_SOURCE, "/par/startsource");
        assert_eq!(par::TRG_OUT_SOURCE, "/par/trgoutsource");
        assert_eq!(par::REC_LEN, "/par/reclen");
        assert_eq!(par::WAVEFORMS, "/par/waveforms");
    }
}

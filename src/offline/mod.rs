//! Offline analysis toolkit — pure, I/O-free replay algorithms that run over
//! stored `.delila` waveforms (not on the reader hot path).
//!
//! Hosts the TODO 59 ELIADE energy-resolution auto-tune:
//! - [`trap`] — SW DPP-PHA trapezoid (recursion + closed-form per-event replay)
//! - [`peak`] — photopeak FWHM estimator
//! - [`scan`] — rise × flat-top grid scan over a set of recorded events
//! - [`fw_scan`] — the pure half of the FW-side scan (grid, config edits, ⁶⁰Co
//!   line finding and ranking)
//!
//! The CLI drivers live in `src/bin/pha_trap_tune.rs` and `src/bin/pha_fw_scan.rs`
//! (`dev-tools` feature).

pub mod fw_scan;
pub mod peak;
pub mod scan;
pub mod trap;

#[cfg(test)]
pub(crate) mod testutil;

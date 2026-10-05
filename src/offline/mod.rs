//! Offline analysis toolkit — pure, I/O-free replay algorithms that run over
//! stored `.delila` waveforms (not on the reader hot path).
//!
//! Hosts the TODO 59 ELIADE energy-resolution auto-tune:
//! - [`trap`] — SW DPP-PHA trapezoid (recursion + closed-form per-event replay)
//! - [`peak`] — photopeak FWHM estimator
//! - [`scan`] — rise × flat-top grid scan over a set of recorded events
//!
//! The CLI driver lives in `src/bin/pha_trap_tune.rs` (`dev-tools` feature).

pub mod peak;
pub mod scan;
pub mod trap;

#[cfg(test)]
pub(crate) mod testutil;

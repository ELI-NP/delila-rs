//! Offline analysis toolkit — pure, I/O-free replay algorithms that run over
//! stored `.delila` waveforms (not on the reader hot path).
//!
//! Currently hosts the SW DPP-PHA trapezoid ([`trap`]) for the TODO 59 ELIADE
//! energy-resolution auto-tune. The CLI driver lives in `src/bin/pha_trap_tune.rs`
//! (`dev-tools` feature).

pub mod trap;

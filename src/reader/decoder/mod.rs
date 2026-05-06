//! Decoder module for CAEN digitizer raw data
//!
//! Converts raw binary data from digitizers into structured EventData.
//!
//! # Hot-path heuristic policy (post-2026-05-04)
//!
//! Pattern-based "looks-like-something-special" checks during the
//! sample-decoding loop are **forbidden** unless both of the following
//! hold:
//!
//! 1. The bit pattern's exclusivity is provable from CAEN spec, with a
//!    page reference in the surrounding comment (CAEN doxygen / UM
//!    document name + section).
//! 2. The check is verified end-to-end with `caen_simple_test` against
//!    actual hardware — the binary captures raw aggregates with no
//!    decoder in the path, so a "the FW does X" claim is provable rather
//!    than inferred from full-pipeline symptoms.
//!
//! ## Why
//!
//! In Phase-3 stress debugging on 2026-05-04 we added a "mid-loop wf-header
//! truncation detector" to PHA2: while reading sample words, if a word
//! matched `bit63=1 ∧ bits[62:60]=0` we assumed the FW had short-written
//! the waveform and rewound to the next event. The heuristic looked safe
//! because the wf-header is documented to satisfy that pattern.
//!
//! It was not safe. PHA2 sample words pack two 32-bit half-samples per
//! 64-bit word: bit 63 carries `digital_probe_4` of the upper half, and
//! bits[62:60] cover `DP3 + AP2[13:12]`. With the default probe assignment
//! (or any probe that fires near the trigger such as `EnergyFilterPeaking`)
//! DP4 routinely toggles inside a real waveform; AP2 small near baseline
//! makes bits[62:60]=0 routinely true. The heuristic fired on legitimate
//! sample data and silently dropped the back half of every waveform —
//! 510 samples delivered out of 4096.
//!
//! The fix (commit `e641e99`) was found only after running
//! `caen_simple_test --firmware pha2 --wave-downsampling 1 --dump-words 0`,
//! which showed `wf_size = 2048` and event-to-event spacing of exactly
//! 2052 words. The FW never truncates a waveform mid-event.
//!
//! ## What this means for new decoder code
//!
//! - Trust the bytes the spec tells you to trust (`wf_size`,
//!   `wf_header.check1/check2`, `n_events`, …). Do not synthesise
//!   independent re-derivations from sample data.
//! - If a pipeline symptom genuinely looks like FW misbehaviour, the
//!   answer is `caen_simple_test` first, not a decoder heuristic.
//! - Keep `dp4_set_in_sample_does_not_truncate_waveform` (`pha2.rs`) as
//!   the regression for the specific bug above; add similar pinned tests
//!   for any future "the FW does X" claim that survives audit.

pub mod amax;
pub mod common;
pub mod pha1;
pub mod pha2;
pub mod psd1;
pub mod psd1_pha1_common;
pub mod psd2;
pub mod rollover;

pub use amax::{AMaxConfig, AMaxDecoder, AMaxEventData};
pub use common::{DataType, DecodeResult, EventData, RawData, Waveform};
pub use pha1::{Pha1Config, Pha1Decoder};
pub use pha2::{Pha2Config, Pha2Decoder};
pub use psd1::{Psd1Config, Psd1Decoder};
pub use psd2::{Psd2Config, Psd2Decoder};
pub use rollover::{RolloverError, RolloverTracker};

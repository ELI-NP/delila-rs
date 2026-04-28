//! CAENDigitizer Library FFI bindings for V1743 digitizers
//!
//! This module provides safe Rust wrappers around the legacy CAENDigitizer Library.
//! Unlike the modern FELib (used by PSD1/PSD2/PHA1/AMax), the CAENDigitizer Library
//! is required for x743 series digitizers which use SAMLONG switched capacitor arrays.
//!
//! # Feature Gate
//! All code in this module requires `feature = "x743"`.

mod error;
pub mod ffi;
mod handle;

pub use error::{DigitizerError, ErrorCode};
pub use handle::{
    AcqMode, BoardInfo, ConnectionType, EventBuffer, IOLevel, ReadoutBuffer, SamCorrectionLevel,
    SamFrequency, SamPulseSource, TriggerMode, TriggerPolarity, X743Handle, CHANNELS_PER_GROUP,
    MAX_CHANNELS, MAX_GROUPS,
};

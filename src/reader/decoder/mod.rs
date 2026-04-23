//! Decoder module for CAEN digitizer raw data
//!
//! Converts raw binary data from digitizers into structured EventData.

pub mod amax;
pub mod common;
pub mod pha1;
pub mod psd1;
pub mod psd2;
pub mod rollover;

pub use amax::{AMaxConfig, AMaxDecoder, AMaxEventData};
pub use common::{DataType, DecodeResult, EventData, RawData, Waveform};
pub use pha1::{Pha1Config, Pha1Decoder};
pub use psd1::{Psd1Config, Psd1Decoder};
pub use psd2::{Psd2Config, Psd2Decoder};
pub use rollover::{RolloverError, RolloverTracker};

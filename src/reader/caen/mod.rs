//! CAEN FELib wrapper module
//!
//! Safe Rust bindings for CAEN digitizer access via FELib.

pub mod amax_registers;
mod amax_registers_generated;
pub mod error;
pub mod ffi;
pub mod handle;
pub mod validation;

// Re-exports for convenience
pub use error::CaenError;
pub use handle::{CaenHandle, DeviceInfo, EndpointHandle, OpenDppEvent, ParamInfo, RawData};
pub use validation::{ApplyConfigResult, ParamApplyResult, ParamApplyStatus, ValidateResult};

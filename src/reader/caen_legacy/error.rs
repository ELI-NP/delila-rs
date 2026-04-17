//! CAENDigitizer error handling
//!
//! Wraps bindgen-generated CAEN_DGTZ_ErrorCode into Rust Result types.

use super::ffi;
use thiserror::Error;

/// Re-export the bindgen-generated error code enum
pub type ErrorCode = ffi::CAEN_DGTZ_ErrorCode;

/// CAENDigitizer error type
#[derive(Debug, Clone, Error)]
#[error("CAENDigitizer error: {code:?} in {context}")]
pub struct DigitizerError {
    pub code: ErrorCode,
    pub context: String,
}

impl DigitizerError {
    /// Create a custom error with a context message (uses GenericError code)
    pub fn new(_code: i32, context: &str) -> Self {
        Self {
            code: ErrorCode::CAEN_DGTZ_GenericError,
            context: context.to_string(),
        }
    }

    /// Check a CAENDigitizer return code, converting non-Success to Err
    pub fn check(ret: ErrorCode, context: &str) -> Result<(), Self> {
        if matches!(ret, ErrorCode::CAEN_DGTZ_Success) {
            Ok(())
        } else {
            Err(Self {
                code: ret,
                context: context.to_string(),
            })
        }
    }

    /// Check from a raw i32 code (for Drop where we get raw return)
    pub fn is_success(ret: ErrorCode) -> bool {
        matches!(ret, ErrorCode::CAEN_DGTZ_Success)
    }
}

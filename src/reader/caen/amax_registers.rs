//! AMax custom-firmware register map (CAEN VX2730 + DELILA AMax FW).
//!
//! The actual constants (`PAGE_BASE`, `PAGE_STRIDE`, `REG_*`) and the
//! `channel_register_byte_addr()` helper are auto-generated from the FW
//! developer's `RegisterFile.json` + `tools/amax_viewer/fw_params.json`.
//!
//! Run `cargo run --bin amax_codegen -- <RegisterFile.json>` to regenerate.

pub use super::amax_registers_generated::*;

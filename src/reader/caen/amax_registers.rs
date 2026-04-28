//! AMax custom-firmware register map (CAEN VX2730 + DELILA AMax FW).
//!
//! Each channel owns a register page at word address `PAGE_BASE + ch * PAGE_STRIDE`.
//! Byte address = `word_address * 4` (matches the FELib `SetUserRegister` ABI used
//! by `tools/amax_viewer/`).
//!
//! Source of truth:
//! - `tools/amax_viewer/fw_params.json` (bit widths + defaults)
//! - `AMAX_firmware32_channel_4input_caenlist/2channels_parameters_05032026.txt`
//!   (offsets, verified against the running FW on 2026-03)
//!
//! Only the writable registers are exported here. Read-only diagnostics
//! (`READ_ENERGY`, `debug_amax_out`, `maxim_outt`) are intentionally omitted —
//! they have nothing useful to write into.

/// Channel-page base in word units (channel 0).
pub const PAGE_BASE: u32 = 0x800000;

/// Word stride between consecutive channel pages.
/// Channel `n` page base = `PAGE_BASE + n * PAGE_STRIDE`.
pub const PAGE_STRIDE: u32 = 0x40000;

// ---- Per-channel register offsets (word, relative to the channel page base) ----

/// 1-bit waveform multiplexer (0 = trapezoidal, 1 = raw input)
pub const REG_SELECTOR_WAVE: u32 = 0x00;
/// 32-bit pre-trigger samples on raw input
pub const REG_PRETRIGGER_INPUT: u32 = 0x01;
/// 1-bit input pulse polarity (0 = neg, 1 = pos)
pub const REG_POLARITY: u32 = 0x02;
/// 16-bit DC offset
pub const REG_OFFSET: u32 = 0x03;
/// 32-bit trigger threshold
pub const REG_THRS: u32 = 0x04;
/// 16-bit fast-trigger rise (samples)
pub const REG_TRIG_K: u32 = 0x05;
/// 16-bit fast-trigger decay (samples)
pub const REG_TRIG_M: u32 = 0x06;
/// 16-bit trapezoidal rise (samples)
pub const REG_TRAP_K: u32 = 0x07;
/// 16-bit trapezoidal decay / flat top (samples)
pub const REG_TRAP_M: u32 = 0x08;
/// 24-bit deconvolution time constant
pub const REG_DECONV_M: u32 = 0x09;
/// 24-bit digital gain on the trapezoidal output
pub const REG_TRAP_GAIN: u32 = 0x0A;
/// 4-bit baseline averaging length (MCA HLS, log2 samples)
pub const REG_BL_LEN: u32 = 0x0B;
/// 16-bit baseline inhibit (samples)
pub const REG_BL_INIB: u32 = 0x0C;
/// 16-bit sample-pickoff position
pub const REG_SAMPLE_POS: u32 = 0x0D;
/// 1-bit run enable
pub const REG_RUN_CFG: u32 = 0x0F;
/// 32-bit AMax measurement window (samples)
pub const REG_AMAX_WINDOW: u32 = 0x12;
/// 32-bit AMax delay (samples)
pub const REG_AMAX_DELAY: u32 = 0x14;
/// 32-bit max-finder window (samples)
pub const REG_WINDOW_MAXIM: u32 = 0x15;
/// 16-bit AMax averaging length (samples)
pub const REG_AMAX_LEN: u32 = 0x16;
/// 32-bit baseline delay (samples)
pub const REG_BASELINE_DELAY: u32 = 0x18;
/// 16-bit baseline length (AMax HLS)
pub const REG_BASELINE_LEN: u32 = 0x19;
/// 16-bit baseline offset
pub const REG_BASELINE_OFFSET: u32 = 0x1A;
/// 32-bit pre-trigger samples on the trapezoidal output
pub const REG_PRETRIGGER_TRAP: u32 = 0x1F;
/// 32-bit pre-trigger samples on the AMax output
pub const REG_PRETRIGGER_AMAX: u32 = 0x20;

/// Compute the FELib byte address for a per-channel register.
/// Channel page base in words = `PAGE_BASE + channel * PAGE_STRIDE`;
/// byte address = `(page_base + word_offset) * 4`.
#[inline]
pub fn channel_register_byte_addr(channel: u8, word_offset: u32) -> u32 {
    let word_addr = PAGE_BASE + (channel as u32) * PAGE_STRIDE + word_offset;
    word_addr * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from `2channels_parameters_05032026.txt` (Decimal hex):
    /// ch0 POLARITY = 0x800002 (word) = 0x2000008 (byte)
    /// ch1 POLARITY = 0x840002 (word) = 0x2100008 (byte)
    /// ch1 PRETRIGGER_AMAX = 0x840020 (word) = 0x2100080 (byte)
    #[test]
    fn channel_register_byte_addr_matches_fw_table() {
        // Channel 0
        assert_eq!(channel_register_byte_addr(0, REG_SELECTOR_WAVE), 0x2000000);
        assert_eq!(channel_register_byte_addr(0, REG_POLARITY), 0x2000008);
        assert_eq!(channel_register_byte_addr(0, REG_THRS), 0x2000010);
        assert_eq!(
            channel_register_byte_addr(0, REG_PRETRIGGER_AMAX),
            0x2000080
        );

        // Channel 1: page stride = 0x40000 words = 0x100000 bytes
        assert_eq!(channel_register_byte_addr(1, REG_POLARITY), 0x2100008);
        assert_eq!(
            channel_register_byte_addr(1, REG_PRETRIGGER_AMAX),
            0x2100080
        );

        // Channel 31 (would-be in 32-ch FW): keeps linear extrapolation
        // 0x800000 + 31 * 0x40000 = 0xFC0000 → byte 0x3F00000
        assert_eq!(channel_register_byte_addr(31, REG_POLARITY), 0x3F00008);
    }

    /// Sanity: the writable offsets we export are pairwise distinct.
    #[test]
    fn register_offsets_are_distinct() {
        let offsets = [
            REG_SELECTOR_WAVE,
            REG_PRETRIGGER_INPUT,
            REG_POLARITY,
            REG_OFFSET,
            REG_THRS,
            REG_TRIG_K,
            REG_TRIG_M,
            REG_TRAP_K,
            REG_TRAP_M,
            REG_DECONV_M,
            REG_TRAP_GAIN,
            REG_BL_LEN,
            REG_BL_INIB,
            REG_SAMPLE_POS,
            REG_RUN_CFG,
            REG_AMAX_WINDOW,
            REG_AMAX_DELAY,
            REG_WINDOW_MAXIM,
            REG_AMAX_LEN,
            REG_BASELINE_DELAY,
            REG_BASELINE_LEN,
            REG_BASELINE_OFFSET,
            REG_PRETRIGGER_TRAP,
            REG_PRETRIGGER_AMAX,
        ];
        let mut sorted = offsets.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), offsets.len(), "duplicate register offset");
    }
}

//! PSD1 Decoder for DT5730 / VX1730 (DPP-PSD1) digitizers.
//!
//! All board-aggregate / dual-channel framing is shared with PHA1 in
//! [`super::psd1_pha1_common`] — this module only supplies the per-firmware
//! pieces (waveform layout, charge-word decode, SW fine-TS interpolation).
//!
//! # Data Format
//!
//! 32-bit Little-Endian words in a hierarchical structure:
//! Board Aggregate → Dual Channel Block → Events. See `psd1_pha1_common.rs`
//! for the framing details. PSD1-specific bits:
//! - `DUAL_CHANNEL_SIZE_MASK` = 22-bit (`0x3F_FFFF`)
//! - Per-event physics word: `charge_long` (high) + `charge_short` (low) + pileup
//! - SW Fine TS: 14-bit unsigned with 8192 baseline
//! - Waveform sample: 14-bit unsigned mask, DP1@14, DP2@15
//!
//! 47-bit timestamp: `(extended_time << 31) | trigger_time_tag` (HW Fine TS
//! mode), or board-aggregate-tracked extended time + sub-tick fraction
//! (SW Fine TS mode).

use super::common::{Waveform, UNKNOWN_PROBE_TYPE};
use super::psd1_pha1_common::{read_u32, Dig1Config, Dig1Decoder, Dig1Variant, WORD_SIZE};

// Re-export the unified channel header under a PSD-flavoured local alias
// purely so call-site naming reads naturally — no semantic difference.
pub use super::psd1_pha1_common::Dig1ChannelHeader as PsdChannelHeader;

mod waveform_bits {
    pub const ANALOG_SAMPLE_MASK: u32 = 0x3FFF;
    pub const DP1_SHIFT: u32 = 14;
    pub const DP2_SHIFT: u32 = 15;
    pub const SECOND_SAMPLE_SHIFT: u32 = 16;
    pub const SAMPLES_PER_GROUP: usize = 8;
}

mod physics_bits {
    pub const CHARGE_SHORT_MASK: u32 = 0x7FFF;
    pub const PILEUP_SHIFT: u32 = 15;
    pub const CHARGE_LONG_SHIFT: u32 = 16;
    pub const CHARGE_LONG_MASK: u32 = 0xFFFF;
}

// ---------------------------------------------------------------------------
// Public type aliases
// ---------------------------------------------------------------------------

/// Configuration for [`Psd1Decoder`]. Aliased to the shared [`Dig1Config`]
/// so PSD1 and PHA1 callers can build configs with identical syntax.
pub type Psd1Config = Dig1Config;

/// PSD1 decoder = monomorphized [`Dig1Decoder`] over [`Psd1Variant`].
pub type Psd1Decoder = Dig1Decoder<Psd1Variant>;

/// Per-firmware customization for DT5730 DPP-PSD1.
pub struct Psd1Variant;

impl Dig1Variant for Psd1Variant {
    const FW_NAME: &'static str = "PSD1";
    /// 22-bit dual-channel size mask (PHA1 uses 31-bit).
    const DUAL_CHANNEL_SIZE_MASK: u32 = 0x003F_FFFF;

    fn calculate_sw_fine_fraction(before_zc: u16, after_zc: u16) -> f64 {
        calculate_sw_fine_fraction_psd(before_zc, after_zc)
    }

    fn decode_physics_word(word: u32) -> (u16, u16, bool) {
        decode_charge_word(word)
    }

    fn decode_waveform(
        data: &[u8],
        offset: &mut usize,
        header: &PsdChannelHeader,
        ns_per_sample: f64,
    ) -> Waveform {
        decode_psd1_waveform(data, offset, header, ns_per_sample)
    }
}

// ---------------------------------------------------------------------------
// PSD1 free functions
// ---------------------------------------------------------------------------

/// SW Fine TS fraction from PSD1 zero-crossing samples.
/// PSD1 reports 14-bit unsigned ADC values centered on baseline 8192.
pub fn calculate_sw_fine_fraction_psd(before_zc: u16, after_zc: u16) -> f64 {
    const ADC_MIDPOINT: f64 = 8192.0;
    let before = before_zc as f64;
    let after = after_zc as f64;
    let denom = before - after;
    if denom.abs() > f64::EPSILON {
        ((ADC_MIDPOINT - after) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Decode the PSD1 per-event "physics" word: `(charge_long, charge_short, pileup)`.
///
/// Bit layout: `[31:16]=charge_long` (16-bit), `[15]=pileup`, `[14:0]=charge_short`.
///
/// Semantically equivalent to `pha1::decode_energy_word` modulo the second-u16
/// interpretation (PSD1: short-gate charge; PHA1: extra_data field). Both
/// flow through [`Dig1Variant::decode_physics_word`].
pub fn decode_charge_word(word: u32) -> (u16, u16, bool) {
    let charge_short = (word & physics_bits::CHARGE_SHORT_MASK) as u16;
    let pileup = ((word >> physics_bits::PILEUP_SHIFT) & 1) != 0;
    let charge_long = ((word >> physics_bits::CHARGE_LONG_SHIFT)
        & physics_bits::CHARGE_LONG_MASK) as u16;
    (charge_long, charge_short, pileup)
}

/// Decode the PSD1 waveform layout starting at `*offset`.
///
/// PSD1 packs two 14-bit unsigned samples per 32-bit word with `DP1` at bit 14
/// and `DP2` at bit 15 of each half.
///
/// Output principle: every probe vector has length `total_samples`.
/// - Single trace (DT=0): all samples → `analog_probe1`, no duplication.
/// - Dual trace (DT=1): even (s1) → probe2, odd (s2) → probe1, each
///   duplicated 2x (sample-and-hold) to match the digital probe length.
fn decode_psd1_waveform(
    data: &[u8],
    offset: &mut usize,
    header: &PsdChannelHeader,
    ns_per_sample: f64,
) -> Waveform {
    let total_words = header.num_samples_wave as usize * 4;
    let total_samples = header.num_samples_wave as usize * waveform_bits::SAMPLES_PER_GROUP;

    let mut analog_probe1 = Vec::with_capacity(total_samples);
    let mut analog_probe2 = Vec::with_capacity(if header.dual_trace { total_samples } else { 0 });
    let mut digital_probe1 = Vec::with_capacity(total_samples);
    let mut digital_probe2 = Vec::with_capacity(total_samples);

    for _ in 0..total_words {
        let w = read_u32(data, *offset);
        *offset += WORD_SIZE;

        // Lower half: sample 2N
        let s1_analog = (w & waveform_bits::ANALOG_SAMPLE_MASK) as i16;
        let s1_dp1 = ((w >> waveform_bits::DP1_SHIFT) & 1) as u8;
        let s1_dp2 = ((w >> waveform_bits::DP2_SHIFT) & 1) as u8;

        // Upper half: sample 2N+1
        let upper = w >> waveform_bits::SECOND_SAMPLE_SHIFT;
        let s2_analog = (upper & waveform_bits::ANALOG_SAMPLE_MASK) as i16;
        let s2_dp1 = ((upper >> waveform_bits::DP1_SHIFT) & 1) as u8;
        let s2_dp2 = ((upper >> waveform_bits::DP2_SHIFT) & 1) as u8;

        if header.dual_trace {
            // CAEN DPP-PSD dual trace: even(s1) = VTrace 1, odd(s2) = VTrace 0.
            // Each duplicated 2x (sample-and-hold) to match digital probe length.
            analog_probe1.push(s2_analog);
            analog_probe1.push(s2_analog);
            analog_probe2.push(s1_analog);
            analog_probe2.push(s1_analog);
        } else {
            analog_probe1.push(s1_analog);
            analog_probe1.push(s2_analog);
        }

        digital_probe1.push(s1_dp1);
        digital_probe1.push(s2_dp1);
        digital_probe2.push(s1_dp2);
        digital_probe2.push(s2_dp2);
    }

    Waveform {
        analog_probe1,
        analog_probe2,
        digital_probe1,
        digital_probe2,
        digital_probe3: vec![],
        digital_probe4: vec![],
        time_resolution: 0,
        trigger_threshold: 0,
        ns_per_sample,
        // PSD1 masks with `0x3FFF` so values land in `[0, 16383]` — unsigned.
        analog_probe1_is_signed: false,
        analog_probe2_is_signed: false,
        // PSD1 wf-extras header doesn't carry probe-type info; emit
        // UNKNOWN so the UI falls back to "A0/A1/D0..D3" generic labels.
        analog_probe_type: [UNKNOWN_PROBE_TYPE; 2],
        digital_probe_type: [UNKNOWN_PROBE_TYPE; 4],
    }
}

// ---------------------------------------------------------------------------
// Tests — PSD1-specific paths only. Framing tests live in psd1_pha1_common.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::decoder::common::{DataType, RawData};

    // -----------------------------------------------------------------------
    // calculate_sw_fine_fraction_psd
    // -----------------------------------------------------------------------

    #[test]
    fn sw_fine_fraction_returns_half_at_midpoint() {
        // before = 8192 + 1, after = 8192 - 1 → fraction = (8192 - 8191) / 2 = 0.5
        let frac = calculate_sw_fine_fraction_psd(8193, 8191);
        assert!((frac - 0.5).abs() < 1e-9);
    }

    #[test]
    fn sw_fine_fraction_clamps_to_unit_interval() {
        // Far above midpoint
        let frac = calculate_sw_fine_fraction_psd(16000, 8000);
        assert!((0.0..=1.0).contains(&frac));
    }

    #[test]
    fn sw_fine_fraction_returns_zero_on_no_crossing() {
        // before == after → division by zero short-circuit returns 0.0
        let frac = calculate_sw_fine_fraction_psd(8200, 8200);
        assert_eq!(frac, 0.0);
    }

    // -----------------------------------------------------------------------
    // decode_charge_word
    // -----------------------------------------------------------------------

    #[test]
    fn decode_charge_word_basic() {
        // charge_long = 1000 (high 16), charge_short = 500 (low 15), pileup = false
        let word = ((1000_u32) << 16) | 500;
        let (cl, cs, pu) = decode_charge_word(word);
        assert_eq!(cl, 1000);
        assert_eq!(cs, 500);
        assert!(!pu);
    }

    #[test]
    fn decode_charge_word_with_pileup() {
        let word = ((2000_u32) << 16) | (1 << 15) | 800;
        let (cl, cs, pu) = decode_charge_word(word);
        assert_eq!(cl, 2000);
        assert_eq!(cs, 800);
        assert!(pu);
    }

    #[test]
    fn decode_charge_word_max_values() {
        // charge_short = 15-bit max, charge_long = 16-bit max
        let word = ((0xFFFF_u32) << 16) | 0x7FFF;
        let (cl, cs, _) = decode_charge_word(word);
        assert_eq!(cl, 0xFFFF);
        assert_eq!(cs, 0x7FFF);
    }

    // -----------------------------------------------------------------------
    // decode_psd1_waveform — PSD1-specific bit layout
    // -----------------------------------------------------------------------

    fn make_header(num_samples_wave: u16, dual_trace: bool) -> PsdChannelHeader {
        PsdChannelHeader {
            block_size: 0,
            num_samples_wave,
            extra_option: 0,
            samples_enabled: true,
            extras_enabled: false,
            time_enabled: false,
            physics_enabled: false,
            dual_trace,
        }
    }

    fn push_u32(buf: &mut Vec<u8>, value: u32) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn waveform_single_trace_has_all_samples_in_probe1() {
        let h = make_header(1, false); // 8 raw samples, 4 wf words
        let mut buf = Vec::new();
        push_u32(&mut buf, 100 | (200 << 16));
        push_u32(&mut buf, 300 | (400 << 16));
        push_u32(&mut buf, 500 | (600 << 16));
        push_u32(&mut buf, 700 | (800 << 16));

        let mut offset = 0;
        let wf = decode_psd1_waveform(&buf, &mut offset, &h, 2.0);

        assert_eq!(offset, 16);
        assert_eq!(wf.analog_probe1.len(), 8);
        assert!(wf.analog_probe2.is_empty());
        assert_eq!(wf.analog_probe1, vec![100, 200, 300, 400, 500, 600, 700, 800]);
        assert!(!wf.analog_probe1_is_signed);
        assert_eq!(wf.ns_per_sample, 2.0);
    }

    #[test]
    fn waveform_dual_trace_swaps_and_duplicates() {
        let h = make_header(1, true);
        let mut buf = Vec::new();
        // lower(s1)=100→probe2, upper(s2)=200→probe1, each 2x duplicated
        push_u32(&mut buf, 100 | (200 << 16));
        push_u32(&mut buf, 300 | (400 << 16));
        push_u32(&mut buf, 500 | (600 << 16));
        push_u32(&mut buf, 700 | (800 << 16));

        let mut offset = 0;
        let wf = decode_psd1_waveform(&buf, &mut offset, &h, 2.0);

        assert_eq!(wf.analog_probe1, vec![200, 200, 400, 400, 600, 600, 800, 800]);
        assert_eq!(wf.analog_probe2, vec![100, 100, 300, 300, 500, 500, 700, 700]);
    }

    #[test]
    fn waveform_digital_probes_extracted_from_bits_14_15() {
        let h = make_header(1, false);
        let mut buf = Vec::new();
        // s1: analog=50, DP1=1 (bit14), DP2=0 (bit15)
        // s2: analog=60, DP1=0 (bit14+16=30), DP2=1 (bit15+16=31)
        let w: u32 = 50 | (1 << 14) | (60 << 16) | (1 << 31);
        push_u32(&mut buf, w);
        push_u32(&mut buf, 0);
        push_u32(&mut buf, 0);
        push_u32(&mut buf, 0);

        let mut offset = 0;
        let wf = decode_psd1_waveform(&buf, &mut offset, &h, 2.0);

        // First word's two samples: dp1=[1,0], dp2=[0,1]
        assert_eq!(wf.digital_probe1[0], 1);
        assert_eq!(wf.digital_probe1[1], 0);
        assert_eq!(wf.digital_probe2[0], 0);
        assert_eq!(wf.digital_probe2[1], 1);
    }

    // -----------------------------------------------------------------------
    // End-to-end smoke via the type alias — proves Dig1Decoder<Psd1Variant>
    // is wired correctly. Exhaustive framing tests live in psd1_pha1_common.
    // -----------------------------------------------------------------------

    fn make_board_header(aggregate_size: u32, mask: u8, board_id: u8, counter: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        push_u32(&mut buf, (0xA << 28) | (aggregate_size & 0x0FFF_FFFF));
        push_u32(&mut buf, ((board_id as u32) << 27) | (mask as u32));
        push_u32(&mut buf, counter & 0x7F_FFFF);
        push_u32(&mut buf, 0x1234_5678);
        buf
    }

    fn make_dual_channel_header(size: u32) -> Vec<u8> {
        // ET + EE (extras) + EQ (charge) enabled, extra_option=2
        let mut buf = Vec::new();
        push_u32(&mut buf, (1 << 31) | (size & 0x3F_FFFF));
        let w1: u32 = (2 << 24) | (1 << 28) | (1 << 29) | (1 << 30);
        push_u32(&mut buf, w1);
        buf
    }

    #[test]
    fn psd1_decoder_classifies_valid_board_header_as_event() {
        let dec = Psd1Decoder::with_defaults();
        let raw = RawData::new(make_board_header(4, 0x01, 0, 1));
        assert_eq!(dec.classify(&raw), DataType::Event);
    }

    #[test]
    fn psd1_decoder_decode_smoke_single_event() {
        let mut dec = Psd1Decoder::new(Psd1Config {
            time_step_ns: 2.0,
            module_id: 3,
            dump_enabled: false,
        });

        let event_words = 3; // time + extras + charge
        let ch_size = 2 + event_words;
        let total_size = 4 + ch_size;

        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32));
        push_u32(&mut data, 1000); // time word
        push_u32(&mut data, (123_u32 << 16) | (4 << 10) | 200); // extras: ext=123, flags=4, fine=200
        push_u32(&mut data, ((1500_u32) << 16) | 250); // charge: cl=1500, cs=250

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.module, 3);
        assert_eq!(e.channel, 0);
        assert_eq!(e.energy, 1500); // charge_long
        assert_eq!(e.energy_short, 250); // charge_short
        assert_eq!(e.fine_time, 200);
        assert_eq!(e.flags, 4);
    }

    #[test]
    fn psd1_decoder_dual_channel_size_mask_matches_22_bit() {
        // Sanity: PSD1's mask is 22-bit, distinct from PHA1's 31-bit.
        assert_eq!(Psd1Variant::DUAL_CHANNEL_SIZE_MASK, 0x003F_FFFF);
    }

    #[test]
    fn psd1_variant_fw_name_is_psd1() {
        assert_eq!(Psd1Variant::FW_NAME, "PSD1");
    }
}


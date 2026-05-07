//! PHA1 Decoder for DT5730 / VX1730 (DPP-PHA1) digitizers.
//!
//! All board-aggregate / dual-channel framing is shared with PSD1 in
//! [`super::psd1_pha1_common`] — this module only supplies the per-firmware
//! pieces (waveform layout with sign-extended samples, energy-word decode,
//! signed SW fine-TS interpolation).
//!
//! # Data Format
//!
//! 32-bit Little-Endian words in a hierarchical structure:
//! Board Aggregate → Dual Channel Block → Events. See `psd1_pha1_common.rs`
//! for the framing details. PHA1-specific bits:
//! - `DUAL_CHANNEL_SIZE_MASK` = 31-bit (`0x7FFF_FFFF`)
//! - Per-event physics word: `energy` (low 15) + `extra_data` (high 10) + pileup
//! - SW Fine TS: signed (RC-CR2 zero-centered) interpolation
//! - Waveform sample: 14-bit sign-extended ([-8192, +8191]),
//!   bit 14 = configurable digital probe (DP), bit 15 = trigger flag (Tn)
//! - Tn → `digital_probe1` (D0), DP → `digital_probe2` (D1) to match
//!   vtrace UI ordering.

use super::common::{sign_extend_14bit, Waveform, UNKNOWN_PROBE_TYPE};
use super::psd1_pha1_common::{read_u32, Dig1Config, Dig1Decoder, Dig1Variant, WORD_SIZE};

// Re-export the unified channel header under a PHA-flavoured local alias
// purely so call-site naming reads naturally — no semantic difference.
pub use super::psd1_pha1_common::Dig1ChannelHeader as PhaChannelHeader;

mod waveform_bits {
    pub const DP_SHIFT: u32 = 14;
    pub const TRIGGER_FLAG_SHIFT: u32 = 15;
    pub const SECOND_SAMPLE_SHIFT: u32 = 16;
    pub const SAMPLES_PER_GROUP: usize = 8;
}

mod physics_bits {
    pub const ENERGY_MASK: u32 = 0x7FFF;
    pub const PILEUP_SHIFT: u32 = 15;
    pub const EXTRA_DATA_SHIFT: u32 = 16;
    pub const EXTRA_DATA_MASK: u32 = 0x3FF;
}

// ---------------------------------------------------------------------------
// Public type aliases
// ---------------------------------------------------------------------------

/// Configuration for [`Pha1Decoder`]. Aliased to the shared [`Dig1Config`]
/// so PSD1 and PHA1 callers can build configs with identical syntax.
pub type Pha1Config = Dig1Config;

/// PHA1 decoder = monomorphized [`Dig1Decoder`] over [`Pha1Variant`].
pub type Pha1Decoder = Dig1Decoder<Pha1Variant>;

/// Per-firmware customization for DT5730 DPP-PHA1.
pub struct Pha1Variant;

impl Dig1Variant for Pha1Variant {
    const FW_NAME: &'static str = "PHA1";
    /// 31-bit dual-channel size mask (PSD1 uses 22-bit).
    const DUAL_CHANNEL_SIZE_MASK: u32 = 0x7FFF_FFFF;

    fn calculate_sw_fine_fraction(before_zc: u16, after_zc: u16) -> f64 {
        calculate_sw_fine_fraction_pha(before_zc, after_zc)
    }

    fn decode_physics_word(word: u32) -> (u16, u16, bool) {
        decode_energy_word(word)
    }

    fn decode_waveform(
        data: &[u8],
        offset: &mut usize,
        header: &PhaChannelHeader,
        ns_per_sample: f64,
    ) -> Waveform {
        decode_pha1_waveform(data, offset, header, ns_per_sample)
    }
}

// ---------------------------------------------------------------------------
// PHA1 free functions
// ---------------------------------------------------------------------------

/// SW Fine TS fraction from PHA1 zero-crossing samples.
/// PHA1 reports signed values (RC-CR2 filter output, zero-centered).
pub fn calculate_sw_fine_fraction_pha(before_zc: u16, after_zc: u16) -> f64 {
    let before = before_zc as i16 as f64;
    let after = after_zc as i16 as f64;
    let denom = before - after;
    if denom.abs() > f64::EPSILON {
        (before / denom).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Decode the PHA1 per-event "physics" word: `(energy, extra_data, pileup)`.
///
/// Bit layout: `[25:16]=extra_data` (10-bit), `[15]=pileup`,
/// `[14:0]=energy` (15-bit).
///
/// Semantically equivalent to `psd1::decode_charge_word` modulo the second-u16
/// interpretation (PSD1: short-gate charge; PHA1: extra_data field). Both
/// flow through [`Dig1Variant::decode_physics_word`].
pub fn decode_energy_word(word: u32) -> (u16, u16, bool) {
    let energy = (word & physics_bits::ENERGY_MASK) as u16;
    let pileup = ((word >> physics_bits::PILEUP_SHIFT) & 1) != 0;
    let extra_data =
        ((word >> physics_bits::EXTRA_DATA_SHIFT) & physics_bits::EXTRA_DATA_MASK) as u16;
    (energy, extra_data, pileup)
}

/// Decode the PHA1 waveform layout starting at `*offset`.
///
/// PHA1 packs two 14-bit signed samples per 32-bit word with `DP`
/// (configurable) at bit 14 and `Tn` (trigger flag, fixed) at bit 15
/// of each half. Tn maps to `digital_probe1` (D0) and DP to
/// `digital_probe2` (D1) to match the vtrace UI ordering.
///
/// Output principle: every probe vector has length `total_samples`.
/// - Single trace (DT=0): all samples → `analog_probe1`, no duplication.
/// - Dual trace (DT=1): even (s1) → probe2, odd (s2) → probe1, each
///   duplicated 2x (sample-and-hold) to match the digital probe length.
fn decode_pha1_waveform(
    data: &[u8],
    offset: &mut usize,
    header: &PhaChannelHeader,
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
        let s1_analog = sign_extend_14bit(w);
        let s1_dp = ((w >> waveform_bits::DP_SHIFT) & 1) as u8;
        let s1_tn = ((w >> waveform_bits::TRIGGER_FLAG_SHIFT) & 1) as u8;

        // Upper half: sample 2N+1
        let upper = w >> waveform_bits::SECOND_SAMPLE_SHIFT;
        let s2_analog = sign_extend_14bit(upper);
        let s2_dp = ((upper >> waveform_bits::DP_SHIFT) & 1) as u8;
        let s2_tn = ((upper >> waveform_bits::TRIGGER_FLAG_SHIFT) & 1) as u8;

        if header.dual_trace {
            // CAEN DPP-PHA dual trace: even(s1) = VTrace 1, odd(s2) = VTrace 0.
            // Each duplicated 2x (sample-and-hold) to match digital probe length.
            analog_probe1.push(s2_analog);
            analog_probe1.push(s2_analog);
            analog_probe2.push(s1_analog);
            analog_probe2.push(s1_analog);
        } else {
            analog_probe1.push(s1_analog);
            analog_probe1.push(s2_analog);
        }

        // Tn → D0 (fixed trigger), DP → D1 (configurable) — matches vtrace UI order.
        digital_probe1.push(s1_tn);
        digital_probe1.push(s2_tn);
        digital_probe2.push(s1_dp);
        digital_probe2.push(s2_dp);
    }

    Waveform {
        analog_probe1,
        analog_probe2,
        analog_probe3: vec![],
        digital_probe1,
        digital_probe2,
        digital_probe3: vec![],
        digital_probe4: vec![],
        digital_probe5: vec![],
        time_resolution: 0,
        trigger_threshold: 0,
        ns_per_sample,
        // PHA1 sign-extends both probes (`sign_extend_14bit`) so values can
        // land in [-8192, 8191] — trapezoid / Delta probes go negative
        // around baseline.
        analog_probe1_is_signed: true,
        analog_probe2_is_signed: true,
        analog_probe3_is_signed: false,
        // PHA1 (DIG1) doesn't carry typed probe info on the wire — probe
        // identity comes from the host-side `vtrace_probe` setting. Emit
        // UNKNOWN.
        analog_probe_type: [UNKNOWN_PROBE_TYPE; 3],
        digital_probe_type: [UNKNOWN_PROBE_TYPE; 5],
    }
}

// ---------------------------------------------------------------------------
// Tests — PHA1-specific paths only. Framing tests live in psd1_pha1_common.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::decoder::common::{DataType, RawData};

    // -----------------------------------------------------------------------
    // calculate_sw_fine_fraction_pha — signed interpolation
    // -----------------------------------------------------------------------

    #[test]
    fn sw_fine_fraction_signed_interpolates_at_zero_cross() {
        // before = +3, after = -2 → fraction = 3 / (3 - (-2)) = 3/5 = 0.6
        let before_zc: u16 = 3;
        let after_zc: u16 = (-2_i16) as u16;
        let frac = calculate_sw_fine_fraction_pha(before_zc, after_zc);
        assert!((frac - 0.6).abs() < 1e-9);
    }

    #[test]
    fn sw_fine_fraction_clamps_to_unit_interval() {
        let before_zc: u16 = 1000;
        let after_zc: u16 = (-100_i16) as u16;
        let frac = calculate_sw_fine_fraction_pha(before_zc, after_zc);
        assert!((0.0..=1.0).contains(&frac));
    }

    #[test]
    fn sw_fine_fraction_returns_zero_when_no_crossing() {
        // before == after (no slope) → division avoided, returns 0.0
        let frac = calculate_sw_fine_fraction_pha(10, 10);
        assert_eq!(frac, 0.0);
    }

    // -----------------------------------------------------------------------
    // decode_energy_word
    // -----------------------------------------------------------------------

    #[test]
    fn decode_energy_word_basic() {
        let energy: u16 = 1000;
        let extra: u16 = 500;
        let word = ((extra as u32) << 16) | (energy as u32);
        let (e, x, pu) = decode_energy_word(word);
        assert_eq!(e, energy);
        assert_eq!(x, extra);
        assert!(!pu);
    }

    #[test]
    fn decode_energy_word_with_pileup() {
        let energy: u16 = 2000;
        let extra: u16 = 800;
        let word = ((extra as u32) << 16) | (1 << 15) | (energy as u32);
        let (e, x, pu) = decode_energy_word(word);
        assert_eq!(e, energy);
        assert_eq!(x, extra);
        assert!(pu);
    }

    #[test]
    fn decode_energy_word_max_values() {
        // PHA1: energy = 15-bit (max 0x7FFF), extra_data = 10-bit (max 0x3FF)
        let word = ((0x3FF_u32) << 16) | 0x7FFF;
        let (e, x, _) = decode_energy_word(word);
        assert_eq!(e, 0x7FFF);
        assert_eq!(x, 0x3FF);
    }

    // -----------------------------------------------------------------------
    // decode_pha1_waveform — PHA1-specific bit layout
    // -----------------------------------------------------------------------

    fn make_header(num_samples_wave: u16, dual_trace: bool) -> PhaChannelHeader {
        PhaChannelHeader {
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
    fn waveform_single_trace_sign_extends_samples() {
        // Use small positives so 14-bit sign extension yields the same value.
        let h = make_header(1, false);
        let mut buf = Vec::new();
        push_u32(&mut buf, 100 | (200 << 16));
        push_u32(&mut buf, 300 | (400 << 16));
        push_u32(&mut buf, 500 | (600 << 16));
        push_u32(&mut buf, 700 | (800 << 16));

        let mut offset = 0;
        let wf = decode_pha1_waveform(&buf, &mut offset, &h, 2.0);

        assert_eq!(offset, 16);
        assert_eq!(wf.analog_probe1.len(), 8);
        assert!(wf.analog_probe2.is_empty());
        assert_eq!(
            wf.analog_probe1,
            vec![100, 200, 300, 400, 500, 600, 700, 800]
        );
        // Negative-sample sign extension regression
        assert!(wf.analog_probe1_is_signed);
        assert_eq!(wf.ns_per_sample, 2.0);
    }

    #[test]
    fn waveform_sign_extension_handles_negative_value() {
        // Bit 13 = 1 (= 0x2000) → sign-extends to -8192
        let h = make_header(1, false);
        let mut buf = Vec::new();
        push_u32(&mut buf, 0x2000 | (0x3FFF << 16)); // s1 = -8192, s2 = -1
        push_u32(&mut buf, 0);
        push_u32(&mut buf, 0);
        push_u32(&mut buf, 0);

        let mut offset = 0;
        let wf = decode_pha1_waveform(&buf, &mut offset, &h, 2.0);

        assert_eq!(wf.analog_probe1[0], -8192);
        assert_eq!(wf.analog_probe1[1], -1);
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
        let wf = decode_pha1_waveform(&buf, &mut offset, &h, 2.0);

        assert_eq!(
            wf.analog_probe1,
            vec![200, 200, 400, 400, 600, 600, 800, 800]
        );
        assert_eq!(
            wf.analog_probe2,
            vec![100, 100, 300, 300, 500, 500, 700, 700]
        );
    }

    #[test]
    fn waveform_digital_probes_tn_to_d0_dp_to_d1() {
        let h = make_header(1, false);
        let mut buf = Vec::new();
        // Word 0: s1 has DP=1 (bit 14), Tn=0 (bit 15); s2 has DP=0, Tn=1 (bit 31)
        let w: u32 = 50 | (1 << 14) | (60 << 16) | (1 << 31);
        push_u32(&mut buf, w);
        push_u32(&mut buf, 0);
        push_u32(&mut buf, 0);
        push_u32(&mut buf, 0);

        let mut offset = 0;
        let wf = decode_pha1_waveform(&buf, &mut offset, &h, 2.0);

        // First word: digital_probe1 (Tn) = [0, 1], digital_probe2 (DP) = [1, 0]
        assert_eq!(wf.digital_probe1[0], 0); // s1 trigger off
        assert_eq!(wf.digital_probe1[1], 1); // s2 trigger on
        assert_eq!(wf.digital_probe2[0], 1); // s1 DP on
        assert_eq!(wf.digital_probe2[1], 0); // s2 DP off
    }

    // -----------------------------------------------------------------------
    // End-to-end smoke via the type alias — proves Dig1Decoder<Pha1Variant>
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

    /// Build a 31-bit-size dual-channel header (PHA1 mask differs from PSD1).
    fn make_dual_channel_header(size: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        push_u32(&mut buf, (1 << 31) | (size & 0x7FFF_FFFF));
        // ET + E2 + EE (energy) enabled, extra_option=2
        let w1: u32 = (2 << 24) | (1 << 28) | (1 << 29) | (1 << 30);
        push_u32(&mut buf, w1);
        buf
    }

    #[test]
    fn pha1_decoder_classifies_valid_board_header_as_event() {
        let dec = Pha1Decoder::with_defaults();
        let raw = RawData::new(make_board_header(4, 0x01, 0, 1));
        assert_eq!(dec.classify(&raw), DataType::Event);
    }

    #[test]
    fn pha1_decoder_decode_smoke_single_event() {
        let mut dec = Pha1Decoder::new(Pha1Config {
            time_step_ns: 2.0,
            module_id: 5,
            dump_enabled: false,
        });

        let event_words = 3; // time + extras2 + energy
        let ch_size = 2 + event_words;
        let total_size = 4 + ch_size;

        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32));
        push_u32(&mut data, 1000); // time word
        push_u32(&mut data, (50_u32 << 16) | 256); // extras: ext=50, fine=256
                                                   // energy: extra_data=300 (high 10), pileup=false, energy=4500 (low 15)
        let ew = ((300_u32) << 16) | 4500;
        push_u32(&mut data, ew);

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.module, 5);
        assert_eq!(e.channel, 0);
        assert_eq!(e.energy, 4500);
        assert_eq!(e.energy_short, 300); // PHA1: extra_data stored in energy_short slot
        assert_eq!(e.fine_time, 256);
    }

    #[test]
    fn pha1_decoder_dual_channel_size_mask_matches_31_bit() {
        // Sanity: PHA1's mask is 31-bit, distinct from PSD1's 22-bit.
        assert_eq!(Pha1Variant::DUAL_CHANNEL_SIZE_MASK, 0x7FFF_FFFF);
    }

    #[test]
    fn pha1_variant_fw_name_is_pha1() {
        assert_eq!(Pha1Variant::FW_NAME, "PHA1");
    }

    #[test]
    fn pha1_decoder_pileup_propagates_through_variant() {
        // Bit-15 pileup in the energy word should set the EventData flag.
        let mut dec = Pha1Decoder::with_defaults();
        let event_words = 3;
        let ch_size = 2 + event_words;
        let total_size = 4 + ch_size;

        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32));
        push_u32(&mut data, 1000);
        push_u32(&mut data, 0);
        push_u32(&mut data, ((100_u32) << 16) | (1 << 15) | 50);

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        // Pileup bit lands at bit 15 of the EventData flags field.
        assert_ne!(events[0].flags & (1 << 15), 0);
    }
}

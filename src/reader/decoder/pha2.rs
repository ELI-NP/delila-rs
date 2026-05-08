//! PHA2 Decoder for CAEN x274x series digitizers (DPP-PHA, trapezoidal-filter MCA).
//!
//! Decodes the DIG2 RAW endpoint Individual-Trigger-Mode aggregate format
//! (`format=0x2`), the same envelope as PSD2. All framing is shared in
//! [`super::dualchannel_common`] — this module only supplies the per-firmware
//! pieces:
//!
//! - **`energy_short` slot is unused** (PSD2 carries `charge_short` here;
//!   PHA2 leaves it 0 on the wire so we force it to 0 in the event).
//! - **Waveform header low bits encode probe metadata** per CAEN doxygen
//!   `legacy/PHA2_Parameters/a00108.html` (DPP-PHA waveform extras):
//!   - bits[5:0]   = analog probe #0 info: `type[2:0] | is_signed[3] | factor[5:4]`
//!   - bits[11:6]  = analog probe #1 info, same layout shifted
//!   - bits[15+4N : 12+4N] = digital probe #N type (4 bits)
//!
//! The is_signed bit drives sample sign-extension in the shared waveform
//! decoder — Time-filter / Energy-filter / Energy-filter-minus-baseline
//! probes arrive as 14-bit signed and would wrap to the top of the visible
//! band if interpreted unsigned (the original "weird Time-filter plot" bug,
//! commit `7ed3285`).
//!
//! 64-bit big-endian on the wire (VX274x hardware byte order).

use super::dualchannel_common::{Dig2Config, Dig2Decoder, Dig2Variant, WaveformMetadata};

/// PHA2 waveform-header low-bit layout. Per-probe info encoding from
/// CAEN doxygen `legacy/PHA2_Parameters/a00108.html`.
mod pha2_waveform_bits {
    /// a.p. #0 info at bits[5:0].
    pub const ANALOG_PROBE0_INFO_SHIFT: u32 = 0;
    /// a.p. #1 info at bits[11:6].
    pub const ANALOG_PROBE1_INFO_SHIFT: u32 = 6;
    pub const ANALOG_PROBE_INFO_MASK: u64 = 0x3F; // 6-bit slot
    /// is_signed lives at bit 3 inside the 6-bit slot.
    pub const ANALOG_PROBE_IS_SIGNED_BIT: u64 = 0x08;
    /// type[2:0] within the 6-bit slot.
    pub const ANALOG_PROBE_TYPE_MASK: u64 = 0x07;

    /// d.p. #N at bits[15+4N : 12+4N] — 4-bit type field per probe.
    pub const DIGITAL_PROBE0_INFO_SHIFT: u32 = 12;
    pub const DIGITAL_PROBE1_INFO_SHIFT: u32 = 16;
    pub const DIGITAL_PROBE2_INFO_SHIFT: u32 = 20;
    pub const DIGITAL_PROBE3_INFO_SHIFT: u32 = 24;
    pub const DIGITAL_PROBE_INFO_MASK: u64 = 0x0F;
}

// ---------------------------------------------------------------------------
// Public type aliases
// ---------------------------------------------------------------------------

/// Configuration for [`Pha2Decoder`]. Aliased to the shared [`Dig2Config`]
/// so PSD2 and PHA2 callers can build configs with identical syntax.
pub type Pha2Config = Dig2Config;

/// PHA2 decoder = monomorphized [`Dig2Decoder`] over [`Pha2Variant`].
pub type Pha2Decoder = Dig2Decoder<Pha2Variant>;

/// Per-firmware customization for VX274x DPP-PHA.
pub struct Pha2Variant;

impl Dig2Variant for Pha2Variant {
    const FW_NAME: &'static str = "PHA2";

    /// PHA2 leaves PSD2's `energy_short` slot (bits[41:26]) unused, so we
    /// always emit 0. Downstream consumers can rely on the FW-distinct
    /// "0 means PHA2" semantics for offline analysis.
    fn decode_energy_short(_second_word: u64) -> u16 {
        0
    }

    /// Parse the analog/digital probe metadata packed into the wf-header
    /// low 16 bits. The is_signed bit decides whether the sample loop
    /// sign-extends 14-bit values; the type bytes flow through to the
    /// frontend so it can render labels like "A0: TimeFilter" instead of
    /// the generic "A0".
    fn parse_waveform_metadata(wf_header: u64) -> WaveformMetadata {
        let ap0_info = (wf_header >> pha2_waveform_bits::ANALOG_PROBE0_INFO_SHIFT)
            & pha2_waveform_bits::ANALOG_PROBE_INFO_MASK;
        let ap1_info = (wf_header >> pha2_waveform_bits::ANALOG_PROBE1_INFO_SHIFT)
            & pha2_waveform_bits::ANALOG_PROBE_INFO_MASK;
        let analog_probe1_is_signed =
            (ap0_info & pha2_waveform_bits::ANALOG_PROBE_IS_SIGNED_BIT) != 0;
        let analog_probe2_is_signed =
            (ap1_info & pha2_waveform_bits::ANALOG_PROBE_IS_SIGNED_BIT) != 0;
        let analog_probe_type = [
            (ap0_info & pha2_waveform_bits::ANALOG_PROBE_TYPE_MASK) as u8,
            (ap1_info & pha2_waveform_bits::ANALOG_PROBE_TYPE_MASK) as u8,
        ];
        let digital_probe_type = [
            ((wf_header >> pha2_waveform_bits::DIGITAL_PROBE0_INFO_SHIFT)
                & pha2_waveform_bits::DIGITAL_PROBE_INFO_MASK) as u8,
            ((wf_header >> pha2_waveform_bits::DIGITAL_PROBE1_INFO_SHIFT)
                & pha2_waveform_bits::DIGITAL_PROBE_INFO_MASK) as u8,
            ((wf_header >> pha2_waveform_bits::DIGITAL_PROBE2_INFO_SHIFT)
                & pha2_waveform_bits::DIGITAL_PROBE_INFO_MASK) as u8,
            ((wf_header >> pha2_waveform_bits::DIGITAL_PROBE3_INFO_SHIFT)
                & pha2_waveform_bits::DIGITAL_PROBE_INFO_MASK) as u8,
        ];
        WaveformMetadata {
            analog_probe1_is_signed,
            analog_probe2_is_signed,
            analog_probe_type,
            digital_probe_type,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — PHA2-specific paths only. Framing tests live in dualchannel_common.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::decoder::common::{DataType, RawData};
    use crate::reader::decoder::dualchannel_common::event_word;

    fn pack_be(words: &[u64]) -> Vec<u8> {
        let mut out = Vec::with_capacity(words.len() * 8);
        for &w in words {
            out.extend_from_slice(&w.to_be_bytes());
        }
        out
    }

    fn raw_data(bytes: Vec<u8>, n_events: u32) -> RawData {
        RawData {
            size: bytes.len(),
            data: bytes,
            n_events,
        }
    }

    /// Build a minimal 1-event aggregate (header + 2-word event, no waveform).
    fn build_aggregate(channel: u8, ts: u64, energy: u16, fine_ts: u16) -> Vec<u8> {
        let total_words: u64 = 3;
        let header = (0x2u64 << 60) | total_words;
        let evt_w1 = ((channel as u64) << 56) | (ts & 0xFFFF_FFFF_FFFF);
        let evt_w2 = (1u64 << 63) | ((fine_ts as u64 & 0x3FF) << 16) | energy as u64;
        pack_be(&[header, evt_w1, evt_w2])
    }

    // -----------------------------------------------------------------------
    // decode_energy_short — the one PHA2-specific event path
    // -----------------------------------------------------------------------

    #[test]
    fn decode_energy_short_always_zero_regardless_of_word() {
        // PHA2 leaves PSD2's bits[41:26] slot unused; we always force 0.
        for word in [0u64, !0u64, 0xCAFE_BABE_DEAD_BEEF, 1u64 << 30] {
            assert_eq!(Pha2Variant::decode_energy_short(word), 0);
        }
    }

    // -----------------------------------------------------------------------
    // parse_waveform_metadata — PHA2's whole reason to exist as a variant
    // -----------------------------------------------------------------------

    #[test]
    fn parse_metadata_unsigned_unsigned_when_low_bits_clear() {
        let metadata = Pha2Variant::parse_waveform_metadata(0u64);
        assert!(!metadata.analog_probe1_is_signed);
        assert!(!metadata.analog_probe2_is_signed);
        assert_eq!(metadata.analog_probe_type, [0, 0]);
        assert_eq!(metadata.digital_probe_type, [0, 0, 0, 0]);
    }

    #[test]
    fn parse_metadata_picks_up_analog_probe_signed_flags() {
        // a.p.#0: factor=×1, is_signed=1, type=1 (TimeFilter)
        // a.p.#1: factor=×1, is_signed=0, type=0 (ADCInput)
        #[allow(clippy::unusual_byte_groupings)]
        let ap0_info: u64 = 0b00_1_001;
        #[allow(clippy::unusual_byte_groupings)]
        let ap1_info: u64 = 0b00_0_000;
        let wf_hdr: u64 = (1u64 << 63) | (ap1_info << 6) | ap0_info;
        let m = Pha2Variant::parse_waveform_metadata(wf_hdr);
        assert!(m.analog_probe1_is_signed);
        assert!(!m.analog_probe2_is_signed);
        assert_eq!(m.analog_probe_type, [1, 0]);
    }

    #[test]
    fn parse_metadata_picks_up_digital_probe_types() {
        // d.p. #0..3 = Trigger / TimeFilterArmed / ReTriggerGuard /
        // EnergyFilterBaselineFreeze (canonical default assignment).
        let dp0: u64 = 0;
        let dp1: u64 = 1;
        let dp2: u64 = 2;
        let dp3: u64 = 3;
        let wf_hdr: u64 = (1u64 << 63)
            | (dp3 << pha2_waveform_bits::DIGITAL_PROBE3_INFO_SHIFT)
            | (dp2 << pha2_waveform_bits::DIGITAL_PROBE2_INFO_SHIFT)
            | (dp1 << pha2_waveform_bits::DIGITAL_PROBE1_INFO_SHIFT)
            | (dp0 << pha2_waveform_bits::DIGITAL_PROBE0_INFO_SHIFT);
        let m = Pha2Variant::parse_waveform_metadata(wf_hdr);
        assert_eq!(m.digital_probe_type, [0, 1, 2, 3]);
    }

    // -----------------------------------------------------------------------
    // End-to-end via Pha2Decoder type alias — PHA2 wf-header parsing must
    // flow into the emitted Waveform (regression for commit 7ed3285 which
    // had hardcoded analog_probe_X_is_signed=false for PHA2).
    // -----------------------------------------------------------------------

    #[test]
    fn analog_probe_is_signed_flag_is_parsed_from_wf_header() {
        // wf_header: a.p.#0 = TimeFilter signed, a.p.#1 = ADCInput unsigned.
        // → analog_probe1_is_signed must be true, analog_probe2_is_signed must be false.
        // → with samples that are 0x3fff (= -1 if signed, +16383 if unsigned),
        //   the decoded buffer should hold -1 in analog_probe1 and +16383 in analog_probe2.
        #[allow(clippy::unusual_byte_groupings)]
        let ap0_info: u64 = 0b00_1_001;
        #[allow(clippy::unusual_byte_groupings)]
        let ap1_info: u64 = 0b00_0_000;
        let wf_hdr: u64 = (1u64 << 63) | (ap1_info << 6) | ap0_info;
        let wf_size: u64 = 1; // 1 word = 2 samples
        let sample: u64 = 0x3fff_3fff_3fff_3fff;

        let total_words: u64 = 6;
        let agg_header = (0x2u64 << 60) | total_words;
        let ev1_w1 = 0u64;
        let ev1_w2 = 1u64 << event_word::WAVEFORM_FLAG_SHIFT;

        let bytes = pack_be(&[agg_header, ev1_w1, ev1_w2, wf_hdr, wf_size, sample]);
        let mut dec = Pha2Decoder::with_defaults();
        let raw = raw_data(bytes, 1);
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);

        assert_eq!(events.len(), 1);
        let wf = events[0].waveform.as_ref().expect("waveform missing");
        assert!(
            wf.analog_probe1_is_signed,
            "TimeFilter probe must be flagged signed"
        );
        assert!(
            !wf.analog_probe2_is_signed,
            "ADCInput probe must be flagged unsigned"
        );
        assert_eq!(
            wf.analog_probe1[0], -1,
            "signed AP0 sign-extends 0x3fff to -1"
        );
        assert_eq!(
            wf.analog_probe2[0], 16383,
            "unsigned AP1 reads 0x3fff as 16383"
        );
        assert_eq!(
            wf.analog_probe_type,
            [1, 0, super::super::common::UNKNOWN_PROBE_TYPE],
            "AP0 type=1 (TimeFilter), AP1 type=0 (ADCInput), AP2=UNKNOWN (PHA2 ships ≤ 2 analog probes)"
        );
    }

    #[test]
    fn digital_probe_types_are_parsed_from_wf_header() {
        let dp0: u64 = 0;
        let dp1: u64 = 1;
        let dp2: u64 = 2;
        let dp3: u64 = 3;
        let wf_hdr: u64 = (1u64 << 63)
            | (dp3 << pha2_waveform_bits::DIGITAL_PROBE3_INFO_SHIFT)
            | (dp2 << pha2_waveform_bits::DIGITAL_PROBE2_INFO_SHIFT)
            | (dp1 << pha2_waveform_bits::DIGITAL_PROBE1_INFO_SHIFT)
            | (dp0 << pha2_waveform_bits::DIGITAL_PROBE0_INFO_SHIFT);
        let wf_size: u64 = 1;
        let sample: u64 = 0;

        let total_words: u64 = 6;
        let agg_header = (0x2u64 << 60) | total_words;
        let ev1_w1 = 0u64;
        let ev1_w2 = 1u64 << event_word::WAVEFORM_FLAG_SHIFT;

        let bytes = pack_be(&[agg_header, ev1_w1, ev1_w2, wf_hdr, wf_size, sample]);
        let mut dec = Pha2Decoder::with_defaults();
        let raw = raw_data(bytes, 1);
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);

        assert_eq!(events.len(), 1);
        let wf = events[0].waveform.as_ref().expect("waveform missing");
        // Slots 4..15 are reserved for future digital-lane bit assignments;
        // PHA2 only ever populates 0..3 from the wf-extras header.
        let mut expected = [super::super::common::UNKNOWN_PROBE_TYPE; 16];
        expected[0] = 0;
        expected[1] = 1;
        expected[2] = 2;
        expected[3] = 3;
        assert_eq!(wf.digital_probe_type, expected);
    }

    // -----------------------------------------------------------------------
    // Pinned regression: 2026-05-04 truncation bug
    // -----------------------------------------------------------------------

    #[test]
    fn dp4_set_in_sample_does_not_truncate_waveform() {
        // Regression: in 2026-05-04 we briefly added a "mid-loop truncation
        // detector" that flagged any sample word with bit63=1 ∧ bits[62:60]=0
        // as the next event's wf_header. That misfired catastrophically
        // because PHA2 sample words have bit63 = digital_probe_4 of the upper
        // 32-bit half — every event with EnergyFilterPeaking (a default DP)
        // has DP4 transiently set, and AP2 is small near baseline so the
        // bits[62:60]=0 condition is also met. Live capture via
        // `pha2_simple_test --wave-downsampling 8` showed wf_size = 0x800
        // and event-to-event spacing of exactly 2052 words on the wire —
        // i.e. NO firmware truncation. The decoder must trust wf_size and
        // deliver the full sample buffer even when sample bytes mimic the
        // wf_header bit pattern.
        let total_words: u64 = 9;
        let agg_header = (0x2u64 << 60) | total_words;
        let ev1_w1: u64 = 10;
        let ev1_w2 = (1u64 << event_word::WAVEFORM_FLAG_SHIFT) | (1u64 << 63);
        let wf_hdr_const: u64 = 1u64 << 63;
        let wf_size_4: u64 = 4;
        let baseline_sample: u64 = 0x0000_135C_0000_135C;
        let dp4_fluke_sample: u64 = 0x80f7_1fd6_00f7_1fdb; // bit63=1, bits[62:60]=0

        let words: Vec<u64> = vec![
            agg_header,
            ev1_w1,
            ev1_w2,
            wf_hdr_const,
            wf_size_4,
            baseline_sample,
            dp4_fluke_sample,
            baseline_sample,
            baseline_sample,
        ];

        let bytes = pack_be(&words);
        let mut dec = Pha2Decoder::with_defaults();
        let raw = raw_data(bytes, 1);
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);

        assert_eq!(
            events.len(),
            1,
            "decoder must NOT split on DP4-fluke samples"
        );
        let wf = events[0]
            .waveform
            .as_ref()
            .expect("waveform must be present");
        assert_eq!(
            wf.analog_probe1.len(),
            8,
            "all 4 sample words (8 samples) must be delivered"
        );
        // Iteration order: low half first, then high half. The fluke is
        // 0x80f71fd6_00f71fdb, so high-half upper sample is 0x80f71fd6 → DP4=1.
        // That's index 1*2 + 1 = 3 (event 1's word index 1, upper half).
        assert_eq!(
            wf.digital_probe4[3], 1,
            "DP4 fluke bit must be preserved in the sample buffer"
        );
    }

    // -----------------------------------------------------------------------
    // Captured-from-hardware bytes regression
    // -----------------------------------------------------------------------

    #[test]
    fn classify_start_signal_real_bytes() {
        // Captured from 172.18.4.56 PHA2 Start signal. Pinned so a future
        // change to the signal classifier doesn't silently drop this.
        let bytes: Vec<u8> = vec![
            0x30, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let dec = Pha2Decoder::with_defaults();
        let raw = raw_data(bytes, 1);
        assert_eq!(dec.classify(&raw), DataType::Start);
    }

    // -----------------------------------------------------------------------
    // Smoke tests via the type alias
    // -----------------------------------------------------------------------

    #[test]
    fn pha2_decoder_decode_smoke_zeroes_energy_short() {
        let mut dec = Pha2Decoder::new(Pha2Config {
            time_step_ns: 2.0,
            module_id: 9,
            dump_enabled: false,
            num_channels: 32,
        });
        let bytes = build_aggregate(
            /*ch*/ 5, /*ts*/ 1_000_000, /*e*/ 4242, /*fts*/ 200,
        );
        let raw = raw_data(bytes, 1);
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.module, 9);
        assert_eq!(e.channel, 5);
        assert_eq!(e.energy, 4242);
        assert_eq!(e.energy_short, 0); // PHA2 always 0 — the variant invariant
        assert_eq!(e.fine_time, 200);
    }

    #[test]
    fn pha2_variant_fw_name_is_pha2() {
        assert_eq!(Pha2Variant::FW_NAME, "PHA2");
    }
}

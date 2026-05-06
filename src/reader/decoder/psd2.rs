//! PSD2 Decoder for CAEN x27xx series digitizers (DPP-PSD).
//!
//! Decodes the DIG2 RAW endpoint Individual-Trigger-Mode aggregate format
//! (`format=0x2`). All framing is shared with PHA2 in
//! [`super::dualchannel_common`] — this module only supplies the per-firmware
//! pieces (energy_short slot at bits[41:26] of the event 2nd word, all-
//! unsigned waveform samples with no probe-type metadata).
//!
//! 64-bit big-endian on the wire (VX27xx hardware byte order).

use super::dualchannel_common::{Dig2Config, Dig2Decoder, Dig2Variant, WaveformMetadata};

/// Bit positions inside the per-event 2nd word that PSD2 owns. PHA2 leaves
/// bits[41:26] unused and forces energy_short to 0.
mod psd2_event_bits {
    pub const ENERGY_SHORT_SHIFT: u32 = 26;
    pub const ENERGY_SHORT_MASK: u64 = 0xFFFF;
}

// ---------------------------------------------------------------------------
// Public type aliases
// ---------------------------------------------------------------------------

/// Configuration for [`Psd2Decoder`]. Aliased to the shared [`Dig2Config`]
/// so PSD2 and PHA2 callers can build configs with identical syntax.
pub type Psd2Config = Dig2Config;

/// PSD2 decoder = monomorphized [`Dig2Decoder`] over [`Psd2Variant`].
pub type Psd2Decoder = Dig2Decoder<Psd2Variant>;

/// Per-firmware customization for VX27xx DPP-PSD.
pub struct Psd2Variant;

impl Dig2Variant for Psd2Variant {
    const FW_NAME: &'static str = "PSD2";

    /// PSD2 packs `charge_short` (the short-gate integral) into the per-event
    /// 2nd word at bits[41:26]. PHA2 leaves this slot unused.
    fn decode_energy_short(second_word: u64) -> u16 {
        ((second_word >> psd2_event_bits::ENERGY_SHORT_SHIFT)
            & psd2_event_bits::ENERGY_SHORT_MASK) as u16
    }

    /// PSD2 doesn't expose typed probe info on the wire — all probes are
    /// unsigned 14-bit ADC samples and probe identity comes from host-side
    /// configuration. Emit defaults (unsigned + UNKNOWN probe types) so the
    /// frontend falls back to generic "A0/A1/D0..D3" labels.
    fn parse_waveform_metadata(_wf_header: u64) -> WaveformMetadata {
        WaveformMetadata::default()
    }
}

// ---------------------------------------------------------------------------
// Tests — PSD2-specific paths only. Framing tests live in dualchannel_common.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::decoder::common::{DataType, RawData};
    use crate::reader::decoder::dualchannel_common::{
        START_SIGNAL_SIZE, STOP_SIGNAL_SIZE,
    };

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

    // -----------------------------------------------------------------------
    // decode_energy_short — the one PSD2-specific event path
    // -----------------------------------------------------------------------

    #[test]
    fn decode_energy_short_extracts_bits_41_26() {
        // Place 0x1234 at bits[41:26] of a u64 word.
        let value: u64 = 0x1234;
        let word = value << psd2_event_bits::ENERGY_SHORT_SHIFT;
        assert_eq!(Psd2Variant::decode_energy_short(word), 0x1234);
    }

    #[test]
    fn decode_energy_short_max_value_16_bit() {
        let value: u64 = 0xFFFF;
        let word = value << psd2_event_bits::ENERGY_SHORT_SHIFT;
        assert_eq!(Psd2Variant::decode_energy_short(word), 0xFFFF);
    }

    #[test]
    fn decode_energy_short_zero_when_slot_clear() {
        // Other bits set, target slot zero → energy_short = 0.
        let word: u64 = 0xFFFF_FFFF & !(0xFFFFu64 << psd2_event_bits::ENERGY_SHORT_SHIFT);
        assert_eq!(Psd2Variant::decode_energy_short(word), 0);
    }

    // -----------------------------------------------------------------------
    // parse_waveform_metadata — PSD2 always returns defaults
    // -----------------------------------------------------------------------

    #[test]
    fn parse_waveform_metadata_returns_unsigned_unknown_for_any_header() {
        // Even with arbitrary low bits set, PSD2 ignores them.
        let metadata = Psd2Variant::parse_waveform_metadata(0xFFFF_FFFF_FFFF_FFFF);
        assert!(!metadata.analog_probe1_is_signed);
        assert!(!metadata.analog_probe2_is_signed);
        assert_eq!(metadata.analog_probe_type[0], 0xFF);
        assert_eq!(metadata.analog_probe_type[1], 0xFF);
        for &dp in &metadata.digital_probe_type {
            assert_eq!(dp, 0xFF);
        }
    }

    // -----------------------------------------------------------------------
    // End-to-end smoke via the type alias — proves Dig2Decoder<Psd2Variant>
    // is wired correctly. Exhaustive framing tests live in dualchannel_common.
    // -----------------------------------------------------------------------

    fn build_psd2_event(
        channel: u8,
        ts: u64,
        energy: u16,
        energy_short: u16,
        fine_ts: u16,
    ) -> Vec<u8> {
        // Aggregate header (type=0x2, total_words=3) + 2-word event.
        let total_words: u64 = 3;
        let header = (0x2u64 << 60) | total_words;
        // event w1: bit63=0 (multi-word), channel, special=0, ts in low 48 bits
        let evt_w1 = ((channel as u64) << 56) | (ts & 0xFFFF_FFFF_FFFF);
        // event w2: bit63=1 (last), bit62=0 (no waveform), energy_short at
        // bits[41:26], fine_ts at bits[25:16], energy at bits[15:0]
        let evt_w2 = (1u64 << 63)
            | ((energy_short as u64 & 0xFFFF) << 26)
            | ((fine_ts as u64 & 0x3FF) << 16)
            | energy as u64;
        pack_be(&[header, evt_w1, evt_w2])
    }

    #[test]
    fn psd2_decoder_classifies_minimum_size_as_event() {
        let dec = Psd2Decoder::with_defaults();
        let raw = raw_data(vec![0u8; 16], 0);
        assert_eq!(dec.classify(&raw), DataType::Event);
    }

    #[test]
    fn psd2_decoder_decode_smoke_carries_charge_short() {
        let mut dec = Psd2Decoder::new(Psd2Config {
            time_step_ns: 2.0,
            module_id: 11,
            dump_enabled: false,
            num_channels: 32,
        });

        let bytes = build_psd2_event(/*ch*/ 5, /*ts*/ 1000, /*e*/ 4242, /*es*/ 567, /*fts*/ 100);
        let raw = raw_data(bytes, 1);
        let events = dec.decode(&raw);
        assert_eq!(events.len(), 1);

        let e = &events[0];
        assert_eq!(e.module, 11);
        assert_eq!(e.channel, 5);
        assert_eq!(e.energy, 4242);
        assert_eq!(e.energy_short, 567); // PSD2 carries this; PHA2 forces 0
        assert_eq!(e.fine_time, 100);
    }

    #[test]
    fn psd2_decoder_classifies_start_and_stop_signals() {
        let dec = Psd2Decoder::with_defaults();

        let mut start_data = vec![0u8; START_SIGNAL_SIZE];
        start_data[0] = 0x30; // type=3, subtype=0
        assert_eq!(dec.classify(&raw_data(start_data, 0)), DataType::Start);

        let mut stop_data = vec![0u8; STOP_SIGNAL_SIZE];
        stop_data[0] = 0x32; // type=3, subtype=2
        assert_eq!(dec.classify(&raw_data(stop_data, 0)), DataType::Stop);
    }

    #[test]
    fn psd2_variant_fw_name_is_psd2() {
        assert_eq!(Psd2Variant::FW_NAME, "PSD2");
    }
}

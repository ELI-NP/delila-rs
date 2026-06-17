//! AMax Decoder for DELILA Custom Firmware (Trapezoidal Filter MCA)
//!
//! Decodes 64-bit word format data from DELILA AMax firmware.
//! This is a custom firmware for nuclear spectroscopy using
//! trapezoidal filter and amplitude maximum detection.
//!
//! Data Format (per event):
//! - Word 0: Header (channel, timestamp)
//! - Word 1: Data (flags, PSD, fine_time, energy)
//! - Word 2+: User Words (variable, contains AMax value, baseline)
//!
//! Timestamp: 1 LSB = 8 ns

use super::common::{DataType, EventData, RawData, Waveform};

/// AMax debug FW probe-type codes. Carved from a reserved namespace
/// (PHA2 canonical codes occupy 0x00..0x0C, `UNKNOWN_PROBE_TYPE = 0xFF`).
/// The frontend label maps live in `web/operator-ui/src/app/models/histogram.types.ts`.
///
/// Spec ref: `FW/debug/debug_config.pdf` (block diagram) +
/// `FW/debug/AMAX_firmware32_channel_4input_caenlist.scf` Wire Merge U75
/// (`OutputOrder = IN_0 LEFT (MSBs)`, IN_0=Trigger_out, IN_1=BL_Hold,
/// IN_2=Energy_Dv, IN_3=shaping_dv, IN_4=shaping_track, IN_5..15=const 0).
pub mod amax_probe_types {
    pub const ANA_RAW: u8 = 0x40;
    pub const ANA_TRAP: u8 = 0x41;
    pub const ANA_TRIANGLE: u8 = 0x42;

    pub const DIG_TRIGGER_OUT: u8 = 0x40;
    pub const DIG_BL_HOLD: u8 = 0x41;
    pub const DIG_ENERGY_DV: u8 = 0x42;
    pub const DIG_SHAPING_DV: u8 = 0x43;
    pub const DIG_SHAPING_TRACK: u8 = 0x44;
    // Reserved namespace for future bits 10..0 of the digital lane:
    // 0x45..0x4F. When Rebeca wires up additional digital signals
    // (currently bits 10..0 are constant 0), assign them codes from
    // this range and add the bit extraction in `decode_debug_waveform`.
    // The carrier `Waveform` already has `digital_probe6..16` slots
    // ready (struct expansion 2026-05-08 per Round 2 plan H.2); only
    // the decoder + frontend label map (`DIGITAL_PROBE_TYPE_LABELS`
    // in `web/operator-ui/src/app/models/histogram.types.ts`) need
    // updating per new bit assignment.
}

/// AMax constants (64-bit words, Big Endian from digitizer)
mod constants {
    pub const WORD_SIZE: usize = 8;

    // Header word (Word 0)
    pub const LAST_WORD_SHIFT: u32 = 63;
    pub const CHANNEL_SHIFT: u32 = 56;
    pub const CHANNEL_MASK: u64 = 0x7F;
    pub const SPECIAL_EVENT_SHIFT: u32 = 55;
    pub const INFO_SHIFT: u32 = 51;
    pub const INFO_MASK: u64 = 0xF;
    pub const TIMESTAMP_MASK: u64 = 0x0000_FFFF_FFFF_FFFF; // 48-bit

    // Data word (Word 1)
    pub const WAVEFORM_FLAG_SHIFT: u32 = 62;
    pub const FLAGS_B_SHIFT: u32 = 50;
    pub const FLAGS_B_MASK: u64 = 0xFFF; // 12 bits
    pub const FLAGS_A_SHIFT: u32 = 42;
    pub const FLAGS_A_MASK: u64 = 0xFF; // 8 bits
    pub const PSD_SHIFT: u32 = 26;
    pub const PSD_MASK: u64 = 0xFFFF; // 16 bits
    pub const FINE_TIME_SHIFT: u32 = 16;
    pub const FINE_TIME_MASK: u64 = 0x3FF; // 10 bits
    pub const FINE_TIME_SCALE: f64 = 1024.0;
    pub const ENERGY_MASK: u64 = 0xFFFF; // 16 bits

    // Time step: 1 LSB = 8 ns
    pub const TIME_STEP_NS: f64 = 8.0;

    // User word
    pub const USER_LAST_WORD_SHIFT: u32 = 63;
    pub const USER_DATA_MASK: u64 = 0x7FFF_FFFF_FFFF_FFFF; // 63 bits

    // Waveform header
    pub const WAVEFORM_TRUNCATED_SHIFT: u32 = 63;
    pub const WAVEFORM_WORDS_MASK: u64 = 0xFFF; // 12 bits

    // Start/Stop signals
    pub const SIGNAL_TYPE_SHIFT: u32 = 60;
    pub const SIGNAL_SUBTYPE_SHIFT: u32 = 56;
    pub const START_SIGNAL_TYPE: u64 = 0x3;
    pub const START_SIGNAL_SUBTYPE: u64 = 0x0;
    pub const STOP_SIGNAL_TYPE: u64 = 0x3;
    pub const STOP_SIGNAL_SUBTYPE: u64 = 0x2;

    // Validation
    pub const MIN_EVENT_SIZE: usize = 4 * WORD_SIZE; // header + data + 2 user words
    pub const START_SIGNAL_SIZE: usize = 4 * WORD_SIZE;
    pub const STOP_SIGNAL_SIZE: usize = 3 * WORD_SIZE;
}

/// AMax Decoder configuration
#[derive(Debug, Clone)]
pub struct AMaxConfig {
    /// Module ID for identification
    pub module_id: u8,
    /// Enable dump output for debugging
    pub dump_enabled: bool,
    /// Number of physical channels (AMax typically uses only ch0)
    pub num_channels: u8,
}

impl Default for AMaxConfig {
    fn default() -> Self {
        Self {
            module_id: 0,
            dump_enabled: false,
            num_channels: 1, // AMax typically uses only ch0
        }
    }
}

/// AMax event with additional fields
#[derive(Debug, Clone, Default)]
pub struct AMaxEventData {
    /// Base event data
    pub base: EventData,
    /// AMax amplitude value from user words
    pub amax_value: Option<u64>,
    /// Baseline value from user words
    pub baseline: Option<u64>,
}

/// AMax Decoder for DELILA custom firmware
#[derive(Debug)]
pub struct AMaxDecoder {
    config: AMaxConfig,
    /// One-shot guard for the "first SE event seen" info log so we don't
    /// flood the log when ENABLE_ACQ=1 is sustained. Reset by reconstructing
    /// the decoder (Idle→Configure transition).
    se_logged: std::sync::atomic::AtomicBool,
}

impl Clone for AMaxDecoder {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            // Cloned decoders re-arm the one-shot log so each parallel
            // worker logs the first SE event it sees, matching the
            // dispatcher → workers pattern in DecodeLoop.
            se_logged: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl AMaxDecoder {
    /// Create a new AMax decoder with given configuration
    pub fn new(config: AMaxConfig) -> Self {
        Self {
            config,
            se_logged: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create a decoder with default configuration
    pub fn with_defaults() -> Self {
        Self::new(AMaxConfig::default())
    }

    /// Enable or disable dump output
    pub fn set_dump_enabled(&mut self, enabled: bool) {
        self.config.dump_enabled = enabled;
    }

    /// Classify the data type (Start/Stop/Event/Unknown)
    pub fn classify(&self, raw: &RawData) -> DataType {
        if raw.size < constants::MIN_EVENT_SIZE {
            // Check for stop signal (3 words = 24 bytes)
            if raw.size == constants::STOP_SIGNAL_SIZE && self.is_stop_signal(&raw.data) {
                return DataType::Stop;
            }
            return DataType::Unknown;
        }

        // Check for start signal (4 words = 32 bytes)
        if raw.size == constants::START_SIGNAL_SIZE && self.is_start_signal(&raw.data) {
            return DataType::Start;
        }

        DataType::Event
    }

    /// Decode raw data into AMax events
    pub fn decode(&self, raw: &RawData) -> Vec<AMaxEventData> {
        let mut events = Vec::new();
        self.decode_into(raw, &mut events);
        events
    }

    /// Decode raw data into provided vector (avoids allocation)
    pub fn decode_into(&self, raw: &RawData, events: &mut Vec<AMaxEventData>) {
        events.clear();

        // Skip start/stop signals
        match self.classify(raw) {
            DataType::Start | DataType::Stop | DataType::Unknown => return,
            DataType::Event => {}
        }

        if self.config.dump_enabled {
            self.dump_raw_data(raw);
        }

        let data = &raw.data;
        let total_words = data.len() / constants::WORD_SIZE;
        let mut word_index = 0;

        // `total_words - 1`: decode_event needs at least 2 words (header +
        // data) and returns None *without advancing* when fewer remain — a
        // trailing odd word would otherwise spin this loop forever.
        while word_index + 1 < total_words {
            if let Some(event) = self.decode_event(data, &mut word_index) {
                events.push(event);
            }
        }
        if word_index + 1 == total_words {
            tracing::warn!(
                total_words,
                "AMax raw decode: trailing odd word ignored (truncated event?)"
            );
        }

        if self.config.dump_enabled {
            println!(
                "[AMax] Decoded {} events from {} words",
                events.len(),
                total_words
            );
        }
    }

    /// Decode a single event from raw data
    fn decode_event(&self, data: &[u8], word_index: &mut usize) -> Option<AMaxEventData> {
        let total_words = data.len() / constants::WORD_SIZE;

        // Need at least 2 words (header + data)
        if *word_index + 1 >= total_words {
            return None;
        }

        // Read Word 0 (Header)
        let word0 = self.read_u64(data, *word_index);
        *word_index += 1;

        let _is_last_word = ((word0 >> constants::LAST_WORD_SHIFT) & 0x1) != 0;
        let channel = ((word0 >> constants::CHANNEL_SHIFT) & constants::CHANNEL_MASK) as u8;
        let special_event = ((word0 >> constants::SPECIAL_EVENT_SHIFT) & 0x1) != 0;
        let info = ((word0 >> constants::INFO_SHIFT) & constants::INFO_MASK) as u8;
        let raw_timestamp = word0 & constants::TIMESTAMP_MASK;

        // SE (Special Event) bit drives the AMax debug FW path. When the
        // FW's `ENABLE_ACQ` register is 1, ch0 events arrive with SE=true
        // and a 4-lane debug WAVE payload (raw / trap / triangle / digital
        // 16-bit). The legacy code path discarded SE events because pre-
        // `ENABLE_ACQ` firmwares could never raise SE; we now route them
        // into `decode_debug_waveform` instead. Spec ref:
        // `FW/debug/debug_config.pdf` + email_from_Rebeca (2026-05-07).
        if special_event
            && !self
                .se_logged
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            tracing::info!(
                channel,
                info = format!("0x{:X}", info),
                "[AMax] first SE event seen — entering debug-wave decode path"
            );
        }

        // Read Word 1 (Data)
        let word1 = self.read_u64(data, *word_index);
        *word_index += 1;

        let data_is_last = ((word1 >> constants::LAST_WORD_SHIFT) & 0x1) != 0;
        let has_waveform = ((word1 >> constants::WAVEFORM_FLAG_SHIFT) & 0x1) != 0;
        let flags_b = ((word1 >> constants::FLAGS_B_SHIFT) & constants::FLAGS_B_MASK) as u16;
        let flags_a = ((word1 >> constants::FLAGS_A_SHIFT) & constants::FLAGS_A_MASK) as u8;
        let psd = ((word1 >> constants::PSD_SHIFT) & constants::PSD_MASK) as u16;
        let fine_time = ((word1 >> constants::FINE_TIME_SHIFT) & constants::FINE_TIME_MASK) as u16;
        let energy = (word1 & constants::ENERGY_MASK) as u16;

        // Calculate timestamp
        let coarse_time_ns = (raw_timestamp as f64) * constants::TIME_STEP_NS;
        let fine_time_ns =
            (fine_time as f64 / constants::FINE_TIME_SCALE) * constants::TIME_STEP_NS;
        let timestamp_ns = coarse_time_ns + fine_time_ns;

        // Combine flags
        let flags = ((flags_a as u32) << 12) | (flags_b as u32);

        // Read User Words. AMax FW emits up to 4 user words per event:
        // [0] = AMax peak value, [1] = baseline, [2..=3] = FW-specific.
        // We pack all 4 into the EventData.user_info fixed-size array;
        // slots beyond 4 are dropped silently (rare; would force a Vec
        // on the hot path otherwise). `amax_value` / `baseline` aliases
        // are retained for legacy callers via `AMaxEventData`.
        let mut user_info = [0u64; 4];
        let mut user_word_index = 0usize;

        // User words (terminated by bit 63) ALWAYS precede the waveform block
        // when the data word is not the last word — see the FW raw event
        // layout (`open-dpp_single_event.png`): word0, word1, user_word#0..W-1
        // (last has bit63=1), then the waveform header + samples. The previous
        // `&& !has_waveform` guard skipped the user words on waveform events,
        // so `decode_waveform` started reading at a user word and desynced the
        // whole buffer (ch0 over-counted + trailing-odd-word warnings,
        // 2026-06-11 RawUDP bring-up).
        if !data_is_last {
            while *word_index < total_words {
                let user_word = self.read_u64(data, *word_index);
                *word_index += 1;

                let is_last = (user_word >> constants::USER_LAST_WORD_SHIFT) & 0x1 != 0;
                let user_data = user_word & constants::USER_DATA_MASK;

                if user_word_index < user_info.len() {
                    user_info[user_word_index] = user_data;
                } else if self.config.dump_enabled {
                    println!(
                        "[AMax] Extra user word {} (dropped): 0x{:016X}",
                        user_word_index, user_data
                    );
                }
                user_word_index += 1;

                if is_last {
                    break;
                }
            }
        }

        // Legacy aliases: preserve the previous semantics (None when slot=0)
        // so existing AMaxEventData consumers don't see ghost zero values.
        let amax_value = (user_info[0] != 0).then_some(user_info[0]);
        let baseline = (user_info[1] != 0 || amax_value.is_none()).then_some(user_info[1]);

        // Handle waveform if present. SE=true → debug FW packs 4 lanes
        // (raw / trap / triangle / 16-bit digital) into the same WAVE field
        // (`decode_debug_waveform`); SE=false → legacy single-lane raw
        // (`decode_waveform`).
        let waveform = if has_waveform {
            if special_event {
                self.decode_debug_waveform(data, word_index)
            } else {
                self.decode_waveform(data, word_index)
            }
        } else {
            None
        };

        if self.config.dump_enabled {
            println!("--- AMax Event ---");
            println!("  Channel:      {}", channel);
            println!("  Timestamp:    {:.3} ns", timestamp_ns);
            println!("  Energy:       {}", energy);
            println!("  PSD:          {}", psd);
            println!("  Fine Time:    {}", fine_time);
            println!("  Flags:        0x{:05x}", flags);
            println!("  AMax Value:   {:?}", amax_value);
            println!("  Baseline:     {:?}", baseline);
            println!("  Has Waveform: {}", has_waveform);
        }

        Some(AMaxEventData {
            base: EventData {
                timestamp_ns,
                module: self.config.module_id,
                channel,
                energy,
                energy_short: psd, // Use energy_short for PSD value
                fine_time,
                flags,
                user_info,
                waveform,
            },
            amax_value,
            baseline,
        })
    }

    /// Decode waveform data
    fn decode_waveform(&self, data: &[u8], word_index: &mut usize) -> Option<Waveform> {
        let total_words = data.len() / constants::WORD_SIZE;

        if *word_index >= total_words {
            return None;
        }

        // Read waveform header
        let header = self.read_u64(data, *word_index);
        *word_index += 1;

        let _truncated = (header >> constants::WAVEFORM_TRUNCATED_SHIFT) & 0x1;
        let wave_word_count = (header & constants::WAVEFORM_WORDS_MASK) as usize;

        if self.config.dump_enabled {
            println!(
                "[AMax] Waveform: {} words = {} samples",
                wave_word_count,
                wave_word_count * 4
            );
        }

        // Read waveform samples (4 samples per 64-bit word)
        let mut analog_probe1 = Vec::with_capacity(wave_word_count * 4);

        for _ in 0..wave_word_count {
            if *word_index >= total_words {
                break;
            }

            let word = self.read_u64(data, *word_index);
            *word_index += 1;

            // Extract 4 samples (16 bits each)
            analog_probe1.push((word & 0xFFFF) as i16);
            analog_probe1.push(((word >> 16) & 0xFFFF) as i16);
            analog_probe1.push(((word >> 32) & 0xFFFF) as i16);
            analog_probe1.push(((word >> 48) & 0xFFFF) as i16);
        }

        Some(Waveform {
            analog_probe1,
            analog_probe2: Vec::new(),
            analog_probe3: Vec::new(),
            digital_probe1: Vec::new(),
            digital_probe2: Vec::new(),
            digital_probe3: Vec::new(),
            digital_probe4: Vec::new(),
            digital_probe5: Vec::new(),
            digital_probe6: Vec::new(),
            digital_probe7: Vec::new(),
            digital_probe8: Vec::new(),
            digital_probe9: Vec::new(),
            digital_probe10: Vec::new(),
            digital_probe11: Vec::new(),
            digital_probe12: Vec::new(),
            digital_probe13: Vec::new(),
            digital_probe14: Vec::new(),
            digital_probe15: Vec::new(),
            digital_probe16: Vec::new(),
            time_resolution: 0,
            trigger_threshold: 0,
            ns_per_sample: constants::TIME_STEP_NS,
            // AMax FW emits the raw 14-bit ADC stream — unsigned.
            analog_probe1_is_signed: false,
            analog_probe2_is_signed: false,
            analog_probe3_is_signed: false,
            // AMax custom OpenDPP FW doesn't carry typed probe info on
            // the wire; emit UNKNOWN.
            analog_probe_type: [crate::reader::decoder::common::UNKNOWN_PROBE_TYPE; 3],
            digital_probe_type: [crate::reader::decoder::common::UNKNOWN_PROBE_TYPE; 16],
        })
    }

    /// Decode a 4-lane debug waveform produced by the AMax debug FW
    /// (`ENABLE_ACQ = 1`, SE bit set on ch0 events).
    ///
    /// Each 64-bit word holds **one sample × 4 lanes × 16 bits**, packed by
    /// the Sci-compiler "TM packer" + 2:1 MUX:
    ///
    /// | bits  | lane | content                       | DELILA slot       |
    /// |-------|------|-------------------------------|-------------------|
    /// | 0-15  | 0    | raw waveform (signed 16-bit)  | `analog_probe1`   |
    /// | 16-31 | 1    | trapezoidal filter (signed)   | `analog_probe2`   |
    /// | 32-47 | 2    | triangle filter (signed)      | `analog_probe3`   |
    /// | 48-63 | 3    | digital lane (16 wires)       | `digital_probe1..5` |
    ///
    /// Digital lane bit map (Wire Merge U75, `IN_0 LEFT (MSBs)`):
    ///   bit15=Trigger_out, bit14=BL_Hold, bit13=Energy_Dv,
    ///   bit12=shaping_dv, bit11=shaping_track, bits10..0 = const 0 (padding).
    ///
    /// Spec ref: `FW/debug/debug_config.pdf` (block diagram) +
    /// `FW/debug/AMAX_firmware32_channel_4input_caenlist.scf` U75 connections.
    fn decode_debug_waveform(&self, data: &[u8], word_index: &mut usize) -> Option<Waveform> {
        let total_words = data.len() / constants::WORD_SIZE;

        if *word_index >= total_words {
            return None;
        }

        // Same waveform header layout as the legacy path.
        let header = self.read_u64(data, *word_index);
        *word_index += 1;

        let _truncated = (header >> constants::WAVEFORM_TRUNCATED_SHIFT) & 0x1;
        let wave_word_count = (header & constants::WAVEFORM_WORDS_MASK) as usize;

        if self.config.dump_enabled {
            println!(
                "[AMax/debug] Waveform: {} words = {} samples (4 lanes)",
                wave_word_count, wave_word_count
            );
        }

        let mut analog_probe1 = Vec::with_capacity(wave_word_count); // raw
        let mut analog_probe2 = Vec::with_capacity(wave_word_count); // trap
        let mut analog_probe3 = Vec::with_capacity(wave_word_count); // triangle
        let mut digital_probe1 = Vec::with_capacity(wave_word_count); // Trigger_out
        let mut digital_probe2 = Vec::with_capacity(wave_word_count); // BL_Hold
        let mut digital_probe3 = Vec::with_capacity(wave_word_count); // Energy_Dv
        let mut digital_probe4 = Vec::with_capacity(wave_word_count); // shaping_dv
        let mut digital_probe5 = Vec::with_capacity(wave_word_count); // shaping_track

        for _ in 0..wave_word_count {
            if *word_index >= total_words {
                break;
            }

            let word = self.read_u64(data, *word_index);
            *word_index += 1;

            // Lane unpack — 4 × 16 bits, lane 0 is the LSBs.
            analog_probe1.push((word & 0xFFFF) as i16);
            analog_probe2.push(((word >> 16) & 0xFFFF) as i16);
            analog_probe3.push(((word >> 32) & 0xFFFF) as i16);

            let dig = (word >> 48) & 0xFFFF;
            digital_probe1.push(((dig >> 15) & 0x1) as u8);
            digital_probe2.push(((dig >> 14) & 0x1) as u8);
            digital_probe3.push(((dig >> 13) & 0x1) as u8);
            digital_probe4.push(((dig >> 12) & 0x1) as u8);
            digital_probe5.push(((dig >> 11) & 0x1) as u8);
        }

        Some(Waveform {
            analog_probe1,
            analog_probe2,
            analog_probe3,
            digital_probe1,
            digital_probe2,
            digital_probe3,
            digital_probe4,
            digital_probe5,
            // Slots 6..16 reserved for future digital-lane bit assignments
            // (currently bits 10..0 are constant 0 in hardware — see
            // `amax_probe_types` mod and Round 2 plan H.2).
            digital_probe6: Vec::new(),
            digital_probe7: Vec::new(),
            digital_probe8: Vec::new(),
            digital_probe9: Vec::new(),
            digital_probe10: Vec::new(),
            digital_probe11: Vec::new(),
            digital_probe12: Vec::new(),
            digital_probe13: Vec::new(),
            digital_probe14: Vec::new(),
            digital_probe15: Vec::new(),
            digital_probe16: Vec::new(),
            time_resolution: 0,
            trigger_threshold: 0,
            ns_per_sample: constants::TIME_STEP_NS,
            // Debug-FW analog lanes are signed 16-bit per Rebeca's spec
            // (raw is the signed ADC stream after the TM packer; trap and
            // triangle are signed filter outputs that swing around 0).
            analog_probe1_is_signed: true,
            analog_probe2_is_signed: true,
            analog_probe3_is_signed: true,
            analog_probe_type: [
                amax_probe_types::ANA_RAW,
                amax_probe_types::ANA_TRAP,
                amax_probe_types::ANA_TRIANGLE,
            ],
            digital_probe_type: [
                amax_probe_types::DIG_TRIGGER_OUT,
                amax_probe_types::DIG_BL_HOLD,
                amax_probe_types::DIG_ENERGY_DV,
                amax_probe_types::DIG_SHAPING_DV,
                amax_probe_types::DIG_SHAPING_TRACK,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
                crate::reader::decoder::common::UNKNOWN_PROBE_TYPE,
            ],
        })
    }

    /// Check if data is a start signal
    fn is_start_signal(&self, data: &[u8]) -> bool {
        if data.len() < constants::START_SIGNAL_SIZE {
            return false;
        }

        let word0 = self.read_u64(data, 0);
        let signal_type = (word0 >> constants::SIGNAL_TYPE_SHIFT) & 0xF;
        let signal_subtype = (word0 >> constants::SIGNAL_SUBTYPE_SHIFT) & 0xF;

        signal_type == constants::START_SIGNAL_TYPE
            && signal_subtype == constants::START_SIGNAL_SUBTYPE
    }

    /// Check if data is a stop signal
    fn is_stop_signal(&self, data: &[u8]) -> bool {
        if data.len() < constants::STOP_SIGNAL_SIZE {
            return false;
        }

        let word0 = self.read_u64(data, 0);
        let signal_type = (word0 >> constants::SIGNAL_TYPE_SHIFT) & 0xF;
        let signal_subtype = (word0 >> constants::SIGNAL_SUBTYPE_SHIFT) & 0xF;

        signal_type == constants::STOP_SIGNAL_TYPE
            && signal_subtype == constants::STOP_SIGNAL_SUBTYPE
    }

    /// Read a 64-bit word from data (big-endian)
    fn read_u64(&self, data: &[u8], word_index: usize) -> u64 {
        let offset = word_index * constants::WORD_SIZE;
        u64::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ])
    }

    /// Dump raw data for debugging
    fn dump_raw_data(&self, raw: &RawData) {
        println!();
        println!("=== AMax Raw Data ===");
        println!("Size: {} bytes, Events: {}", raw.size, raw.n_events);

        let num_words = raw.data.len() / constants::WORD_SIZE;
        for i in 0..num_words.min(16) {
            let word = self.read_u64(&raw.data, i);
            println!("  Word {:2}: 0x{:016X}", i, word);
        }
        if num_words > 16 {
            println!("  ... ({} more words)", num_words - 16);
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_creation() {
        let decoder = AMaxDecoder::with_defaults();
        assert_eq!(decoder.config.module_id, 0);
        assert_eq!(decoder.config.num_channels, 1);
    }

    #[test]
    fn test_decoder_with_config() {
        let config = AMaxConfig {
            module_id: 3,
            dump_enabled: true,
            num_channels: 1,
        };
        let decoder = AMaxDecoder::new(config);
        assert_eq!(decoder.config.module_id, 3);
        assert!(decoder.config.dump_enabled);
    }

    #[test]
    fn test_classify_start_signal() {
        let decoder = AMaxDecoder::with_defaults();

        // Start signal: [0x3][0x0][reserved][0x3][0x4]
        let start_data: [u8; 32] = [
            0x30, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, // Word 0
            0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // Word 1
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // Word 2
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Word 3
        ];

        let raw = RawData {
            data: start_data.to_vec(),
            size: 32,
            n_events: 1,
        };

        assert!(matches!(decoder.classify(&raw), DataType::Start));
    }

    #[test]
    fn test_read_u64_big_endian() {
        let decoder = AMaxDecoder::with_defaults();
        let data: [u8; 8] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let word = decoder.read_u64(&data, 0);
        assert_eq!(word, 0x0001020304050607);
    }

    #[test]
    fn test_timestamp_calculation() {
        // 48-bit timestamp with 8ns LSB
        let raw_timestamp: u64 = 1000;
        let fine_time: u16 = 512; // Half of 1024

        let coarse_ns = (raw_timestamp as f64) * 8.0;
        let fine_ns = (fine_time as f64 / 1024.0) * 8.0;
        let total_ns = coarse_ns + fine_ns;

        // 1000 * 8 + 0.5 * 8 = 8000 + 4 = 8004
        assert!((total_ns - 8004.0).abs() < 0.001);
    }

    #[test]
    fn test_decode_event_structure() {
        let decoder = AMaxDecoder::with_defaults();

        // Simulated event data (4 words = 32 bytes)
        // Word 0: channel=0, timestamp=1000
        // Word 1: energy=5000, fine_time=100
        // Word 2: user word (amax value)
        // Word 3: user word with last bit (baseline)
        let event_data: [u8; 32] = [
            // Word 0: 0x00000000000003E8 (timestamp=1000)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0xE8,
            // Word 1: energy=5000, fine_time=100, psd=200
            0x00, 0x00, 0x00, 0x32, 0x00, 0x64, 0x13,
            0x88, // psd=200<<26, fine=100<<16, energy=5000
            // Word 2: user word (amax=1000)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0xE8,
            // Word 3: last user word (baseline=500)
            0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xF4,
        ];

        let raw = RawData {
            data: event_data.to_vec(),
            size: 32,
            n_events: 1,
        };

        let events = decoder.decode(&raw);
        assert_eq!(events.len(), 1);

        let event = &events[0];
        assert_eq!(event.base.channel, 0);
        // Note: exact values depend on bit field parsing which may need adjustment
    }

    #[test]
    fn test_constants() {
        assert_eq!(constants::WORD_SIZE, 8);
        assert_eq!(constants::TIME_STEP_NS, 8.0);
        assert_eq!(constants::FINE_TIME_SCALE, 1024.0);
    }

    /// Pack u64 words into a big-endian byte vector (digitizer wire format).
    fn pack_be(words: &[u64]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn decode_event_legacy_path_unchanged_when_se_false() {
        // Regression guard: SE=0 events must keep going through the legacy
        // single-lane waveform decoder (`decode_waveform`) — analog_probe1
        // only, no probe-type tags, unsigned samples. This pins the
        // `ENABLE_ACQ=0` mode behaviour after the SE branch was added.
        let decoder = AMaxDecoder::with_defaults();

        // Word 0: SE=0, channel=0, last_word=0, info=0, raw_timestamp=0x12345
        let word0: u64 = 0x0000_0000_0001_2345;
        // Word 1: last_word=0, has_waveform=1 (bit62), energy=42
        let word1: u64 = (1u64 << constants::WAVEFORM_FLAG_SHIFT) | 42;
        // Waveform header: 1 word count, truncated=0
        let wave_hdr: u64 = 0x0000_0000_0000_0001;
        // Wave word: samples [1, 2, 3, 4] (i16) packed as 4×16 bits, lane 0 LSBs.
        // Wave words have NO last_word reservation — bit 63 is the high bit of
        // the 4th lane's sample. The outer `decode_into` loop terminates on
        // word_index == total_words instead.
        let wave_word: u64 = ((4u64 & 0xFFFF) << 48)
            | ((3u64 & 0xFFFF) << 32)
            | ((2u64 & 0xFFFF) << 16)
            | (1u64 & 0xFFFF);
        // User-word block: real events always emit a bit63-terminated user-word
        // block between word1 and the waveform header, and the decoder reads it
        // for any non-last data word (see the `!data_is_last` guard). A single
        // terminated word here keeps the waveform aligned.
        let user_word: u64 = (1u64 << constants::USER_LAST_WORD_SHIFT) | 0xABCD;

        let bytes = pack_be(&[word0, word1, user_word, wave_hdr, wave_word]);
        let mut raw = RawData::new(bytes);
        raw.n_events = 1;

        let events = decoder.decode(&raw);
        assert_eq!(events.len(), 1);

        let event = &events[0];
        assert_eq!(event.base.channel, 0);
        assert_eq!(event.base.energy, 42);

        let wf = event
            .base
            .waveform
            .as_ref()
            .expect("legacy SE=0 path must produce a waveform when has_waveform=1");
        assert_eq!(wf.analog_probe1, vec![1i16, 2, 3, 4]);
        assert!(wf.analog_probe2.is_empty());
        assert!(wf.analog_probe3.is_empty());
        assert!(wf.digital_probe1.is_empty());
        assert!(wf.digital_probe2.is_empty());
        assert!(wf.digital_probe3.is_empty());
        assert!(wf.digital_probe4.is_empty());
        assert!(wf.digital_probe5.is_empty());
        assert!(!wf.analog_probe1_is_signed);
        assert_eq!(
            wf.analog_probe_type,
            [crate::reader::decoder::common::UNKNOWN_PROBE_TYPE; 3]
        );
        assert_eq!(
            wf.digital_probe_type,
            [crate::reader::decoder::common::UNKNOWN_PROBE_TYPE; 16]
        );
    }

    #[test]
    fn decode_event_debug_path_when_se_true() {
        // SE=1 → ENABLE_ACQ=1 debug FW path. Same envelope as the legacy
        // test above, but the wave word now carries 4 lanes (raw / trap /
        // triangle / 16-bit digital). All 5 digital probes lit (bits 15..11).
        let decoder = AMaxDecoder::with_defaults();

        // Word 0: SE=1 (bit 55), channel=0, raw_timestamp=0x12345
        let word0: u64 = (1u64 << constants::SPECIAL_EVENT_SHIFT) | 0x0001_2345;
        // Word 1: has_waveform=1, energy=42 (same as legacy test)
        let word1: u64 = (1u64 << constants::WAVEFORM_FLAG_SHIFT) | 42;
        // Waveform header: 1 word count
        let wave_hdr: u64 = 0x0000_0000_0000_0001;
        // Lane 0 = 0x0102, lane 1 = 0x0304, lane 2 = 0x0506,
        // lane 3 = 0xF800 (bits 15,14,13,12,11 set, padding 0).
        // No last_word bit on wave words (it would alias into lane 3 bit 15).
        let wave_word: u64 =
            ((0xF800u64) << 48) | ((0x0506u64) << 32) | ((0x0304u64) << 16) | 0x0102u64;
        // Bit63-terminated user-word block before the waveform (see test above).
        let user_word: u64 = (1u64 << constants::USER_LAST_WORD_SHIFT) | 0xABCD;

        let bytes = pack_be(&[word0, word1, user_word, wave_hdr, wave_word]);
        let mut raw = RawData::new(bytes);
        raw.n_events = 1;

        let events = decoder.decode(&raw);
        assert_eq!(events.len(), 1);

        let event = &events[0];
        assert_eq!(event.base.channel, 0);
        assert_eq!(event.base.energy, 42);

        let wf = event
            .base
            .waveform
            .as_ref()
            .expect("SE=1 debug path must produce a waveform when has_waveform=1");
        assert_eq!(wf.analog_probe1, vec![0x0102i16]);
        assert_eq!(wf.analog_probe2, vec![0x0304i16]);
        assert_eq!(wf.analog_probe3, vec![0x0506i16]);
        assert_eq!(wf.digital_probe1, vec![1u8]);
        assert_eq!(wf.digital_probe2, vec![1u8]);
        assert_eq!(wf.digital_probe3, vec![1u8]);
        assert_eq!(wf.digital_probe4, vec![1u8]);
        assert_eq!(wf.digital_probe5, vec![1u8]);
        assert!(wf.analog_probe1_is_signed);
        assert!(wf.analog_probe2_is_signed);
        assert!(wf.analog_probe3_is_signed);
        assert_eq!(
            wf.analog_probe_type,
            [
                amax_probe_types::ANA_RAW,
                amax_probe_types::ANA_TRAP,
                amax_probe_types::ANA_TRIANGLE,
            ]
        );
        // Slots 5..15 are reserved for future digital-lane bit assignments;
        // padded with UNKNOWN_PROBE_TYPE today.
        let mut expected = [crate::reader::decoder::common::UNKNOWN_PROBE_TYPE; 16];
        expected[0] = amax_probe_types::DIG_TRIGGER_OUT;
        expected[1] = amax_probe_types::DIG_BL_HOLD;
        expected[2] = amax_probe_types::DIG_ENERGY_DV;
        expected[3] = amax_probe_types::DIG_SHAPING_DV;
        expected[4] = amax_probe_types::DIG_SHAPING_TRACK;
        assert_eq!(wf.digital_probe_type, expected);
    }

    #[test]
    fn decode_debug_waveform_bit_unpack_per_signal() {
        // Pin the digital lane bit-position mapping so future edits can't
        // silently swap probe slots: each sample lights exactly one bit in
        // the 16-bit digital lane (bits 15..11), and we expect that bit to
        // land in the matching probe vec at the matching sample index.
        let decoder = AMaxDecoder::with_defaults();

        // Same header/data envelope as test 2.
        let word0: u64 = (1u64 << constants::SPECIAL_EVENT_SHIFT) | 0x0001_2345;
        let word1: u64 = (1u64 << constants::WAVEFORM_FLAG_SHIFT) | 42;
        // 5 wave words (samples 0..4)
        let wave_hdr: u64 = 0x0000_0000_0000_0005;

        // Build per-sample digital lanes. Lanes 0/1/2 = 0 (analog probes
        // checked elsewhere); lane 3 lights one bit per sample.
        let lane3_per_sample: [u64; 5] = [
            0x8000, // sample 0: bit 15 → digital_probe1 (Trigger_out)
            0x4000, // sample 1: bit 14 → digital_probe2 (BL_Hold)
            0x2000, // sample 2: bit 13 → digital_probe3 (Energy_Dv)
            0x1000, // sample 3: bit 12 → digital_probe4 (shaping_dv)
            0x0800, // sample 4: bit 11 → digital_probe5 (shaping_track)
        ];
        let mut wave_words = Vec::with_capacity(5);
        for dig in lane3_per_sample.iter() {
            // No last_word bit on wave words (would alias into the same
            // bit-15 slot as Trigger_out and corrupt digital_probe1).
            wave_words.push((dig & 0xFFFF) << 48);
        }

        // Bit63-terminated user-word block before the waveform (see tests above).
        let user_word: u64 = (1u64 << constants::USER_LAST_WORD_SHIFT) | 0xABCD;
        let mut all_words = vec![word0, word1, user_word, wave_hdr];
        all_words.extend(wave_words);
        let bytes = pack_be(&all_words);
        let mut raw = RawData::new(bytes);
        raw.n_events = 1;

        let events = decoder.decode(&raw);
        assert_eq!(events.len(), 1);

        let wf = events[0]
            .base
            .waveform
            .as_ref()
            .expect("SE=1 debug waveform expected");

        assert_eq!(wf.digital_probe1, vec![1u8, 0, 0, 0, 0]); // Trigger_out @ s0
        assert_eq!(wf.digital_probe2, vec![0u8, 1, 0, 0, 0]); // BL_Hold @ s1
        assert_eq!(wf.digital_probe3, vec![0u8, 0, 1, 0, 0]); // Energy_Dv @ s2
        assert_eq!(wf.digital_probe4, vec![0u8, 0, 0, 1, 0]); // shaping_dv @ s3
        assert_eq!(wf.digital_probe5, vec![0u8, 0, 0, 0, 1]); // shaping_track @ s4

        // Sanity: analog lanes were all zero in this test.
        assert_eq!(wf.analog_probe1, vec![0i16; 5]);
        assert_eq!(wf.analog_probe2, vec![0i16; 5]);
        assert_eq!(wf.analog_probe3, vec![0i16; 5]);
    }
}

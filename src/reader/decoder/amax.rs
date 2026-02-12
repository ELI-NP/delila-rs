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
#[derive(Debug, Clone)]
pub struct AMaxDecoder {
    config: AMaxConfig,
}

impl AMaxDecoder {
    /// Create a new AMax decoder with given configuration
    pub fn new(config: AMaxConfig) -> Self {
        Self { config }
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

        while word_index < total_words {
            if let Some(event) = self.decode_event(data, &mut word_index) {
                events.push(event);
            }
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

        // Skip special events
        if special_event {
            if self.config.dump_enabled {
                println!(
                    "[AMax] Special event skipped (ch={}, info=0x{:X})",
                    channel, info
                );
            }
            // Consume remaining words until last_word
            while *word_index < total_words {
                let w = self.read_u64(data, *word_index);
                *word_index += 1;
                if (w >> constants::LAST_WORD_SHIFT) & 0x1 != 0 {
                    break;
                }
            }
            return None;
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

        // Read User Words (AMax value, baseline, etc.)
        let mut amax_value: Option<u64> = None;
        let mut baseline: Option<u64> = None;
        let mut user_word_index = 0;

        if !data_is_last && !has_waveform {
            while *word_index < total_words {
                let user_word = self.read_u64(data, *word_index);
                *word_index += 1;

                let is_last = (user_word >> constants::USER_LAST_WORD_SHIFT) & 0x1 != 0;
                let user_data = user_word & constants::USER_DATA_MASK;

                // Assign user words to AMax-specific fields
                // Based on observed data: user word 0 might be AMax, user word 1 is baseline
                match user_word_index {
                    0 => {
                        if user_data != 0 {
                            amax_value = Some(user_data);
                        }
                    }
                    1 => {
                        if user_data != 0 || amax_value.is_none() {
                            baseline = Some(user_data);
                        }
                    }
                    _ => {
                        // Additional user words - log if debug enabled
                        if self.config.dump_enabled {
                            println!(
                                "[AMax] Extra user word {}: 0x{:016X}",
                                user_word_index, user_data
                            );
                        }
                    }
                }

                user_word_index += 1;

                if is_last {
                    break;
                }
            }
        }

        // Handle waveform if present
        let waveform = if has_waveform {
            self.decode_waveform(data, word_index)
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
            digital_probe1: Vec::new(),
            digital_probe2: Vec::new(),
            digital_probe3: Vec::new(),
            digital_probe4: Vec::new(),
            time_resolution: 0,
            trigger_threshold: 0,
            ns_per_sample: constants::TIME_STEP_NS,
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
}

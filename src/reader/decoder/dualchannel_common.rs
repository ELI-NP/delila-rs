//! Common framing for DPP-PSD2 / DPP-PHA2 (VX27xx / VX274x family) decoders.
//!
//! PSD2 and PHA2 share the entire DIG2 RAW endpoint Individual-Trigger-Mode
//! envelope (`format=0x2`): 64-bit big-endian aggregate header, per-event
//! first/second words, optional waveform extras (header + size + N samples).
//! The only per-firmware divergence sits in **two** places:
//!
//! | What | PSD2 | PHA2 |
//! |---|---|---|
//! | `energy_short` (event 2nd word bits[41:26]) | charge_short integral | unused → 0 |
//! | Waveform header low bits | trigger_threshold + reserved | per-probe `is_signed` + probe-type IDs |
//!
//! Sample bit-packing is identical (two 14-bit half-samples per 32-bit
//! half-word + 4 digital-probe bits). Sign-extension differs **per probe** in
//! PHA2 only; PSD2 always reads unsigned.
//!
//! The generic [`Dig2Decoder<V>`] zero-cost-monomorphizes over a variant via
//! the [`Dig2Variant`] trait. Hot-path overhead is the same as the original
//! hand-written decoders — both `cargo --release` builds inline the trait
//! calls into the framing loop.
//!
//! # Hot-path policy
//!
//! See `decoder/mod.rs` "Hot-path heuristic policy" — the framing layer here
//! trusts `total_words` (aggregate header) and `n_waveform_words` (size word)
//! from the FW and never second-guesses with bit-pattern heuristics during
//! sample decoding. The PHA2 truncation regression of 2026-05-04 was the
//! reason that policy exists.

use std::marker::PhantomData;

use super::common::{
    sign_extend_14bit, DataType, EventData, RawData, Waveform, UNKNOWN_PROBE_TYPE,
};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Shared constants (PSD2 + PHA2 identical)
// ---------------------------------------------------------------------------

pub const WORD_SIZE: usize = 8;
pub const MIN_DATA_SIZE: usize = 2 * WORD_SIZE;
pub const START_SIGNAL_SIZE: usize = 4 * WORD_SIZE;
pub const STOP_SIGNAL_SIZE: usize = 3 * WORD_SIZE;

pub mod aggregate_header {
    pub const TYPE_SHIFT: u32 = 60;
    pub const TYPE_MASK: u64 = 0xF;
    pub const TYPE_DATA: u64 = 0x2;
    pub const FAIL_CHECK_SHIFT: u32 = 56;
    pub const FAIL_CHECK_MASK: u64 = 0x1;
    pub const COUNTER_SHIFT: u32 = 32;
    pub const COUNTER_MASK: u64 = 0xFFFF;
    pub const TOTAL_SIZE_MASK: u64 = 0xFFFF_FFFF;
}

pub mod event_word {
    // First word
    pub const LAST_WORD_SHIFT: u32 = 63;
    pub const CHANNEL_SHIFT: u32 = 56;
    pub const CHANNEL_MASK: u64 = 0x7F;
    pub const SPECIAL_EVENT_SHIFT: u32 = 55;
    pub const SPECIAL_EVENT_MASK: u64 = 0x1;
    pub const TIMESTAMP_MASK: u64 = 0xFFFF_FFFF_FFFF;
    /// Single-word event: timestamp truncated to 32 bits.
    pub const TIMESTAMP_REDUCED_MASK: u64 = 0xFFFF_FFFF;

    // Second word
    pub const WAVEFORM_FLAG_SHIFT: u32 = 62;
    pub const FLAGS_LOW_PRIORITY_SHIFT: u32 = 50;
    pub const FLAGS_LOW_PRIORITY_MASK: u64 = 0xFFF; // 12 bits
    pub const FLAGS_HIGH_PRIORITY_SHIFT: u32 = 42;
    pub const FLAGS_HIGH_PRIORITY_MASK: u64 = 0xFF; // 8 bits
    pub const FINE_TIME_SHIFT: u32 = 16;
    pub const FINE_TIME_MASK: u64 = 0x3FF;
    pub const FINE_TIME_SCALE: f64 = 1024.0;
    pub const ENERGY_MASK: u64 = 0xFFFF;

    /// Single-word compressed event: flag-high lives at bits[55:48] of the
    /// (only) word.
    pub const SINGLE_WORD_FLAG_HIGH_SHIFT: u32 = 48;
}

pub mod waveform_header {
    pub const CHECK1_SHIFT: u32 = 63;
    pub const CHECK2_SHIFT: u32 = 60;
    pub const CHECK2_MASK: u64 = 0x7;
    pub const TIME_RESOLUTION_SHIFT: u32 = 44;
    pub const TIME_RESOLUTION_MASK: u64 = 0x3;
    /// PSD2: trigger_threshold. PHA2: reused for probe-type info (carried
    /// via [`super::WaveformMetadata`] from the variant). The raw 16 bits
    /// land in [`Waveform::trigger_threshold`] regardless — meaningful for
    /// PSD2, opaque for PHA2.
    pub const TRIGGER_THRESHOLD_SHIFT: u32 = 28;
    pub const TRIGGER_THRESHOLD_MASK: u64 = 0xFFFF;

    pub const SIZE_MASK: u64 = 0xFFF;
}

pub mod sample_bits {
    pub const ANALOG_PROBE_MASK: u32 = 0x3FFF;
    pub const ANALOG_PROBE2_SHIFT: u32 = 16;
    pub const DIGITAL_PROBE1_SHIFT: u32 = 14;
    pub const DIGITAL_PROBE2_SHIFT: u32 = 15;
    pub const DIGITAL_PROBE3_SHIFT: u32 = 30;
    pub const DIGITAL_PROBE4_SHIFT: u32 = 31;
}

pub mod signal {
    pub const TYPE_SHIFT: u32 = 60;
    pub const SUBTYPE_SHIFT: u32 = 56;
    pub const TYPE_MASK: u64 = 0xF;
    pub const START_TYPE: u64 = 0x3;
    pub const START_SUBTYPE: u64 = 0x0;
    pub const STOP_TYPE: u64 = 0x3;
    pub const STOP_SUBTYPE: u64 = 0x2;
}

// ---------------------------------------------------------------------------
// Shared structures
// ---------------------------------------------------------------------------

/// Configuration shared by Psd2Decoder + Pha2Decoder.
#[derive(Debug, Clone)]
pub struct Dig2Config {
    /// ADC time step in nanoseconds (typically 2.0 for 500 MS/s).
    pub time_step_ns: f64,
    /// Module ID stamped onto every emitted event.
    pub module_id: u8,
    /// Verbose dump of every aggregate (slow — testing only).
    pub dump_enabled: bool,
    /// Number of physical channels — events with `channel >= num_channels`
    /// are still emitted but logged once per aggregate as out-of-range.
    pub num_channels: u8,
}

impl Default for Dig2Config {
    fn default() -> Self {
        Self {
            time_step_ns: 2.0,
            module_id: 0,
            dump_enabled: false,
            num_channels: 32,
        }
    }
}

/// Per-probe metadata extracted from the waveform header. PSD2 returns
/// "unsigned + UNKNOWN probe types" verbatim; PHA2 parses the wf-header low
/// bits to fill in real values. PSD2/PHA2 only ship 2 analog + 4 digital
/// probes; the 3rd analog / 5th digital slots in the carrier `Waveform` are
/// reserved for AMax debug FW and remain `UNKNOWN_PROBE_TYPE` here.
#[derive(Debug, Clone, Copy)]
pub struct WaveformMetadata {
    pub analog_probe1_is_signed: bool,
    pub analog_probe2_is_signed: bool,
    pub analog_probe_type: [u8; 2],
    pub digital_probe_type: [u8; 4],
}

impl Default for WaveformMetadata {
    fn default() -> Self {
        Self {
            analog_probe1_is_signed: false,
            analog_probe2_is_signed: false,
            analog_probe_type: [UNKNOWN_PROBE_TYPE; 2],
            digital_probe_type: [UNKNOWN_PROBE_TYPE; 4],
        }
    }
}

impl WaveformMetadata {
    /// Promote the 2-analog / 4-digital metadata into the carrier `Waveform`
    /// shape (3 analog / 5 digital). The 3rd analog / 5th digital slot is
    /// padded with `UNKNOWN_PROBE_TYPE` since PSD2/PHA2 do not populate them.
    fn analog_probe_type_padded(&self) -> [u8; 3] {
        [
            self.analog_probe_type[0],
            self.analog_probe_type[1],
            UNKNOWN_PROBE_TYPE,
        ]
    }

    fn digital_probe_type_padded(&self) -> [u8; 16] {
        [
            self.digital_probe_type[0],
            self.digital_probe_type[1],
            self.digital_probe_type[2],
            self.digital_probe_type[3],
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
            UNKNOWN_PROBE_TYPE,
        ]
    }
}

// ---------------------------------------------------------------------------
// Per-firmware variant trait
// ---------------------------------------------------------------------------

/// Per-firmware customization for the shared VX27xx/VX274x DIG2 RAW decoder.
pub trait Dig2Variant {
    /// Used in log prefixes and metric tags (e.g. `"PSD2"` / `"PHA2"`).
    const FW_NAME: &'static str;

    /// Pull the `energy_short` value out of the per-event second word.
    /// PSD2 reads bits[41:26] (charge_short integral); PHA2 returns 0 (the
    /// slot is unused on the wire).
    fn decode_energy_short(second_word: u64) -> u16;

    /// Extract per-probe metadata from the waveform header. PSD2 returns
    /// [`WaveformMetadata::default`] (all unsigned, all UNKNOWN probe types).
    /// PHA2 parses the low 16 bits of the wf-header per CAEN doxygen
    /// `legacy/PHA2_Parameters/a00108.html` (DPP-PHA waveform extras).
    fn parse_waveform_metadata(wf_header: u64) -> WaveformMetadata;
}

// ---------------------------------------------------------------------------
// Generic decoder
// ---------------------------------------------------------------------------

/// Generic VX27xx/VX274x DIG2 RAW decoder, monomorphized per firmware via
/// [`Dig2Variant`].
#[derive(Debug, Clone)]
pub struct Dig2Decoder<V: Dig2Variant> {
    pub config: Dig2Config,
    last_aggregate_counter: u16,
    /// Track fine-TS clamps so we warn at most once per run. Effectively
    /// dead for PSD2 (the `0x3FF` mask precludes >= 1024) but cheap to
    /// keep for symmetry; PHA2's defensive clamp legacy is preserved.
    fine_ts_clamp_warned: bool,
    /// Dump the first malformed aggregate (one-shot per run). Helps
    /// reverse-engineer FW wedge states without flooding logs.
    fault_dumped: bool,
    _phantom: PhantomData<V>,
}

impl<V: Dig2Variant> Dig2Decoder<V> {
    pub fn new(config: Dig2Config) -> Self {
        Self {
            config,
            last_aggregate_counter: 0,
            fine_ts_clamp_warned: false,
            fault_dumped: false,
            _phantom: PhantomData,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(Dig2Config::default())
    }

    pub fn set_dump_enabled(&mut self, enabled: bool) {
        self.config.dump_enabled = enabled;
    }

    /// Reset run-level state. Called by the Reader on Start signal.
    pub fn reset_for_new_run(&mut self) {
        self.last_aggregate_counter = 0;
        self.fine_ts_clamp_warned = false;
        self.fault_dumped = false;
    }

    /// Classify the data type by structural inspection.
    pub fn classify(&self, raw: &RawData) -> DataType {
        if raw.size < MIN_DATA_SIZE {
            return DataType::Unknown;
        }
        if raw.size == STOP_SIGNAL_SIZE && self.is_stop_signal(&raw.data) {
            return DataType::Stop;
        }
        if raw.size == START_SIGNAL_SIZE && self.is_start_signal(&raw.data) {
            return DataType::Start;
        }
        DataType::Event
    }

    pub fn decode(&mut self, raw: &RawData) -> Vec<EventData> {
        let mut events = Vec::new();
        self.decode_into(raw, &mut events);
        events
    }

    /// Decode an aggregate into `events` (cleared first). Special events
    /// (Start / Stop / per-event flagged) are logged and dropped — they
    /// never enter the physics stream.
    pub fn decode_into(&mut self, raw: &RawData, events: &mut Vec<EventData>) {
        events.clear();

        if self.config.dump_enabled {
            self.dump_raw_data(raw);
        }

        match self.classify(raw) {
            DataType::Start => {
                info!(fw = V::FW_NAME, size = raw.size, "Start signal received");
                return;
            }
            DataType::Stop => {
                info!(fw = V::FW_NAME, size = raw.size, "Stop signal received");
                return;
            }
            DataType::Unknown => {
                warn!(
                    fw = V::FW_NAME,
                    size = raw.size,
                    "Unknown data type, dropping"
                );
                return;
            }
            DataType::Event => {}
        }

        let header = read_u64(&raw.data, 0);
        if !self.validate_header(header, raw.size) {
            return;
        }

        let total_size = (header & aggregate_header::TOTAL_SIZE_MASK) as usize;
        let total_words = raw.data.len() / WORD_SIZE;
        events.reserve(total_size / 2);
        let mut word_index = 1; // skip aggregate header
        let mut out_of_range_count = 0u32;

        while word_index < total_size {
            if let Some(event) = self.decode_event(&raw.data, &mut word_index) {
                if event.channel >= self.config.num_channels {
                    out_of_range_count += 1;
                    if self.config.dump_enabled && out_of_range_count <= 5 {
                        warn!(
                            fw = V::FW_NAME,
                            channel = event.channel,
                            num_channels = self.config.num_channels,
                            "channel out-of-range",
                        );
                    }
                }
                events.push(event);
            }
        }

        if word_index != total_size {
            warn!(
                fw = V::FW_NAME,
                word_index,
                total_size,
                total_words,
                "DECODE MISMATCH: words consumed != aggregate header size",
            );
        }

        if out_of_range_count > 0 {
            warn!(
                fw = V::FW_NAME,
                out_of_range = out_of_range_count,
                num_channels = self.config.num_channels,
                decoded = events.len(),
                "events with channel >= num_channels in aggregate",
            );

            // One-shot fault dump: capture the FIRST malformed aggregate so
            // we can reverse-engineer the actual wire format from real bytes.
            if !self.fault_dumped {
                self.fault_dumped = true;
                self.dump_fault_aggregate(raw, total_words, events.len());
            }
        }

        events.sort_by(|a, b| {
            a.timestamp_ns
                .partial_cmp(&b.timestamp_ns)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if self.config.dump_enabled {
            debug!(fw = V::FW_NAME, events = events.len(), "aggregate decoded");
        }
    }

    /// Decode a single event from the aggregate (multi-word or compressed
    /// single-word format). Returns `None` for special events (filtered) or
    /// fragmented tails (alignment recovery).
    fn decode_event(&mut self, data: &[u8], word_index: &mut usize) -> Option<EventData> {
        let total_words = data.len() / WORD_SIZE;
        if *word_index >= total_words {
            return None;
        }

        let first_word = read_u64(data, *word_index);
        *word_index += 1;

        let is_last_word = ((first_word >> event_word::LAST_WORD_SHIFT) & 0x1) != 0;
        let channel = ((first_word >> event_word::CHANNEL_SHIFT) & event_word::CHANNEL_MASK) as u8;

        if is_last_word {
            // Single-word compressed event (data-reduction mode).
            return Some(self.decode_single_word_event(first_word, channel));
        }

        let is_special_event =
            ((first_word >> event_word::SPECIAL_EVENT_SHIFT) & event_word::SPECIAL_EVENT_MASK) != 0;
        let raw_timestamp = first_word & event_word::TIMESTAMP_MASK;

        if *word_index >= total_words {
            return None;
        }

        let second_word = read_u64(data, *word_index);
        *word_index += 1;

        let has_waveform = ((second_word >> event_word::WAVEFORM_FLAG_SHIFT) & 0x1) != 0;
        let is_last = ((second_word >> event_word::LAST_WORD_SHIFT) & 0x1) != 0;

        if is_special_event {
            // Per-event "stat" / time-counter words. Drain extra words to
            // keep word alignment, then drop — these are not physics.
            if !is_last {
                while *word_index < total_words {
                    let extra_word = read_u64(data, *word_index);
                    *word_index += 1;
                    if ((extra_word >> event_word::LAST_WORD_SHIFT) & 0x1) != 0 {
                        break;
                    }
                }
            }
            debug!(fw = V::FW_NAME, channel, "special event filtered");
            return None;
        }

        let flags_low = (second_word >> event_word::FLAGS_LOW_PRIORITY_SHIFT)
            & event_word::FLAGS_LOW_PRIORITY_MASK;
        let flags_high = (second_word >> event_word::FLAGS_HIGH_PRIORITY_SHIFT)
            & event_word::FLAGS_HIGH_PRIORITY_MASK;
        let flags = ((flags_high << 12) | flags_low) as u32;

        let energy = (second_word & event_word::ENERGY_MASK) as u16;
        let energy_short = V::decode_energy_short(second_word);

        // Fine-TS defensive clamp. The mask already enforces [0, 1023]
        // (FINE_TIME_MASK=0x3FF), but warn-once if we ever observe the
        // boundary so a future FW quirk surfaces as a log line rather
        // than silent saturation.
        let raw_fine_ts =
            ((second_word >> event_word::FINE_TIME_SHIFT) & event_word::FINE_TIME_MASK) as u16;
        let fine_time = if raw_fine_ts >= 1024 {
            if !self.fine_ts_clamp_warned {
                warn!(
                    fw = V::FW_NAME,
                    raw_fine_ts, "fine_ts >= 1024 — clamping (one-shot)"
                );
                self.fine_ts_clamp_warned = true;
            }
            1023
        } else {
            raw_fine_ts
        };

        let coarse_time_ns = (raw_timestamp as f64) * self.config.time_step_ns;
        let fine_time_ns =
            (fine_time as f64 / event_word::FINE_TIME_SCALE) * self.config.time_step_ns;
        let timestamp_ns = coarse_time_ns + fine_time_ns;

        let waveform = if has_waveform {
            decode_waveform::<V>(data, word_index, self.config.time_step_ns)
        } else {
            None
        };

        if self.config.dump_enabled {
            debug!(
                fw = V::FW_NAME,
                channel,
                timestamp_ns,
                energy,
                energy_short,
                fine_time,
                flags = format!("0x{:05x}", flags),
                has_waveform,
                "event decoded"
            );
        }

        Some(EventData {
            timestamp_ns,
            module: self.config.module_id,
            channel,
            energy,
            energy_short,
            fine_time,
            flags,
            user_info: [0; 4],
            waveform,
        })
    }

    /// Single-word compressed event (data-reduction mode). No waveform,
    /// no extras2, no fine_ts; flag_high lives at bits[55:48].
    fn decode_single_word_event(&self, word: u64, channel: u8) -> EventData {
        let flags_high = ((word >> event_word::SINGLE_WORD_FLAG_HIGH_SHIFT)
            & event_word::FLAGS_HIGH_PRIORITY_MASK) as u32;
        let timestamp_reduced =
            (word >> event_word::FINE_TIME_SHIFT) & event_word::TIMESTAMP_REDUCED_MASK;
        let energy = (word & event_word::ENERGY_MASK) as u16;

        let timestamp_ns = (timestamp_reduced as f64) * self.config.time_step_ns;
        let flags = flags_high << 12;

        if self.config.dump_enabled {
            debug!(
                fw = V::FW_NAME,
                channel,
                timestamp_ns,
                energy,
                flags_high = format!("0x{:02x}", flags_high),
                "single-word event"
            );
        }

        EventData {
            timestamp_ns,
            module: self.config.module_id,
            channel,
            energy,
            energy_short: 0,
            fine_time: 0,
            flags,
            user_info: [0; 4],
            waveform: None,
        }
    }

    fn validate_header(&mut self, header: u64, data_size: usize) -> bool {
        let header_type = (header >> aggregate_header::TYPE_SHIFT) & aggregate_header::TYPE_MASK;
        if header_type != aggregate_header::TYPE_DATA {
            warn!(
                fw = V::FW_NAME,
                header_type = format!("0x{:x}", header_type),
                expected = format!("0x{:x}", aggregate_header::TYPE_DATA),
                "invalid aggregate header type"
            );
            return false;
        }

        let fail_check =
            (header >> aggregate_header::FAIL_CHECK_SHIFT) & aggregate_header::FAIL_CHECK_MASK;
        if fail_check != 0 {
            warn!(fw = V::FW_NAME, "board fail bit set in aggregate header");
        }

        let aggregate_counter =
            ((header >> aggregate_header::COUNTER_SHIFT) & aggregate_header::COUNTER_MASK) as u16;
        if aggregate_counter != 0
            && aggregate_counter != self.last_aggregate_counter.wrapping_add(1)
            && self.config.dump_enabled
        {
            debug!(
                fw = V::FW_NAME,
                last = self.last_aggregate_counter,
                current = aggregate_counter,
                "aggregate counter discontinuity"
            );
        }
        self.last_aggregate_counter = aggregate_counter;

        let total_size = (header & aggregate_header::TOTAL_SIZE_MASK) as usize;
        if total_size * WORD_SIZE != data_size {
            debug!(
                fw = V::FW_NAME,
                header_bytes = total_size * WORD_SIZE,
                actual_bytes = data_size,
                "aggregate size mismatch — using header value"
            );
        }

        true
    }

    fn is_start_signal(&self, data: &[u8]) -> bool {
        if data.len() < START_SIGNAL_SIZE {
            return false;
        }
        let w = read_u64(data, 0);
        let t = (w >> signal::TYPE_SHIFT) & signal::TYPE_MASK;
        let s = (w >> signal::SUBTYPE_SHIFT) & signal::TYPE_MASK;
        t == signal::START_TYPE && s == signal::START_SUBTYPE
    }

    fn is_stop_signal(&self, data: &[u8]) -> bool {
        if data.len() < STOP_SIGNAL_SIZE {
            return false;
        }
        let w = read_u64(data, 0);
        let t = (w >> signal::TYPE_SHIFT) & signal::TYPE_MASK;
        let s = (w >> signal::SUBTYPE_SHIFT) & signal::TYPE_MASK;
        t == signal::STOP_TYPE && s == signal::STOP_SUBTYPE
    }

    fn dump_raw_data(&self, raw: &RawData) {
        debug!(
            fw = V::FW_NAME,
            size = raw.size,
            n_events = raw.n_events,
            "aggregate raw dump",
        );
        let num_words = raw.size / WORD_SIZE;
        for i in 0..num_words.min(8) {
            let word = read_u64(&raw.data, i);
            debug!(
                fw = V::FW_NAME,
                word_index = i,
                word = format!("0x{:016x}", word),
                "word"
            );
        }
    }

    /// One-shot fault dump (head + several deep probes). Triggered by the
    /// first malformed aggregate per run; subsequent malformed aggregates
    /// log out-of-range counts only.
    fn dump_fault_aggregate(&self, raw: &RawData, total_words: usize, decoded: usize) {
        warn!(
            fw = V::FW_NAME,
            raw_size = raw.size,
            raw_n_events = raw.n_events,
            total_words,
            decoded,
            "FAULT: dumping malformed aggregate (head 256 + deep probes)",
        );
        for i in 0..total_words.min(256) {
            let w = read_u64(&raw.data, i);
            warn!(
                fw = V::FW_NAME,
                idx = i,
                word = format!("0x{:016x}", w),
                "FAULT word"
            );
        }
        let probe_starts: Vec<usize> = vec![
            400,
            612,
            820,
            1020,
            1024,
            2048,
            4096,
            8192,
            16384,
            total_words.saturating_sub(64),
        ];
        for start in probe_starts {
            if start + 32 <= total_words {
                warn!(fw = V::FW_NAME, range_start = start, "FAULT probing range");
                for i in start..(start + 32).min(total_words) {
                    let w = read_u64(&raw.data, i);
                    warn!(
                        fw = V::FW_NAME,
                        idx = i,
                        word = format!("0x{:016x}", w),
                        "FAULT word"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers (pure, easy to test)
// ---------------------------------------------------------------------------

/// Read a 64-bit big-endian word at the given word index. Returns 0 on
/// out-of-bounds — the caller-guarded framing checks should make this
/// unreachable in practice. (Both VX27xx and VX274x deliver RAW data in
/// big-endian order.)
#[inline]
pub fn read_u64(data: &[u8], word_index: usize) -> u64 {
    let offset = word_index * WORD_SIZE;
    data.get(offset..offset + WORD_SIZE)
        .and_then(|slice| slice.try_into().ok())
        .map(u64::from_be_bytes)
        .unwrap_or(0)
}

/// Decode the waveform extras (header + size + N data words). Sample
/// bit-packing is shared between PSD2 / PHA2; per-probe sign extension and
/// probe-type metadata come from `V::parse_waveform_metadata`.
fn decode_waveform<V: Dig2Variant>(
    data: &[u8],
    word_index: &mut usize,
    ns_per_sample: f64,
) -> Option<Waveform> {
    let total_words = data.len() / WORD_SIZE;
    if *word_index + 2 > total_words {
        return None;
    }

    let wf_header = read_u64(data, *word_index);
    let check1 = (wf_header >> waveform_header::CHECK1_SHIFT) & 0x1;
    let check2 = (wf_header >> waveform_header::CHECK2_SHIFT) & waveform_header::CHECK2_MASK;
    if check1 != 1 || check2 != 0 {
        warn!(
            fw = V::FW_NAME,
            word_index = *word_index,
            wf_header = format!("0x{:016x}", wf_header),
            check1,
            check2,
            "invalid waveform header — skipping waveform"
        );
        return None;
    }
    *word_index += 1;

    let time_resolution = ((wf_header >> waveform_header::TIME_RESOLUTION_SHIFT)
        & waveform_header::TIME_RESOLUTION_MASK) as u8;
    // PSD2: real trigger_threshold. PHA2: probe-type info — opaque here.
    let trigger_threshold = ((wf_header >> waveform_header::TRIGGER_THRESHOLD_SHIFT)
        & waveform_header::TRIGGER_THRESHOLD_MASK) as u16;

    let metadata = V::parse_waveform_metadata(wf_header);

    let size_word = read_u64(data, *word_index);
    *word_index += 1;

    let n_waveform_words = (size_word & waveform_header::SIZE_MASK) as usize;
    let n_samples = n_waveform_words * 2;

    if *word_index + n_waveform_words > total_words {
        warn!(
            fw = V::FW_NAME,
            need = n_waveform_words,
            have = total_words - *word_index,
            "truncated waveform — skipping"
        );
        *word_index = total_words.min(*word_index + n_waveform_words);
        return None;
    }

    let mut analog_probe1 = Vec::with_capacity(n_samples);
    let mut analog_probe2 = Vec::with_capacity(n_samples);
    let mut digital_probe1 = Vec::with_capacity(n_samples);
    let mut digital_probe2 = Vec::with_capacity(n_samples);
    let mut digital_probe3 = Vec::with_capacity(n_samples);
    let mut digital_probe4 = Vec::with_capacity(n_samples);

    let decode_ap = |raw: u32, signed: bool| -> i16 {
        if signed {
            sign_extend_14bit(raw)
        } else {
            (raw & sample_bits::ANALOG_PROBE_MASK) as i16
        }
    };

    // Trust wf_size from the FW. See decoder/mod.rs hot-path policy — bit-
    // pattern heuristics that "look like" a wf-header mid-stream are
    // forbidden because real sample words satisfy them routinely.
    for _ in 0..n_waveform_words {
        let word = read_u64(data, *word_index);
        *word_index += 1;

        for shift in [0u32, 32u32] {
            let sample = ((word >> shift) & 0xFFFF_FFFF) as u32;

            let ap1_raw = sample & sample_bits::ANALOG_PROBE_MASK;
            let ap2_raw =
                (sample >> sample_bits::ANALOG_PROBE2_SHIFT) & sample_bits::ANALOG_PROBE_MASK;
            let ap1 = decode_ap(ap1_raw, metadata.analog_probe1_is_signed);
            let ap2 = decode_ap(ap2_raw, metadata.analog_probe2_is_signed);
            let dp1 = ((sample >> sample_bits::DIGITAL_PROBE1_SHIFT) & 0x1) as u8;
            let dp2 = ((sample >> sample_bits::DIGITAL_PROBE2_SHIFT) & 0x1) as u8;
            let dp3 = ((sample >> sample_bits::DIGITAL_PROBE3_SHIFT) & 0x1) as u8;
            let dp4 = ((sample >> sample_bits::DIGITAL_PROBE4_SHIFT) & 0x1) as u8;

            analog_probe1.push(ap1);
            analog_probe2.push(ap2);
            digital_probe1.push(dp1);
            digital_probe2.push(dp2);
            digital_probe3.push(dp3);
            digital_probe4.push(dp4);
        }
    }

    Some(Waveform {
        analog_probe1,
        analog_probe2,
        analog_probe3: Vec::new(),
        digital_probe1,
        digital_probe2,
        digital_probe3,
        digital_probe4,
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
        time_resolution,
        trigger_threshold,
        ns_per_sample,
        analog_probe1_is_signed: metadata.analog_probe1_is_signed,
        analog_probe2_is_signed: metadata.analog_probe2_is_signed,
        analog_probe3_is_signed: false,
        analog_probe_type: metadata.analog_probe_type_padded(),
        digital_probe_type: metadata.digital_probe_type_padded(),
    })
}

// ---------------------------------------------------------------------------
// Tests — Mock variant exercises the framing in isolation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock variant: PSD2-flavoured (energy_short = bits[41:26],
    /// metadata = unsigned + UNKNOWN). Adequate for framing-only tests.
    struct MockVariant;

    impl Dig2Variant for MockVariant {
        const FW_NAME: &'static str = "MOCK";

        fn decode_energy_short(second_word: u64) -> u16 {
            const ENERGY_SHORT_SHIFT: u32 = 26;
            const ENERGY_SHORT_MASK: u64 = 0xFFFF;
            ((second_word >> ENERGY_SHORT_SHIFT) & ENERGY_SHORT_MASK) as u16
        }

        fn parse_waveform_metadata(_wf_header: u64) -> WaveformMetadata {
            WaveformMetadata::default()
        }
    }

    type MockDecoder = Dig2Decoder<MockVariant>;

    fn pack_be(words: &[u64]) -> Vec<u8> {
        let mut out = Vec::with_capacity(words.len() * 8);
        for &w in words {
            out.extend_from_slice(&w.to_be_bytes());
        }
        out
    }

    /// Build a minimal 1-event aggregate (header + first word + second word).
    fn build_aggregate(channel: u8, ts: u64, energy: u16, fine_ts: u16) -> Vec<u8> {
        let total_words: u64 = 3;
        let header = (0x2u64 << 60) | total_words;
        // event word 1: bit63=0, channel, special=0, ts in low 48 bits
        let evt_w1 = ((channel as u64) << 56) | (ts & 0xFFFF_FFFF_FFFF);
        // event word 2: bit63=1 (last), bit62=0 (no waveform), fine_ts + energy
        let evt_w2 = (1u64 << 63) | ((fine_ts as u64 & 0x3FF) << 16) | energy as u64;
        pack_be(&[header, evt_w1, evt_w2])
    }

    fn raw_data(bytes: Vec<u8>, n_events: u32) -> RawData {
        RawData {
            size: bytes.len(),
            data: bytes,
            n_events,
        }
    }

    // -----------------------------------------------------------------------
    // classify
    // -----------------------------------------------------------------------

    #[test]
    fn classify_unknown_for_tiny_data() {
        let dec = MockDecoder::with_defaults();
        let raw = raw_data(vec![0u8; 8], 0);
        assert_eq!(dec.classify(&raw), DataType::Unknown);
    }

    #[test]
    fn classify_minimum_size_is_event() {
        let dec = MockDecoder::with_defaults();
        let raw = raw_data(vec![0u8; 16], 0);
        assert_eq!(dec.classify(&raw), DataType::Event);
    }

    #[test]
    fn classify_stop_signal_three_words() {
        let dec = MockDecoder::with_defaults();
        let mut data = vec![0u8; STOP_SIGNAL_SIZE];
        data[0] = 0x32; // type=3 (high nibble), subtype=2 (low nibble)
        let raw = raw_data(data, 0);
        assert_eq!(dec.classify(&raw), DataType::Stop);
    }

    #[test]
    fn classify_start_signal_four_words() {
        let dec = MockDecoder::with_defaults();
        let mut data = vec![0u8; START_SIGNAL_SIZE];
        data[0] = 0x30; // type=3, subtype=0
        let raw = raw_data(data, 0);
        assert_eq!(dec.classify(&raw), DataType::Start);
    }

    #[test]
    fn classify_avoid_false_positives_on_event_size() {
        // 24-byte aggregate that's not actually a stop signal — should be
        // Event, not Stop, since byte 0 doesn't match the type/subtype.
        let dec = MockDecoder::with_defaults();
        let mut data = vec![0u8; STOP_SIGNAL_SIZE];
        data[0] = 0x20; // type=2 (DATA), not 0x3 (signal)
        let raw = raw_data(data, 0);
        assert_eq!(dec.classify(&raw), DataType::Event);
    }

    // -----------------------------------------------------------------------
    // decode — happy path
    // -----------------------------------------------------------------------

    #[test]
    fn decode_single_event_basic() {
        let mut dec = MockDecoder::with_defaults();
        let bytes = build_aggregate(
            /*ch*/ 5, /*ts*/ 1_000_000, /*energy*/ 4242, /*fts*/ 200,
        );
        let raw = raw_data(bytes, 1);
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.channel, 5);
        assert_eq!(e.energy, 4242);
        assert_eq!(e.fine_time, 200);
        // 1_000_000 * 2 ns + 200/1024 * 2 ns ≈ 2_000_000.39 ns
        assert!((e.timestamp_ns - 2_000_000.39).abs() < 1.0);
    }

    #[test]
    fn decode_zero_event_aggregate_header_only() {
        // CAEN sometimes sends keep-alive aggregates with header-only.
        let header = (0x2u64 << 60) | 1; // total_words = 1
        let bytes = pack_be(&[header]);
        let mut dec = MockDecoder::with_defaults();
        let raw = raw_data(bytes, 0);
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn decode_invalid_header_type_drops_aggregate() {
        let mut data = vec![0u8; 24];
        data[0] = 0x10; // type=1 (invalid; expected 0x2)
        let raw = raw_data(data, 0);
        let mut dec = MockDecoder::with_defaults();
        let events = dec.decode(&raw);
        assert!(events.is_empty());
    }

    #[test]
    fn decode_start_signal_returns_no_events() {
        let mut dec = MockDecoder::with_defaults();
        let mut data = vec![0u8; START_SIGNAL_SIZE];
        data[0] = 0x30;
        let raw = raw_data(data, 0);
        let events = dec.decode(&raw);
        assert!(events.is_empty());
    }

    #[test]
    fn decode_stop_signal_returns_no_events() {
        let mut dec = MockDecoder::with_defaults();
        let mut data = vec![0u8; STOP_SIGNAL_SIZE];
        data[0] = 0x32;
        let raw = raw_data(data, 0);
        let events = dec.decode(&raw);
        assert!(events.is_empty());
    }

    #[test]
    fn decode_emits_module_id_from_config() {
        let mut dec = MockDecoder::new(Dig2Config {
            time_step_ns: 2.0,
            module_id: 13,
            dump_enabled: false,
            num_channels: 32,
        });
        let bytes = build_aggregate(0, 1000, 100, 0);
        let raw = raw_data(bytes, 1);
        let events = dec.decode(&raw);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].module, 13);
    }

    // -----------------------------------------------------------------------
    // decode — special events filtered, single-word events accepted
    // -----------------------------------------------------------------------

    #[test]
    fn decode_special_event_is_filtered() {
        // Build event with bit55 set (special_event flag) — should be
        // dropped and second word consumed (bit63=1 = is_last so no extra
        // words to drain).
        let total_words: u64 = 3;
        let header = (0x2u64 << 60) | total_words;
        let evt_w1 = ((7u64) << 56) | (1u64 << 55) | 1000; // ch=7, special=1, ts=1000
        let evt_w2 = 1u64 << 63; // bit63=1 (last), no flags/energy
        let bytes = pack_be(&[header, evt_w1, evt_w2]);
        let raw = raw_data(bytes, 1);
        let mut dec = MockDecoder::with_defaults();
        let events = dec.decode(&raw);
        assert!(events.is_empty());
    }

    #[test]
    fn decode_single_word_compressed_event() {
        // Aggregate header + 1 single-word event (bit63=1 in first word).
        let total_words: u64 = 2;
        let header = (0x2u64 << 60) | total_words;
        // bit63=1, channel=4, flag_high=0, timestamp_reduced=500, energy=999
        let evt = (1u64 << 63) | (4u64 << 56) | (500u64 << 16) | 999;
        let bytes = pack_be(&[header, evt]);
        let raw = raw_data(bytes, 1);
        let mut dec = MockDecoder::with_defaults();
        let events = dec.decode(&raw);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.channel, 4);
        assert_eq!(e.energy, 999);
        assert_eq!(e.energy_short, 0); // single-word never carries energy_short
        assert_eq!(e.fine_time, 0);
        // 500 * 2 ns = 1000 ns
        assert!((e.timestamp_ns - 1000.0).abs() < 0.001);
    }

    // -----------------------------------------------------------------------
    // Multi-event ordering, reset, dump
    // -----------------------------------------------------------------------

    #[test]
    fn decode_multiple_events_sorted_by_timestamp() {
        // Two events: ts=5000 first in stream, ts=1000 second.
        let total_words: u64 = 5;
        let header = (0x2u64 << 60) | total_words;
        let later_w1: u64 = 5000;
        let later_w2 = (1u64 << 63) | 200; // energy=200
        let earlier_w1: u64 = 1000;
        let earlier_w2 = (1u64 << 63) | 100; // energy=100
        let bytes = pack_be(&[header, later_w1, later_w2, earlier_w1, earlier_w2]);
        let raw = raw_data(bytes, 2);
        let mut dec = MockDecoder::with_defaults();
        let events = dec.decode(&raw);
        assert_eq!(events.len(), 2);
        assert!(events[0].timestamp_ns < events[1].timestamp_ns);
        assert_eq!(events[0].energy, 100);
        assert_eq!(events[1].energy, 200);
    }

    #[test]
    fn reset_for_new_run_clears_diagnostic_state() {
        let mut dec = MockDecoder::with_defaults();
        // Force fault_dumped + counter advance via one decode.
        let bytes = build_aggregate(0, 1000, 100, 0);
        let raw = raw_data(bytes, 1);
        let _ = dec.decode(&raw);
        // last_aggregate_counter only advances if header counter != 0; the
        // helper sets counter=0 so we just exercise the path.
        dec.reset_for_new_run();
        assert!(!dec.fault_dumped);
        assert!(!dec.fine_ts_clamp_warned);
        assert_eq!(dec.last_aggregate_counter, 0);
    }

    // -----------------------------------------------------------------------
    // Free helpers
    // -----------------------------------------------------------------------

    #[test]
    fn read_u64_big_endian() {
        let data: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(read_u64(&data, 0), 0x0102_0304_0506_0708);
    }

    #[test]
    fn read_u64_returns_zero_on_out_of_bounds() {
        let data: Vec<u8> = vec![0x01];
        assert_eq!(read_u64(&data, 5), 0);
    }

    // -----------------------------------------------------------------------
    // Energy-short variant hook — verify the trait method is actually called
    // -----------------------------------------------------------------------

    /// Variant that always returns energy_short=0xBEEF so we can prove the
    /// generic decoder picks up the trait method's value (vs. having any
    /// hard-coded behavior).
    struct BeefVariant;
    impl Dig2Variant for BeefVariant {
        const FW_NAME: &'static str = "BEEF";
        fn decode_energy_short(_second_word: u64) -> u16 {
            0xBEEF
        }
        fn parse_waveform_metadata(_wf_header: u64) -> WaveformMetadata {
            WaveformMetadata::default()
        }
    }

    #[test]
    fn variant_decode_energy_short_drives_event_field() {
        let bytes = build_aggregate(0, 1000, 100, 0);
        let raw = raw_data(bytes, 1);
        let mut dec: Dig2Decoder<BeefVariant> = Dig2Decoder::with_defaults();
        let events = dec.decode(&raw);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].energy_short, 0xBEEF);
    }

    // -----------------------------------------------------------------------
    // Waveform metadata variant hook — verify is_signed flips sample interp
    // -----------------------------------------------------------------------

    /// Variant that forces analog_probe1_is_signed=true so we can prove the
    /// metadata trait drives sample sign-extension.
    struct SignedAp1Variant;
    impl Dig2Variant for SignedAp1Variant {
        const FW_NAME: &'static str = "SIGN";
        fn decode_energy_short(_second_word: u64) -> u16 {
            0
        }
        fn parse_waveform_metadata(_wf_header: u64) -> WaveformMetadata {
            WaveformMetadata {
                analog_probe1_is_signed: true,
                ..WaveformMetadata::default()
            }
        }
    }

    #[test]
    fn waveform_metadata_drives_sign_extension() {
        // Build an event with one waveform sample of 0x2000 (= -8192 if
        // sign-extended, 8192 if treated unsigned).
        let total_words: u64 = 5;
        let header = (0x2u64 << 60) | total_words;
        // event w1: ch=0, ts=0
        let evt_w1 = 0u64;
        // event w2: bit62=1 (has wf), bit63=0 (not last)
        let evt_w2 = 1u64 << 62;
        // wf header: check1=1 (bit63), check2=0 (bits[62:60] all zero)
        let wf_hdr = 1u64 << 63;
        // size word: 1 wf word = 2 samples
        let size_word = 1u64;
        // sample word: low half sample = 0x2000 (will sign-extend to -8192)
        let sample_word = 0x0000_2000_u64;
        let bytes = pack_be(&[header, evt_w1, evt_w2, wf_hdr, size_word, sample_word]);
        let raw = raw_data(bytes, 1);

        let mut dec: Dig2Decoder<SignedAp1Variant> = Dig2Decoder::with_defaults();
        let events = dec.decode(&raw);
        assert_eq!(events.len(), 1);
        let wf = events[0].waveform.as_ref().expect("waveform present");
        assert!(wf.analog_probe1_is_signed);
        // First sample's analog_probe1 raw was 0x2000 → -8192 with sign extension.
        assert_eq!(wf.analog_probe1[0], -8192);
    }
}

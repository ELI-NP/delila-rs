//! PHA2 Decoder for CAEN x274x series digitizers (DPP-PHA, trapezoidal-filter MCA).
//!
//! Decodes 64-bit big-endian word format from the DIG2 RAW endpoint when the
//! board is running DPP-PHA firmware. The aggregate-header / event-word
//! layout is the same Individual Trigger Mode (`format=0x2`) used by PSD2;
//! the only per-event difference is the absence of `energy_short` (PSD2 puts
//! a charge_short value in bits [41:26]; PHA2 leaves that slot unused, so we
//! force it to 0).
//!
//! Waveform extras header carries PHA-specific probe-type info (analog probe
//! type + sign + multiplication factor; digital probe type). Phase 2 keeps
//! the probe-type bytes parsed locally for diagnostics; surfacing them on
//! [`EventData`] is deferred to Phase 4 (cross-cutting wire-format change).
//!
//! Reference: `legacy/PHA2_Parameters/a00108.html` (CAEN FELib doxygen
//! "Supported Endpoints" → DPPPHA Raw / DPPPHA decoded sections) and
//! `docs/devtree_examples/vx2730_pha2_sn52622.json`.

use super::common::{DataType, EventData, RawData, Waveform};
use tracing::{debug, info, warn};

/// PHA2 wire constants — 64-bit big-endian words, same Individual Trigger
/// Mode aggregate envelope as PSD2.
mod constants {
    pub const WORD_SIZE: usize = 8;

    // Aggregate header (word 0)
    pub const HEADER_TYPE_SHIFT: u32 = 60;
    pub const HEADER_TYPE_MASK: u64 = 0xF;
    pub const HEADER_TYPE_DATA: u64 = 0x2;
    pub const HEADER_FAIL_CHECK_SHIFT: u32 = 56;
    pub const HEADER_FAIL_CHECK_MASK: u64 = 0x1;
    pub const AGGREGATE_COUNTER_SHIFT: u32 = 32;
    pub const AGGREGATE_COUNTER_MASK: u64 = 0xFFFF;
    pub const TOTAL_SIZE_MASK: u64 = 0xFFFFFFFF;

    // Event first word
    pub const LAST_WORD_SHIFT: u32 = 63;
    pub const CHANNEL_SHIFT: u32 = 56;
    pub const CHANNEL_MASK: u64 = 0x7F;
    pub const SPECIAL_EVENT_SHIFT: u32 = 55;
    pub const SPECIAL_EVENT_MASK: u64 = 0x1;
    pub const TIMESTAMP_MASK: u64 = 0xFFFFFFFFFFFF;
    pub const TIMESTAMP_REDUCED_MASK: u64 = 0xFFFFFFFF;

    // Event second word — identical layout to PSD2 with bits[41:26] unused.
    pub const WAVEFORM_FLAG_SHIFT: u32 = 62;
    pub const FLAGS_LOW_PRIORITY_SHIFT: u32 = 50;
    pub const FLAGS_LOW_PRIORITY_MASK: u64 = 0xFFF; // 12 bits
    pub const FLAGS_HIGH_PRIORITY_SHIFT: u32 = 42;
    pub const FLAGS_HIGH_PRIORITY_MASK: u64 = 0xFF; // 8 bits
    pub const FINE_TIME_SHIFT: u32 = 16;
    pub const FINE_TIME_MASK: u64 = 0x3FF;
    pub const FINE_TIME_SCALE: f64 = 1024.0;
    pub const ENERGY_MASK: u64 = 0xFFFF;

    // Single-word event flag-high lives at bits[55:48] (per PSD2 convention).
    pub const SINGLE_WORD_FLAG_HIGH_SHIFT: u32 = 48;

    // Waveform header (word 0 of waveform extras)
    pub const WAVEFORM_CHECK1_SHIFT: u32 = 63;
    pub const WAVEFORM_CHECK2_SHIFT: u32 = 60;
    pub const WAVEFORM_CHECK2_MASK: u64 = 0x7;
    pub const TIME_RESOLUTION_SHIFT: u32 = 44;
    pub const TIME_RESOLUTION_MASK: u64 = 0x3;
    // Trigger-threshold field in PSD2 lives at bits[43:28]. PHA2 uses this
    // slot for analog/digital probe info; we read but do not propagate.
    pub const TRIGGER_THRESHOLD_SHIFT: u32 = 28;
    pub const TRIGGER_THRESHOLD_MASK: u64 = 0xFFFF;

    // Waveform size word
    pub const WAVEFORM_WORDS_MASK: u64 = 0xFFF;

    // Sample format inside each 32-bit half-word (2 samples per 64-bit word)
    pub const ANALOG_PROBE_MASK: u32 = 0x3FFF;
    pub const ANALOG_PROBE2_SHIFT: u32 = 16;
    pub const DIGITAL_PROBE1_SHIFT: u32 = 14;
    pub const DIGITAL_PROBE2_SHIFT: u32 = 15;
    pub const DIGITAL_PROBE3_SHIFT: u32 = 30;
    pub const DIGITAL_PROBE4_SHIFT: u32 = 31;

    // Start/Stop signals (special aggregates)
    pub const SIGNAL_TYPE_SHIFT: u32 = 60;
    pub const SIGNAL_SUBTYPE_SHIFT: u32 = 56;
    pub const SIGNAL_TYPE_MASK: u64 = 0xF;
    pub const START_SIGNAL_TYPE: u64 = 0x3;
    pub const START_SIGNAL_SUBTYPE: u64 = 0x0;
    pub const STOP_SIGNAL_TYPE: u64 = 0x3;
    pub const STOP_SIGNAL_SUBTYPE: u64 = 0x2;

    pub const MIN_DATA_SIZE: usize = 2 * WORD_SIZE;
    pub const START_SIGNAL_SIZE: usize = 4 * WORD_SIZE;
    pub const STOP_SIGNAL_SIZE: usize = 3 * WORD_SIZE;
}

/// PHA2 Decoder configuration
#[derive(Debug, Clone)]
pub struct Pha2Config {
    /// ADC time step in nanoseconds (typically 2 ns for 500 MS/s).
    pub time_step_ns: f64,
    /// Module ID to stamp on every emitted event.
    pub module_id: u8,
    /// Verbose dump of every aggregate (slow — testing only).
    pub dump_enabled: bool,
    /// Number of physical channels; events with `channel >= num_channels`
    /// are still emitted but logged once per aggregate as "out of range".
    pub num_channels: u8,
}

impl Default for Pha2Config {
    fn default() -> Self {
        Self {
            time_step_ns: 2.0,
            module_id: 0,
            dump_enabled: false,
            num_channels: 32,
        }
    }
}

/// PHA2 Decoder for x274x series digitizers
#[derive(Debug, Clone)]
pub struct Pha2Decoder {
    config: Pha2Config,
    last_aggregate_counter: u16,
    /// Track fine-TS clamps so we warn at most once per run.
    fine_ts_clamp_warned: bool,
    /// Dump the first aggregate that triggers a wf-header check failure
    /// (for Phase 3 throughput debugging; one-shot per run).
    fault_dumped: bool,
}

impl Pha2Decoder {
    pub fn new(config: Pha2Config) -> Self {
        Self {
            config,
            last_aggregate_counter: 0,
            fine_ts_clamp_warned: false,
            fault_dumped: false,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(Pha2Config::default())
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
        if raw.size < constants::MIN_DATA_SIZE {
            return DataType::Unknown;
        }
        if raw.size == constants::STOP_SIGNAL_SIZE && self.is_stop_signal(&raw.data) {
            return DataType::Stop;
        }
        if raw.size == constants::START_SIGNAL_SIZE && self.is_start_signal(&raw.data) {
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
    /// (Start/Stop/per-event flagged) are logged and dropped — they never
    /// enter the physics stream (per Gemini review note).
    pub fn decode_into(&mut self, raw: &RawData, events: &mut Vec<EventData>) {
        events.clear();

        if self.config.dump_enabled {
            self.dump_raw_data(raw);
        }

        match self.classify(raw) {
            DataType::Start => {
                info!(size = raw.size, "[PHA2] Start signal received");
                return;
            }
            DataType::Stop => {
                info!(size = raw.size, "[PHA2] Stop signal received");
                return;
            }
            DataType::Unknown => {
                warn!(size = raw.size, "[PHA2] Unknown data type, dropping");
                return;
            }
            DataType::Event => {}
        }

        let header = self.read_u64(&raw.data, 0);
        if !self.validate_header(header, raw.size) {
            return;
        }

        let total_size = (header & constants::TOTAL_SIZE_MASK) as usize;
        let total_words = raw.data.len() / constants::WORD_SIZE;
        events.reserve(total_size / 2);
        let mut word_index = 1; // skip aggregate header
        let mut out_of_range_count = 0u32;

        while word_index < total_size {
            if let Some(event) = self.decode_event(&raw.data, &mut word_index) {
                if event.channel >= self.config.num_channels {
                    out_of_range_count += 1;
                    if self.config.dump_enabled && out_of_range_count <= 5 {
                        warn!(
                            channel = event.channel,
                            num_channels = self.config.num_channels,
                            "[PHA2] channel out-of-range",
                        );
                    }
                }
                events.push(event);
            }
        }

        if word_index != total_size {
            warn!(
                word_index,
                total_size,
                total_words,
                "[PHA2] DECODE MISMATCH: words consumed != aggregate header size",
            );
        }

        if out_of_range_count > 0 {
            warn!(
                out_of_range = out_of_range_count,
                num_channels = self.config.num_channels,
                decoded = events.len(),
                "[PHA2] events with channel >= num_channels in aggregate",
            );

            // One-shot dump: capture the FIRST malformed aggregate so we can
            // reverse-engineer the actual wire format from real bytes.
            if !self.fault_dumped {
                self.fault_dumped = true;
                let num_words = raw.data.len() / constants::WORD_SIZE;
                warn!(
                    raw_size = raw.size,
                    raw_n_events = raw.n_events,
                    total_words = num_words,
                    decoded = events.len(),
                    "[PHA2-FAULT] dumping malformed aggregate (head 256 + sample around evt boundaries)",
                );
                for i in 0..num_words.min(256) {
                    let w = self.read_u64(&raw.data, i);
                    warn!(idx = i, word = format!("0x{:016x}", w), "[PHA2-FAULT] word");
                }
                // Probe deep into the aggregate: events appear to switch format mid-aggregate
                // when the FW enters the bad state. Look at multiple positions including
                // 1000, 2000, 5000, 10000, near-end.
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
                    num_words.saturating_sub(64),
                ];
                for start in probe_starts {
                    if start + 32 <= num_words {
                        warn!(range_start = start, "[PHA2-FAULT] probing range");
                        for i in start..(start + 32).min(num_words) {
                            let w = self.read_u64(&raw.data, i);
                            warn!(idx = i, word = format!("0x{:016x}", w), "[PHA2-FAULT] word");
                        }
                    }
                }
            }
        }

        events.sort_by(|a, b| {
            a.timestamp_ns
                .partial_cmp(&b.timestamp_ns)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if self.config.dump_enabled {
            debug!(events = events.len(), "[PHA2] aggregate decoded");
        }
    }

    fn decode_event(&mut self, data: &[u8], word_index: &mut usize) -> Option<EventData> {
        let total_words = data.len() / constants::WORD_SIZE;

        if *word_index >= total_words {
            return None;
        }

        let first_word = self.read_u64(data, *word_index);
        *word_index += 1;

        let is_last_word = ((first_word >> constants::LAST_WORD_SHIFT) & 0x1) != 0;
        let channel = ((first_word >> constants::CHANNEL_SHIFT) & constants::CHANNEL_MASK) as u8;

        if is_last_word {
            // Single-word compressed event (data-reduction mode).
            return self.decode_single_word_event(first_word, channel);
        }

        let is_special_event =
            ((first_word >> constants::SPECIAL_EVENT_SHIFT) & constants::SPECIAL_EVENT_MASK) != 0;
        let raw_timestamp = first_word & constants::TIMESTAMP_MASK;

        if *word_index >= total_words {
            return None;
        }

        let second_word = self.read_u64(data, *word_index);
        *word_index += 1;

        let has_waveform = ((second_word >> constants::WAVEFORM_FLAG_SHIFT) & 0x1) != 0;
        let is_last = ((second_word >> constants::LAST_WORD_SHIFT) & 0x1) != 0;

        if is_special_event {
            // Per-event "stat" / time-counter words. Drain extra words to
            // keep word alignment, then drop — these are not physics.
            if !is_last {
                while *word_index < total_words {
                    let extra_word = self.read_u64(data, *word_index);
                    *word_index += 1;
                    if ((extra_word >> constants::LAST_WORD_SHIFT) & 0x1) != 0 {
                        break;
                    }
                }
            }
            debug!(channel, "[PHA2] special event filtered");
            return None;
        }

        let flags_low = (second_word >> constants::FLAGS_LOW_PRIORITY_SHIFT)
            & constants::FLAGS_LOW_PRIORITY_MASK;
        let flags_high = (second_word >> constants::FLAGS_HIGH_PRIORITY_SHIFT)
            & constants::FLAGS_HIGH_PRIORITY_MASK;
        let flags = ((flags_high << 12) | flags_low) as u32;

        let energy = (second_word & constants::ENERGY_MASK) as u16;

        // Fine-TS defensive parsing: spec says 10 bits, [0, 1023]. The mask
        // already enforces this, but warn once if we ever observe it at the
        // boundary so Phase 3 throughput tests can flag firmware quirks.
        let raw_fine_ts =
            ((second_word >> constants::FINE_TIME_SHIFT) & constants::FINE_TIME_MASK) as u16;
        let fine_time = if raw_fine_ts >= 1024 {
            if !self.fine_ts_clamp_warned {
                warn!(raw_fine_ts, "[PHA2] fine_ts >= 1024 — clamping (one-shot)");
                self.fine_ts_clamp_warned = true;
            }
            1023
        } else {
            raw_fine_ts
        };

        let coarse_time_ns = (raw_timestamp as f64) * self.config.time_step_ns;
        let fine_time_ns =
            (fine_time as f64 / constants::FINE_TIME_SCALE) * self.config.time_step_ns;
        let timestamp_ns = coarse_time_ns + fine_time_ns;

        let waveform = if has_waveform {
            self.decode_waveform(data, word_index)
        } else {
            None
        };

        Some(EventData {
            timestamp_ns,
            module: self.config.module_id,
            channel,
            energy,
            // PHA2 leaves PSD2's energy_short slot (bits[41:26]) unused.
            energy_short: 0,
            fine_time,
            flags,
            user_info: [0; 4],
            waveform,
        })
    }

    /// Single-word compressed event — same layout as PSD2 (no PHA2 spec
    /// difference observed). No waveform, no extras2, no fine_ts.
    fn decode_single_word_event(&self, word: u64, channel: u8) -> Option<EventData> {
        let flags_high = ((word >> constants::SINGLE_WORD_FLAG_HIGH_SHIFT)
            & constants::FLAGS_HIGH_PRIORITY_MASK) as u32;
        let timestamp_reduced =
            (word >> constants::FINE_TIME_SHIFT) & constants::TIMESTAMP_REDUCED_MASK;
        let energy = (word & constants::ENERGY_MASK) as u16;

        let timestamp_ns = (timestamp_reduced as f64) * self.config.time_step_ns;
        let flags = flags_high << 12;

        Some(EventData {
            timestamp_ns,
            module: self.config.module_id,
            channel,
            energy,
            energy_short: 0,
            fine_time: 0,
            flags,
            user_info: [0; 4],
            waveform: None,
        })
    }

    /// Decode waveform extras (header word + size word + N data words).
    /// Sample bit-packing matches PSD2; the extras header carries PHA2-
    /// specific probe-type info that we read but don't yet propagate (see
    /// Phase 4 in TODO/51 for the cross-cutting `EventData` extension).
    fn decode_waveform(&self, data: &[u8], word_index: &mut usize) -> Option<Waveform> {
        let total_words = data.len() / constants::WORD_SIZE;
        if *word_index + 2 > total_words {
            return None;
        }

        let wf_header = self.read_u64(data, *word_index);
        let check1 = (wf_header >> constants::WAVEFORM_CHECK1_SHIFT) & 0x1;
        let check2 =
            (wf_header >> constants::WAVEFORM_CHECK2_SHIFT) & constants::WAVEFORM_CHECK2_MASK;
        if check1 != 1 || check2 != 0 {
            warn!(
                word_index = *word_index,
                wf_header = format!("0x{:016x}", wf_header),
                check1,
                check2,
                "[PHA2] invalid waveform header — skipping waveform"
            );
            return None;
        }
        *word_index += 1;

        let time_resolution = ((wf_header >> constants::TIME_RESOLUTION_SHIFT)
            & constants::TIME_RESOLUTION_MASK) as u8;
        // PHA2 puts probe-type info where PSD2 carries trigger_threshold.
        // We expose `trigger_threshold` raw for now — not strictly correct
        // for PHA2, but harmless until Phase 4 exposes typed probe info.
        let trigger_threshold = ((wf_header >> constants::TRIGGER_THRESHOLD_SHIFT)
            & constants::TRIGGER_THRESHOLD_MASK) as u16;

        let size_word = self.read_u64(data, *word_index);
        *word_index += 1;

        let n_waveform_words = (size_word & constants::WAVEFORM_WORDS_MASK) as usize;
        let n_samples = n_waveform_words * 2;

        if *word_index + n_waveform_words > total_words {
            warn!(
                need = n_waveform_words,
                have = total_words - *word_index,
                "[PHA2] truncated waveform — skipping"
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

        // Trust wf_size from the FW. The previous "mid-loop wf_header
        // pattern" rewind was a misdiagnosis: sample words have bit63=1
        // whenever digital_probe_4 is asserted (e.g. EnergyFilterPeaking
        // around the trigger), and bits[62:60]=0 whenever DP3=0 and
        // analog_probe_2 is small (baseline) — both routinely satisfied
        // during normal acquisition. The "truncation" we observed in
        // Phase 3 stress tests was the FW genuinely wedging into a low-
        // rate state under rapid Configure cycles (waveforms came back
        // healthy after a few minutes idle); the FW does not partial-write
        // a waveform mid-event. Verified 2026-05-04 with `pha2_simple_test`
        // (`--wave-downsampling 1` and `8`): wf_size=2048 in both cases,
        // event-to-event spacing is exactly 2052 words, no actual
        // truncation observed.
        for _ in 0..n_waveform_words {
            let word = self.read_u64(data, *word_index);
            *word_index += 1;

            for shift in [0u32, 32u32] {
                let sample = ((word >> shift) & 0xFFFFFFFF) as u32;

                let ap1 = (sample & constants::ANALOG_PROBE_MASK) as i16;
                let ap2 = ((sample >> constants::ANALOG_PROBE2_SHIFT)
                    & constants::ANALOG_PROBE_MASK) as i16;
                let dp1 = ((sample >> constants::DIGITAL_PROBE1_SHIFT) & 0x1) as u8;
                let dp2 = ((sample >> constants::DIGITAL_PROBE2_SHIFT) & 0x1) as u8;
                let dp3 = ((sample >> constants::DIGITAL_PROBE3_SHIFT) & 0x1) as u8;
                let dp4 = ((sample >> constants::DIGITAL_PROBE4_SHIFT) & 0x1) as u8;

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
            digital_probe1,
            digital_probe2,
            digital_probe3,
            digital_probe4,
            time_resolution,
            trigger_threshold,
            ns_per_sample: self.config.time_step_ns,
            // PHA2 trapezoid / energy-filter probes carry signed 14-bit
            // values per CAEN spec ("is_signed" flag in the extras header).
            // Phase 2 default = false (raw 14-bit unsigned, [0, 16383]).
            // Phase 4 will read the per-probe is_signed flag from the
            // waveform extras header and set this correctly.
            analog_probe1_is_signed: false,
            analog_probe2_is_signed: false,
        })
    }

    fn validate_header(&mut self, header: u64, data_size: usize) -> bool {
        let header_type = (header >> constants::HEADER_TYPE_SHIFT) & constants::HEADER_TYPE_MASK;
        if header_type != constants::HEADER_TYPE_DATA {
            warn!(
                header_type = format!("0x{:x}", header_type),
                expected = format!("0x{:x}", constants::HEADER_TYPE_DATA),
                "[PHA2] invalid aggregate header type"
            );
            return false;
        }

        let fail_check =
            (header >> constants::HEADER_FAIL_CHECK_SHIFT) & constants::HEADER_FAIL_CHECK_MASK;
        if fail_check != 0 {
            warn!("[PHA2] board fail bit set in aggregate header");
        }

        let aggregate_counter = ((header >> constants::AGGREGATE_COUNTER_SHIFT)
            & constants::AGGREGATE_COUNTER_MASK) as u16;
        if aggregate_counter != 0
            && aggregate_counter != self.last_aggregate_counter.wrapping_add(1)
            && self.config.dump_enabled
        {
            debug!(
                last = self.last_aggregate_counter,
                current = aggregate_counter,
                "[PHA2] aggregate counter discontinuity"
            );
        }
        self.last_aggregate_counter = aggregate_counter;

        let total_size = (header & constants::TOTAL_SIZE_MASK) as usize;
        if total_size * constants::WORD_SIZE != data_size {
            debug!(
                header_bytes = total_size * constants::WORD_SIZE,
                actual_bytes = data_size,
                "[PHA2] aggregate size mismatch — using header value"
            );
        }

        true
    }

    fn is_start_signal(&self, data: &[u8]) -> bool {
        if data.len() < constants::START_SIGNAL_SIZE {
            return false;
        }
        let w = self.read_u64(data, 0);
        let t = (w >> constants::SIGNAL_TYPE_SHIFT) & constants::SIGNAL_TYPE_MASK;
        let s = (w >> constants::SIGNAL_SUBTYPE_SHIFT) & constants::SIGNAL_TYPE_MASK;
        t == constants::START_SIGNAL_TYPE && s == constants::START_SIGNAL_SUBTYPE
    }

    fn is_stop_signal(&self, data: &[u8]) -> bool {
        if data.len() < constants::STOP_SIGNAL_SIZE {
            return false;
        }
        let w = self.read_u64(data, 0);
        let t = (w >> constants::SIGNAL_TYPE_SHIFT) & constants::SIGNAL_TYPE_MASK;
        let s = (w >> constants::SIGNAL_SUBTYPE_SHIFT) & constants::SIGNAL_TYPE_MASK;
        t == constants::STOP_SIGNAL_TYPE && s == constants::STOP_SIGNAL_SUBTYPE
    }

    fn read_u64(&self, data: &[u8], word_index: usize) -> u64 {
        let offset = word_index * constants::WORD_SIZE;
        u64::from_be_bytes(
            data[offset..offset + constants::WORD_SIZE]
                .try_into()
                .unwrap_or([0; 8]),
        )
    }

    fn dump_raw_data(&self, raw: &RawData) {
        debug!(
            size = raw.size,
            n_events = raw.n_events,
            "[PHA2] aggregate raw dump",
        );
        let num_words = raw.size / constants::WORD_SIZE;
        for i in 0..num_words.min(8) {
            let word = self.read_u64(&raw.data, i);
            debug!(
                word_index = i,
                word = format!("0x{:016x}", word),
                "[PHA2] word"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a synthetic 64-bit BE word stream from a slice of u64.
    fn pack_be(words: &[u64]) -> Vec<u8> {
        let mut out = Vec::with_capacity(words.len() * 8);
        for &w in words {
            out.extend_from_slice(&w.to_be_bytes());
        }
        out
    }

    /// Build a minimal 1-event aggregate (header + event word 1 + event word 2).
    /// Returns (bytes, total_words).
    fn build_aggregate(channel: u8, ts: u64, energy: u16, fine_ts: u16) -> (Vec<u8>, u32) {
        // word 0: aggregate header. type=0x2, total_words=3
        let total_words: u64 = 3;
        let header = (0x2u64 << 60) | total_words;
        // word 1: event word 1. bit63=0, channel, special=0, ts in low 48 bits
        let evt_w1 = ((channel as u64) << 56) | (ts & 0xFFFFFFFFFFFF);
        // word 2: event word 2. bit63=1 (last header), bit62=0 (no wave), energy + fine_ts
        let evt_w2 = (1u64 << 63) | ((fine_ts as u64 & 0x3FF) << 16) | energy as u64;
        let bytes = pack_be(&[header, evt_w1, evt_w2]);
        (bytes, total_words as u32)
    }

    #[test]
    fn classify_unknown_for_tiny_data() {
        let dec = Pha2Decoder::with_defaults();
        let raw = RawData {
            data: vec![0u8; 8],
            size: 8,
            n_events: 0,
        };
        assert_eq!(dec.classify(&raw), DataType::Unknown);
    }

    #[test]
    fn classify_start_signal_real_bytes() {
        // Captured from 172.18.4.56 PHA2 Start signal.
        let bytes: Vec<u8> = vec![
            0x30, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let dec = Pha2Decoder::with_defaults();
        let raw = RawData {
            size: bytes.len(),
            data: bytes,
            n_events: 1,
        };
        assert_eq!(dec.classify(&raw), DataType::Start);
    }

    #[test]
    fn decode_single_event_basic() {
        let mut dec = Pha2Decoder::with_defaults();
        let (bytes, _) = build_aggregate(
            /*ch*/ 5, /*ts*/ 1_000_000, /*energy*/ 4242, /*fts*/ 200,
        );
        let raw = RawData {
            size: bytes.len(),
            data: bytes,
            n_events: 1,
        };
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.channel, 5);
        assert_eq!(e.energy, 4242);
        assert_eq!(e.energy_short, 0); // PHA2 leaves this unused
        assert_eq!(e.fine_time, 200);
        // 1_000_000 * 2 ns + 200/1024 * 2 ns ≈ 2_000_000.39 ns
        assert!((e.timestamp_ns - 2_000_000.39).abs() < 1.0);
    }

    #[test]
    fn decode_zero_event_aggregate_header_only() {
        // CAEN sometimes sends keep-alive aggregates with header-only
        // (total_words=1). Decoder must not panic and emits 0 events.
        let header = (0x2u64 << 60) | 1; // total_words = 1
        let bytes = pack_be(&[header]);
        let mut dec = Pha2Decoder::with_defaults();
        let raw = RawData {
            size: bytes.len(),
            data: bytes,
            n_events: 0,
        };
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn pile_up_flag_propagated_in_flags_field() {
        // FLAGS_LOW_PRIORITY bit 0 = pile-up per CAEN convention.
        // Pack flags_low = 0x001 (pile-up only).
        let mut dec = Pha2Decoder::with_defaults();
        let total_words: u64 = 3;
        let header = (0x2u64 << 60) | total_words;
        let evt_w1 = (3u64 << 56) | 100;
        let flags_low = 0x001u64;
        let evt_w2 = (1u64 << 63) | (flags_low << 50) | 0xCAFE;
        let bytes = pack_be(&[header, evt_w1, evt_w2]);
        let raw = RawData {
            size: bytes.len(),
            data: bytes,
            n_events: 1,
        };
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].flags & 0xFFF, 0x001);
        assert_eq!(events[0].energy, 0xCAFE);
    }

    #[test]
    fn fine_ts_at_max_does_not_clamp() {
        // 10-bit max = 1023 → no clamp needed (mask already enforces).
        let mut dec = Pha2Decoder::with_defaults();
        let (bytes, _) = build_aggregate(0, 100, 1, 1023);
        let raw = RawData {
            size: bytes.len(),
            data: bytes,
            n_events: 1,
        };
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fine_time, 1023);
    }

    #[test]
    fn ts_rollover_synthetic_does_not_panic() {
        // 48-bit TS rollover boundary: emit one event near TS=0xFFFF_FFFF_FFFE
        // and another at TS=0x0000_0000_0001 in the next aggregate, ensuring
        // the decoder doesn't choke on backward-going coarse TS within the
        // same call. (ReorderBuffer downstream handles cross-aggregate
        // rollover via run-level state, not the decoder.)
        let mut dec = Pha2Decoder::with_defaults();
        let (b1, _) = build_aggregate(0, 0xFFFF_FFFF_FFFE, 100, 0);
        let raw1 = RawData {
            size: b1.len(),
            data: b1,
            n_events: 1,
        };
        let mut events = Vec::new();
        dec.decode_into(&raw1, &mut events);
        assert_eq!(events.len(), 1);
        let (b2, _) = build_aggregate(0, 1, 200, 0);
        let raw2 = RawData {
            size: b2.len(),
            data: b2,
            n_events: 1,
        };
        dec.decode_into(&raw2, &mut events);
        assert_eq!(events.len(), 1); // decode_into clears events
    }

    #[test]
    fn special_event_filtered_out() {
        // bit 55 of word 1 set → filter out without emitting an event.
        let mut dec = Pha2Decoder::with_defaults();
        let total_words: u64 = 3;
        let header = (0x2u64 << 60) | total_words;
        let evt_w1 = (1u64 << 55) | 100; // special bit set, no channel
        let evt_w2 = 1u64 << 63; // last header, no waveform
        let bytes = pack_be(&[header, evt_w1, evt_w2]);
        let raw = RawData {
            size: bytes.len(),
            data: bytes,
            n_events: 1,
        };
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn reset_for_new_run_clears_state() {
        let mut dec = Pha2Decoder::with_defaults();
        dec.last_aggregate_counter = 42;
        dec.fine_ts_clamp_warned = true;
        dec.reset_for_new_run();
        assert_eq!(dec.last_aggregate_counter, 0);
        assert!(!dec.fine_ts_clamp_warned);
    }

    #[test]
    fn dp4_set_in_sample_does_not_truncate_waveform() {
        // Regression: in 2026-05-04 we briefly added a "mid-loop truncation
        // detector" that flagged any sample word with bit63=1 ∧ bits[62:60]=0
        // as the next event's wf_header. That misfired catastrophically
        // because PHA2 sample words have bit63 = digital_probe_4 of the upper
        // 32-bit half — every event with EnergyFilterPeaking (a default DP)
        // has DP4 transiently set, and AP2 is small near baseline so the
        // bits[62:60]=0 condition is also met. Live capture via
        // `pha2_simple_test --wave-downsampling 8` showed `wf_size = 0x800`
        // and event-to-event spacing of exactly 2052 words on the wire —
        // i.e. NO firmware truncation. The decoder must trust wf_size and
        // deliver the full sample buffer even when sample bytes mimic the
        // wf_header bit pattern.
        //
        // Synthetic aggregate:
        //   word 0: agg header
        //   word 1: ev1 first_word
        //   word 2: ev1 second_word (bit62 = waveform present)
        //   word 3: ev1 wf_header
        //   word 4: ev1 wf_size = 4
        //   words 5..8: 4 sample words; word 6 has bit63=1 ∧ bits[62:60]=0
        //               (mimics a real "DP4 fluke" sample)
        let total_words: u64 = 9;
        let agg_header = (0x2u64 << 60) | total_words;
        let ev1_w1 = (0u64 << 56) | 10;
        let ev1_w2 = (1u64 << constants::WAVEFORM_FLAG_SHIFT) | (1u64 << 63);
        let wf_hdr_const: u64 = 1u64 << 63; // check1=1, check2=0
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
        let raw = RawData {
            size: bytes.len(),
            data: bytes,
            n_events: 1,
        };
        let mut events = Vec::new();
        dec.decode_into(&raw, &mut events);

        assert_eq!(events.len(), 1, "decoder must NOT split on DP4-fluke samples");
        let wf = events[0]
            .waveform
            .as_ref()
            .expect("waveform must be present");
        // 4 sample words × 2 samples/word = 8 samples — the full configured length.
        assert_eq!(
            wf.analog_probe1.len(),
            8,
            "all 4 sample words (8 samples) must be delivered"
        );
        // The DP4 fluke must end up as a normal sample, with DP4=1 in its slot.
        // Iteration order: low half first, then high half. The fluke is
        // 0x80f71fd6_00f71fdb, so high-half upper sample is 0x80f71fd6 → DP4=1.
        // That's index 1*2 + 1 = 3 (event 1's word index 1, upper half).
        assert_eq!(
            wf.digital_probe4[3], 1,
            "DP4 fluke bit must be preserved in the sample buffer"
        );
    }
}

//! Common framing for DPP-PSD1 / DPP-PHA1 (DT5730 / VX1730 family) decoders.
//!
//! PSD1 and PHA1 share the entire CAEN aggregate framing (board aggregate +
//! dual-channel block + per-event time/extras/physics/waveform layout). The
//! only per-firmware divergence sits in 5 places:
//!
//! | What                         | PSD1                     | PHA1                       |
//! |------------------------------|--------------------------|----------------------------|
//! | `DUAL_CHANNEL_SIZE_MASK`     | 22-bit (`0x3F_FFFF`)     | 31-bit (`0x7FFF_FFFF`)     |
//! | `calculate_sw_fine_fraction` | 8192-baseline unsigned   | signed (zero-centered)     |
//! | `decode_physics_word`        | charge_long+charge_short | energy+extra_data          |
//! | Waveform analog samples      | 14-bit unsigned mask     | 14-bit sign-extended       |
//! | Waveform DP / probe mapping  | DP1@14, DP2@15           | DP@14, Tn@15 (Tn → DP1)    |
//!
//! Channel-header bits 27/28/29/30/31 are at identical positions in both
//! firmwares — only the names differ (PSD1 calls bit 28 `EE`, PHA1 calls it
//! `E2`; PSD1 calls bit 30 `EQ`, PHA1 calls it `EE`). The unified
//! [`Dig1ChannelHeader`] uses neutral names: `extras_enabled` (bit 28) and
//! `physics_enabled` (bit 30).
//!
//! The generic [`Dig1Decoder<V>`] zero-cost-monomorphizes over a variant via
//! the [`Dig1Variant`] trait. Hot-path overhead is the same as the original
//! hand-written PSD1 / PHA1 decoders — both `cargo --release` builds inline
//! the trait calls into the framing loop.
//!
//! # Hot-path policy
//!
//! See `decoder/mod.rs` "Hot-path heuristic policy" — pattern-based "looks
//! like a header" checks during sample decoding are forbidden without a CAEN
//! spec citation + `caen_simple_test` validation. The framing layer here
//! trusts `block_size` / `num_samples_wave` from the per-FW header and never
//! second-guesses.

use std::marker::PhantomData;

use super::common::{DataType, EventData, RawData, Waveform};
use super::rollover::RolloverTracker;

// ---------------------------------------------------------------------------
// Shared constants (PSD1 + PHA1 identical)
// ---------------------------------------------------------------------------

pub const WORD_SIZE: usize = 4;

pub mod board_header_bits {
    pub const HEADER_SIZE_WORDS: usize = 4;
    pub const HEADER_SIZE_BYTES: usize = HEADER_SIZE_WORDS * super::WORD_SIZE;

    pub const TYPE_SHIFT: u32 = 28;
    pub const TYPE_MASK: u32 = 0xF;
    pub const TYPE_DATA: u32 = 0xA;
    pub const AGGREGATE_SIZE_MASK: u32 = 0x0FFF_FFFF;

    pub const BOARD_ID_SHIFT: u32 = 27;
    pub const BOARD_ID_MASK: u32 = 0x1F;
    pub const BOARD_FAIL_SHIFT: u32 = 26;
    pub const DUAL_CHANNEL_MASK: u32 = 0xFF;

    pub const COUNTER_MASK: u32 = 0x7F_FFFF;
}

pub mod channel_header_bits {
    pub const HEADER_SIZE_WORDS: usize = 2;

    pub const NUM_SAMPLES_MASK: u32 = 0xFFFF;
    pub const EXTRA_OPTION_SHIFT: u32 = 24;
    pub const EXTRA_OPTION_MASK: u32 = 0x7;
    pub const SAMPLES_ENABLED_SHIFT: u32 = 27;
    /// PSD1 calls this `EE` (extras enabled). PHA1 calls it `E2` (extras2
    /// enabled). Same bit position, same meaning at the framing level.
    pub const EXTRAS_ENABLED_SHIFT: u32 = 28;
    pub const TIME_ENABLED_SHIFT: u32 = 29;
    /// PSD1 calls this `EQ` (charge enabled). PHA1 calls it `EE` (energy
    /// enabled). Same bit position, same role: presence of the per-event
    /// physics word that ends each event.
    pub const PHYSICS_ENABLED_SHIFT: u32 = 30;
    pub const DUAL_TRACE_SHIFT: u32 = 31;
}

pub mod event_bits {
    pub const TRIGGER_TIME_MASK: u32 = 0x7FFF_FFFF;
    pub const CHANNEL_FLAG_SHIFT: u32 = 31;

    // EXTRAS option 0b010
    pub const FINE_TIME_MASK: u32 = 0x3FF;
    pub const FLAGS_SHIFT: u32 = 10;
    pub const FLAGS_MASK: u32 = 0x3F;
    pub const EXTENDED_TIME_SHIFT: u32 = 16;
    pub const EXTENDED_TIME_MASK: u32 = 0xFFFF;

    pub const FINE_TIME_SCALE: f64 = 1024.0;

    /// Per-event flag bit set when the physics word reports pile-up.
    pub const PILEUP_FLAG_BIT: u32 = 1 << 15;
}

// ---------------------------------------------------------------------------
// Shared structures
// ---------------------------------------------------------------------------

/// Parsed Board Aggregate header (4 words). Identical for PSD1 + PHA1.
#[derive(Debug, Clone)]
pub struct Dig1BoardHeader {
    pub aggregate_size: u32,
    pub board_id: u8,
    pub board_fail: bool,
    pub dual_channel_mask: u8,
    pub aggregate_counter: u32,
    pub board_time_tag: u32,
}

/// Unified Dual Channel Header. PSD1 + PHA1 share bit positions 27-31; only
/// the per-firmware DUAL_CHANNEL_SIZE_MASK differs (handled by
/// [`Dig1Variant::DUAL_CHANNEL_SIZE_MASK`]).
#[derive(Debug, Clone)]
pub struct Dig1ChannelHeader {
    pub block_size: u32,
    pub num_samples_wave: u16,
    pub extra_option: u8,
    /// ES (bit 27): waveform samples present.
    pub samples_enabled: bool,
    /// PSD1 EE / PHA1 E2 (bit 28): EXTRAS word present.
    pub extras_enabled: bool,
    /// ET (bit 29): trigger time tag word present.
    pub time_enabled: bool,
    /// PSD1 EQ / PHA1 EE (bit 30): physics word (charge or energy) present.
    pub physics_enabled: bool,
    /// DT (bit 31): dual-trace mode.
    pub dual_trace: bool,
}

impl Dig1ChannelHeader {
    /// Words per event derived from the enable flags. Mirrors the original
    /// per-FW `event_size_words` — all enable flags map to a single 32-bit
    /// word except samples (`num_samples_wave * 4`).
    pub fn event_size_words(&self) -> usize {
        let mut size = 0;
        if self.time_enabled {
            size += 1;
        }
        if self.extras_enabled {
            size += 1;
        }
        if self.samples_enabled {
            // CAEN spec: waveform words = num_samples_wave * 4 (2 raw samples per word).
            size += self.num_samples_wave as usize * 4;
        }
        if self.physics_enabled {
            size += 1;
        }
        size
    }
}

/// Configuration shared by Psd1Decoder + Pha1Decoder.
#[derive(Debug, Clone)]
pub struct Dig1Config {
    /// Time step in nanoseconds (DT5730 = 2.0 at 500 MS/s).
    pub time_step_ns: f64,
    /// Module identifier carried into emitted [`EventData`].
    pub module_id: u8,
    /// Print decode-time diagnostics to stdout. Off in production.
    pub dump_enabled: bool,
}

impl Default for Dig1Config {
    fn default() -> Self {
        Self {
            time_step_ns: 2.0,
            module_id: 0,
            dump_enabled: false,
        }
    }
}

/// Result of decoding the EXTRAS word. PSD1 + PHA1 share the option-0/1/2/5
/// shape; only [`Dig1Variant::calculate_sw_fine_fraction`] turns the option-5
/// pair into a fraction.
pub enum ExtrasDecoded {
    HwFineTs {
        extended_time: u16,
        fine_time: u16,
        flags: u32,
    },
    SwFineTs {
        before_zc: u16,
        after_zc: u16,
    },
}

// ---------------------------------------------------------------------------
// Per-firmware variant trait
// ---------------------------------------------------------------------------

/// Per-firmware customization for the shared DT5730/VX1730 decoder.
/// Implementors must remain `pub`-visible so [`Dig1Decoder`]'s generic param
/// monomorphizes at the call site.
pub trait Dig1Variant {
    /// Used in dump / log prefixes (e.g. `[PSD1]` / `[PHA1]`).
    const FW_NAME: &'static str;
    /// PSD1: 22-bit (`0x003F_FFFF`). PHA1: 31-bit (`0x7FFF_FFFF`).
    const DUAL_CHANNEL_SIZE_MASK: u32;

    /// SW Fine TS interpolation fraction from EXTRAS option 0b101 zero-cross
    /// samples. PSD1: 14-bit unsigned with 8192 baseline. PHA1: signed
    /// (RC-CR2 zero-centered).
    fn calculate_sw_fine_fraction(before_zc: u16, after_zc: u16) -> f64;

    /// Decode the per-event physics word.
    /// Returns: `(energy_long, energy_short_or_extra, pileup)`.
    /// PSD1: `(charge_long, charge_short, pileup)`.
    /// PHA1: `(energy, extra_data, pileup)`.
    fn decode_physics_word(word: u32) -> (u16, u16, bool);

    /// Decode the variant-specific waveform layout starting at `*offset`,
    /// advancing it by `header.num_samples_wave * 4 * WORD_SIZE` bytes.
    /// PSD1: unsigned 14-bit + DP1@14 + DP2@15. PHA1: sign-extended 14-bit
    /// + DP@14 + Tn@15 (Tn → digital_probe1, DP → digital_probe2).
    fn decode_waveform(
        data: &[u8],
        offset: &mut usize,
        header: &Dig1ChannelHeader,
        ns_per_sample: f64,
    ) -> Waveform;
}

// ---------------------------------------------------------------------------
// Generic decoder
// ---------------------------------------------------------------------------

/// Generic DT5730/VX1730 decoder, monomorphized per firmware via [`Dig1Variant`].
pub struct Dig1Decoder<V: Dig1Variant> {
    pub config: Dig1Config,
    last_aggregate_counter: u32,
    /// 32-bit Board Aggregate TimeTag rollover tracker. Pure modulo
    /// arithmetic — no host-clock dependency. (See
    /// `memory/layering_principle_clock_sync.md`: clock sync belongs at the
    /// physical layer or above the decoder, never inside it.)
    tracker: RolloverTracker,
    /// Current extended board time in `time_step_ns` ticks (updated per
    /// board aggregate).
    extended_board_time: u64,
    _phantom: PhantomData<V>,
}

impl<V: Dig1Variant> Dig1Decoder<V> {
    pub fn new(config: Dig1Config) -> Self {
        Self {
            config,
            last_aggregate_counter: 0,
            tracker: RolloverTracker::new(32),
            extended_board_time: 0,
            _phantom: PhantomData,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(Dig1Config::default())
    }

    pub fn set_dump_enabled(&mut self, enabled: bool) {
        self.config.dump_enabled = enabled;
    }

    /// Reset state for a new run. Must be called when the hardware counter
    /// is known to have been cleared (CAEN SW Start Acquisition).
    pub fn reset_for_new_run(&mut self) {
        self.last_aggregate_counter = 0;
        self.tracker.reset();
        self.extended_board_time = 0;
    }

    /// Classify the data type. PSD1/PHA1 have no Start/Stop signals in the
    /// data stream — returns Event for valid board headers, Unknown otherwise.
    pub fn classify(&self, raw: &RawData) -> DataType {
        if raw.size < board_header_bits::HEADER_SIZE_BYTES {
            return DataType::Unknown;
        }
        if !raw.size.is_multiple_of(WORD_SIZE) {
            return DataType::Unknown;
        }
        let word0 = read_u32(&raw.data, 0);
        let header_type =
            (word0 >> board_header_bits::TYPE_SHIFT) & board_header_bits::TYPE_MASK;
        if header_type == board_header_bits::TYPE_DATA {
            DataType::Event
        } else {
            DataType::Unknown
        }
    }

    pub fn decode(&mut self, raw: &RawData) -> Vec<EventData> {
        let mut events = Vec::new();
        self.decode_into(raw, &mut events);
        events
    }

    /// Decode raw data into a reusable Vec (caller-allocated, cleared first).
    pub fn decode_into(&mut self, raw: &RawData, all_events: &mut Vec<EventData>) {
        all_events.clear();

        let data_type = self.classify(raw);
        if data_type != DataType::Event {
            if self.config.dump_enabled {
                println!("[{}] Non-event data, size={}", V::FW_NAME, raw.size);
            }
            return;
        }

        let total_bytes = raw.size;
        let mut offset: usize = 0;

        while offset + board_header_bits::HEADER_SIZE_BYTES <= total_bytes {
            match self.decode_board_aggregate(&raw.data, &mut offset) {
                Ok(mut events) => all_events.append(&mut events),
                Err(msg) => {
                    if self.config.dump_enabled {
                        println!("[{}] Board aggregate error: {}", V::FW_NAME, msg);
                    }
                    break;
                }
            }
        }

        all_events.sort_by(|a, b| {
            a.timestamp_ns
                .partial_cmp(&b.timestamp_ns)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if self.config.dump_enabled {
            println!("[{}] Decoded {} events", V::FW_NAME, all_events.len());
        }
    }

    fn decode_board_aggregate(
        &mut self,
        data: &[u8],
        offset: &mut usize,
    ) -> Result<Vec<EventData>, String> {
        let header = decode_board_header(data, *offset)?;

        let block_end = *offset + (header.aggregate_size as usize) * WORD_SIZE;
        if block_end > data.len() {
            return Err(format!(
                "Board aggregate size {} exceeds data length {}",
                block_end,
                data.len()
            ));
        }

        if self.last_aggregate_counter != 0
            && header.aggregate_counter != self.last_aggregate_counter.wrapping_add(1)
            && self.config.dump_enabled
        {
            println!(
                "[{}] Aggregate counter discontinuity: {} -> {}",
                V::FW_NAME,
                self.last_aggregate_counter,
                header.aggregate_counter
            );
        }
        self.last_aggregate_counter = header.aggregate_counter;

        // Extend the 32-bit board_time_tag to 64-bit ticks via pure modulo
        // rollover tracking. On underflow (which shouldn't happen at
        // production rates — would require a backward jump > half a period)
        // fall back to the previous extended value rather than poisoning
        // downstream sort invariants.
        let prev_extended = self.extended_board_time;
        self.extended_board_time = self
            .tracker
            .extend(header.board_time_tag as u64)
            .unwrap_or_else(|e| {
                tracing::warn!(
                    fw = V::FW_NAME,
                    btt = header.board_time_tag,
                    error = ?e,
                    "rollover tracker underflow — falling back to previous extended time"
                );
                prev_extended
            });

        if header.board_fail && self.config.dump_enabled {
            println!("[{}] Board fail bit set!", V::FW_NAME);
        }

        *offset += board_header_bits::HEADER_SIZE_WORDS * WORD_SIZE;

        let mut events = Vec::new();
        let mask = header.dual_channel_mask;

        for pair_index in 0u8..8 {
            if mask & (1 << pair_index) == 0 {
                continue;
            }
            if *offset >= block_end {
                break;
            }

            match self.decode_dual_channel_block(data, offset, pair_index, block_end) {
                Ok(mut ch_events) => events.append(&mut ch_events),
                Err(msg) => {
                    if self.config.dump_enabled {
                        println!(
                            "[{}] Dual channel pair {} error: {}",
                            V::FW_NAME,
                            pair_index,
                            msg
                        );
                    }
                    *offset = block_end;
                    break;
                }
            }
        }

        *offset = block_end;
        Ok(events)
    }

    fn decode_dual_channel_block(
        &mut self,
        data: &[u8],
        offset: &mut usize,
        pair_index: u8,
        block_end: usize,
    ) -> Result<Vec<EventData>, String> {
        let ch_header = decode_dual_channel_header::<V>(data, *offset)?;

        let ch_block_end = *offset + (ch_header.block_size as usize) * WORD_SIZE;
        let ch_block_end = ch_block_end.min(block_end);

        *offset += channel_header_bits::HEADER_SIZE_WORDS * WORD_SIZE;

        let event_size = ch_header.event_size_words();
        if event_size == 0 {
            return Ok(vec![]);
        }

        let mut events = Vec::new();

        while *offset + event_size * WORD_SIZE <= ch_block_end {
            match self.decode_event(data, offset, &ch_header, pair_index) {
                Ok(event) => events.push(event),
                Err(msg) => {
                    if self.config.dump_enabled {
                        println!("[{}] Event decode error: {}", V::FW_NAME, msg);
                    }
                    break;
                }
            }
        }

        *offset = ch_block_end;
        Ok(events)
    }

    fn decode_event(
        &mut self,
        data: &[u8],
        offset: &mut usize,
        ch_header: &Dig1ChannelHeader,
        pair_index: u8,
    ) -> Result<EventData, String> {
        // Event data order per CAEN PDF (Fig. 2.2):
        //   1. Time tag (if ET=1)
        //   2. Waveform (if ES=1)
        //   3. Extras / Extras2 (if EE/E2=1)
        //   4. Physics word: charge for PSD1 / energy for PHA1 (if EQ/EE=1)

        // 1. Time tag
        let mut trigger_time_tag: u32 = 0;
        let mut channel_flag: u8 = 0;
        if ch_header.time_enabled {
            let w = read_u32(data, *offset);
            *offset += WORD_SIZE;
            channel_flag = ((w >> event_bits::CHANNEL_FLAG_SHIFT) & 1) as u8;
            trigger_time_tag = w & event_bits::TRIGGER_TIME_MASK;
        }

        // 2. Waveform
        let waveform = if ch_header.samples_enabled {
            Some(V::decode_waveform(
                data,
                offset,
                ch_header,
                self.config.time_step_ns,
            ))
        } else {
            None
        };

        // 3. Extras
        let mut extended_time: u16 = 0;
        let mut fine_time: u16 = 0;
        let mut flags: u32 = 0;
        let mut sw_fine_fraction: Option<f64> = None;
        if ch_header.extras_enabled {
            let w = read_u32(data, *offset);
            *offset += WORD_SIZE;
            match decode_extras_word(w, ch_header.extra_option) {
                ExtrasDecoded::HwFineTs {
                    extended_time: ext,
                    fine_time: ft,
                    flags: fl,
                } => {
                    extended_time = ext;
                    fine_time = ft;
                    flags = fl;
                }
                ExtrasDecoded::SwFineTs {
                    before_zc,
                    after_zc,
                } => {
                    let frac = V::calculate_sw_fine_fraction(before_zc, after_zc);
                    fine_time = (frac * 1024.0).clamp(0.0, 1023.0) as u16;
                    sw_fine_fraction = Some(frac);
                }
            }
        }

        // 4. Physics word (charge / energy)
        let mut energy_long: u16 = 0;
        let mut energy_short: u16 = 0;
        if ch_header.physics_enabled {
            let w = read_u32(data, *offset);
            *offset += WORD_SIZE;
            let (el, es, pileup) = V::decode_physics_word(w);
            energy_long = el;
            energy_short = es;
            if pileup {
                flags |= event_bits::PILEUP_FLAG_BIT;
            }
        }

        let channel = pair_index * 2 + channel_flag;

        let timestamp_ns = if let Some(frac) = sw_fine_fraction {
            // SW Fine TS: reconstruct the 31-bit event TTT against the
            // 32-bit BoardAggregate context so roll-overs line up. The
            // fraction contributes a sub-tick offset.
            let full_ttt = self.tracker.reconstruct_subcounter(
                self.extended_board_time,
                trigger_time_tag as u64,
                31,
            );
            (full_ttt as f64) * self.config.time_step_ns + frac * self.config.time_step_ns
        } else {
            calculate_timestamp(&self.config, trigger_time_tag, extended_time, fine_time)
        };

        if self.config.dump_enabled {
            println!("--- {} Event ---", V::FW_NAME);
            println!("  Channel:      {}", channel);
            println!("  Timestamp:    {:.3} ns", timestamp_ns);
            println!("  Energy:       {}", energy_long);
            println!("  Energy short: {}", energy_short);
            println!("  Fine Time:    {}", fine_time);
            println!("  Flags:        0x{:08x}", flags);
        }

        Ok(EventData {
            timestamp_ns,
            module: self.config.module_id,
            channel,
            energy: energy_long,
            energy_short,
            fine_time,
            flags,
            user_info: [0; 4],
            waveform,
        })
    }
}

// ---------------------------------------------------------------------------
// Free helpers (pure, easy to test)
// ---------------------------------------------------------------------------

/// Read a u32 at `offset` (Little-Endian). Returns 0 on out-of-bounds — the
/// caller-guarded framing checks should make this unreachable in practice.
#[inline]
pub fn read_u32(data: &[u8], offset: usize) -> u32 {
    data.get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

/// Decode the 4-word board aggregate header at `offset`. Returns a parsed
/// header or a descriptive error on either type-mismatch (not 0xA) or
/// insufficient data.
pub fn decode_board_header(data: &[u8], offset: usize) -> Result<Dig1BoardHeader, String> {
    if offset + board_header_bits::HEADER_SIZE_BYTES > data.len() {
        return Err("Insufficient data for board header".to_string());
    }

    let w0 = read_u32(data, offset);
    let w1 = read_u32(data, offset + 4);
    let w2 = read_u32(data, offset + 8);
    let w3 = read_u32(data, offset + 12);

    let header_type = (w0 >> board_header_bits::TYPE_SHIFT) & board_header_bits::TYPE_MASK;
    if header_type != board_header_bits::TYPE_DATA {
        return Err(format!(
            "Invalid header type: 0x{:x} (expected 0xA)",
            header_type
        ));
    }

    Ok(Dig1BoardHeader {
        aggregate_size: w0 & board_header_bits::AGGREGATE_SIZE_MASK,
        board_id: ((w1 >> board_header_bits::BOARD_ID_SHIFT)
            & board_header_bits::BOARD_ID_MASK) as u8,
        board_fail: ((w1 >> board_header_bits::BOARD_FAIL_SHIFT) & 1) != 0,
        dual_channel_mask: (w1 & board_header_bits::DUAL_CHANNEL_MASK) as u8,
        aggregate_counter: w2 & board_header_bits::COUNTER_MASK,
        board_time_tag: w3,
    })
}

/// Decode the 2-word dual-channel header at `offset` for variant `V`. The
/// only per-variant difference is `DUAL_CHANNEL_SIZE_MASK` — everything else
/// is at identical bit positions.
pub fn decode_dual_channel_header<V: Dig1Variant>(
    data: &[u8],
    offset: usize,
) -> Result<Dig1ChannelHeader, String> {
    let needed = channel_header_bits::HEADER_SIZE_WORDS * WORD_SIZE;
    if offset + needed > data.len() {
        return Err("Insufficient data for channel header".to_string());
    }

    let w0 = read_u32(data, offset);
    let w1 = read_u32(data, offset + 4);

    Ok(Dig1ChannelHeader {
        block_size: w0 & V::DUAL_CHANNEL_SIZE_MASK,
        num_samples_wave: (w1 & channel_header_bits::NUM_SAMPLES_MASK) as u16,
        extra_option: ((w1 >> channel_header_bits::EXTRA_OPTION_SHIFT)
            & channel_header_bits::EXTRA_OPTION_MASK) as u8,
        samples_enabled: ((w1 >> channel_header_bits::SAMPLES_ENABLED_SHIFT) & 1) != 0,
        extras_enabled: ((w1 >> channel_header_bits::EXTRAS_ENABLED_SHIFT) & 1) != 0,
        time_enabled: ((w1 >> channel_header_bits::TIME_ENABLED_SHIFT) & 1) != 0,
        physics_enabled: ((w1 >> channel_header_bits::PHYSICS_ENABLED_SHIFT) & 1) != 0,
        dual_trace: ((w1 >> channel_header_bits::DUAL_TRACE_SHIFT) & 1) != 0,
    })
}

/// Decode the EXTRAS / EXTRAS2 word according to `extra_option`. Both
/// firmwares share options 0/1/2 (HwFineTs variants) and 5 (SwFineTs zero-cross
/// pair). Option 5's interpretation differs by variant — the fraction is
/// computed by [`Dig1Variant::calculate_sw_fine_fraction`] downstream.
pub fn decode_extras_word(word: u32, extra_option: u8) -> ExtrasDecoded {
    match extra_option {
        // 0b010: Extended time + flags + fine time
        2 => {
            let extended_time = ((word >> event_bits::EXTENDED_TIME_SHIFT)
                & event_bits::EXTENDED_TIME_MASK) as u16;
            let flags = (word >> event_bits::FLAGS_SHIFT) & event_bits::FLAGS_MASK;
            let fine_time = (word & event_bits::FINE_TIME_MASK) as u16;
            ExtrasDecoded::HwFineTs {
                extended_time,
                fine_time,
                flags,
            }
        }
        // 0b101: SAZC/SBZC (PSD1) or EBZC/EAZC (PHA1) zero-cross sample pair.
        // Verified by fine_ts_verify on real DT5730: bits[31:16] = before ZC,
        // bits[15:0] = after ZC.
        5 => {
            let before_zc = ((word >> 16) & 0xFFFF) as u16;
            let after_zc = (word & 0xFFFF) as u16;
            ExtrasDecoded::SwFineTs {
                before_zc,
                after_zc,
            }
        }
        // 0b001: Extended time + flags (16-bit)
        1 => {
            let extended_time = ((word >> event_bits::EXTENDED_TIME_SHIFT)
                & event_bits::EXTENDED_TIME_MASK) as u16;
            let flags = word & 0xFFFF;
            ExtrasDecoded::HwFineTs {
                extended_time,
                fine_time: 0,
                flags,
            }
        }
        // 0b000 and others: Extended time + baseline×4 (PSD1) / baseline (PHA1)
        _ => {
            let extended_time = ((word >> event_bits::EXTENDED_TIME_SHIFT)
                & event_bits::EXTENDED_TIME_MASK) as u16;
            ExtrasDecoded::HwFineTs {
                extended_time,
                fine_time: 0,
                flags: 0,
            }
        }
    }
}

/// HW Fine TS timestamp combination: `(ext_time << 31) | trigger_time_tag`
/// in `time_step_ns` ticks plus fine-time sub-tick.
pub fn calculate_timestamp(
    config: &Dig1Config,
    trigger_time_tag: u32,
    extended_time: u16,
    fine_time: u16,
) -> f64 {
    let combined = ((extended_time as u64) << 31) | (trigger_time_tag as u64);
    let coarse_ns = (combined as f64) * config.time_step_ns;
    let fine_ns = (fine_time as f64) * (config.time_step_ns / event_bits::FINE_TIME_SCALE);
    coarse_ns + fine_ns
}

// ---------------------------------------------------------------------------
// Tests — Mock variant exercises the framing in isolation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock variant with 22-bit size mask (PSD1-shaped) for framing-only tests.
    /// Physics word format mirrors PSD1 so we can build identical test frames.
    struct MockVariant;

    impl Dig1Variant for MockVariant {
        const FW_NAME: &'static str = "MOCK";
        const DUAL_CHANNEL_SIZE_MASK: u32 = 0x3F_FFFF;

        fn calculate_sw_fine_fraction(_before_zc: u16, _after_zc: u16) -> f64 {
            0.5
        }

        fn decode_physics_word(word: u32) -> (u16, u16, bool) {
            let energy = (word & 0x7FFF) as u16;
            let pileup = ((word >> 15) & 1) != 0;
            let extra = ((word >> 16) & 0xFFFF) as u16;
            (energy, extra, pileup)
        }

        fn decode_waveform(
            _data: &[u8],
            offset: &mut usize,
            header: &Dig1ChannelHeader,
            ns_per_sample: f64,
        ) -> Waveform {
            // Mock just advances the offset past the waveform bytes without
            // parsing them — adequate for framing tests.
            *offset += header.num_samples_wave as usize * 4 * WORD_SIZE;
            Waveform {
                ns_per_sample,
                ..Waveform::default()
            }
        }
    }

    type MockDecoder = Dig1Decoder<MockVariant>;

    fn push_u32(buf: &mut Vec<u8>, value: u32) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn make_board_header(aggregate_size: u32, mask: u8, board_id: u8, counter: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        push_u32(&mut buf, (0xA << 28) | (aggregate_size & 0x0FFF_FFFF));
        push_u32(&mut buf, ((board_id as u32) << 27) | (mask as u32));
        push_u32(&mut buf, counter & 0x7F_FFFF);
        push_u32(&mut buf, 0x1234_5678);
        buf
    }

    /// Channel-header build flags. Bit positions match production PSD1/PHA1
    /// (since the whole point of this module is they're identical).
    struct ChFlags {
        time: bool,
        extras: bool,
        physics: bool,
        samples: bool,
        dual_trace: bool,
        extra_option: u8,
        num_samples: u16,
    }

    impl Default for ChFlags {
        fn default() -> Self {
            Self {
                time: true,
                extras: true,
                physics: true,
                samples: false,
                dual_trace: false,
                extra_option: 2,
                num_samples: 0,
            }
        }
    }

    fn make_dual_channel_header(size: u32, flags: &ChFlags) -> Vec<u8> {
        let mut buf = Vec::new();
        push_u32(&mut buf, (1 << 31) | (size & 0x3F_FFFF));
        let mut w1: u32 = flags.num_samples as u32;
        w1 |= (flags.extra_option as u32 & 0x7) << 24;
        if flags.samples {
            w1 |= 1 << 27;
        }
        if flags.extras {
            w1 |= 1 << 28;
        }
        if flags.time {
            w1 |= 1 << 29;
        }
        if flags.physics {
            w1 |= 1 << 30;
        }
        if flags.dual_trace {
            w1 |= 1 << 31;
        }
        push_u32(&mut buf, w1);
        buf
    }

    fn make_time_word(trigger_time: u32, odd_channel: bool) -> u32 {
        let mut w = trigger_time & 0x7FFF_FFFF;
        if odd_channel {
            w |= 1 << 31;
        }
        w
    }

    fn make_extras_word(extended_time: u16, flags: u8, fine_time: u16) -> u32 {
        ((extended_time as u32) << 16)
            | (((flags as u32) & 0x3F) << 10)
            | ((fine_time as u32) & 0x3FF)
    }

    fn make_physics_word(energy: u16, extra: u16, pileup: bool) -> u32 {
        let mut w = ((extra as u32) & 0xFFFF) << 16 | ((energy as u32) & 0x7FFF);
        if pileup {
            w |= 1 << 15;
        }
        w
    }

    fn default_decoder() -> MockDecoder {
        MockDecoder::with_defaults()
    }

    // -----------------------------------------------------------------------
    // classify
    // -----------------------------------------------------------------------

    #[test]
    fn classify_too_small() {
        let dec = default_decoder();
        let raw = RawData::new(vec![0; 12]);
        assert_eq!(dec.classify(&raw), DataType::Unknown);
    }

    #[test]
    fn classify_not_word_aligned() {
        let dec = default_decoder();
        let raw = RawData::new(vec![0; 17]);
        assert_eq!(dec.classify(&raw), DataType::Unknown);
    }

    #[test]
    fn classify_valid_data_header() {
        let dec = default_decoder();
        let data = make_board_header(4, 0x01, 0, 1);
        assert_eq!(dec.classify(&RawData::new(data)), DataType::Event);
    }

    #[test]
    fn classify_invalid_header_type() {
        let dec = default_decoder();
        let mut data = vec![0u8; 16];
        data[..4].copy_from_slice(&0xB000_0004u32.to_le_bytes());
        assert_eq!(dec.classify(&RawData::new(data)), DataType::Unknown);
    }

    #[test]
    fn classify_no_start_or_stop() {
        // PSD1/PHA1 family never produces Start/Stop on the wire.
        let dec = default_decoder();
        let raw = RawData::new(make_board_header(4, 0x01, 0, 1));
        let dt = dec.classify(&raw);
        assert_ne!(dt, DataType::Start);
        assert_ne!(dt, DataType::Stop);
    }

    // -----------------------------------------------------------------------
    // Board header parsing
    // -----------------------------------------------------------------------

    #[test]
    fn decode_board_header_basic_fields() {
        let data = make_board_header(100, 0x03, 5, 42);
        let header = decode_board_header(&data, 0).unwrap();
        assert_eq!(header.aggregate_size, 100);
        assert_eq!(header.dual_channel_mask, 0x03);
        assert_eq!(header.board_id, 5);
        assert_eq!(header.aggregate_counter, 42);
        assert!(!header.board_fail);
        assert_eq!(header.board_time_tag, 0x1234_5678);
    }

    #[test]
    fn decode_board_header_fail_bit() {
        let mut data = make_board_header(4, 0x01, 0, 1);
        let w1 = read_u32(&data, 4) | (1 << 26);
        data[4..8].copy_from_slice(&w1.to_le_bytes());
        let header = decode_board_header(&data, 0).unwrap();
        assert!(header.board_fail);
    }

    #[test]
    fn decode_board_header_insufficient_data() {
        let data = vec![0u8; 12];
        assert!(decode_board_header(&data, 0).is_err());
    }

    #[test]
    fn decode_board_header_wrong_type_errors() {
        let mut data = vec![0u8; 16];
        data[..4].copy_from_slice(&0xB000_0004u32.to_le_bytes());
        assert!(decode_board_header(&data, 0).is_err());
    }

    // -----------------------------------------------------------------------
    // Dual channel header parsing
    // -----------------------------------------------------------------------

    #[test]
    fn dual_channel_header_default_flags() {
        let data = make_dual_channel_header(50, &ChFlags::default());
        let h = decode_dual_channel_header::<MockVariant>(&data, 0).unwrap();
        assert_eq!(h.block_size, 50);
        assert!(h.time_enabled);
        assert!(h.extras_enabled);
        assert!(h.physics_enabled);
        assert!(!h.samples_enabled);
        assert!(!h.dual_trace);
        assert_eq!(h.extra_option, 2);
        assert_eq!(h.num_samples_wave, 0);
    }

    #[test]
    fn dual_channel_header_all_flags_on() {
        let data = make_dual_channel_header(
            100,
            &ChFlags {
                samples: true,
                dual_trace: true,
                extra_option: 5,
                num_samples: 16,
                ..ChFlags::default()
            },
        );
        let h = decode_dual_channel_header::<MockVariant>(&data, 0).unwrap();
        assert!(h.dual_trace);
        assert!(h.physics_enabled);
        assert!(h.time_enabled);
        assert!(h.extras_enabled);
        assert!(h.samples_enabled);
        assert_eq!(h.num_samples_wave, 16);
        assert_eq!(h.extra_option, 5);
    }

    #[test]
    fn channel_header_event_size_minimal() {
        let h = Dig1ChannelHeader {
            block_size: 0,
            num_samples_wave: 0,
            extra_option: 2,
            samples_enabled: false,
            extras_enabled: true,
            time_enabled: true,
            physics_enabled: true,
            dual_trace: false,
        };
        assert_eq!(h.event_size_words(), 3);
    }

    #[test]
    fn channel_header_event_size_with_waveform() {
        let h = Dig1ChannelHeader {
            block_size: 0,
            num_samples_wave: 4, // 4 * 4 = 16 wf words
            extra_option: 2,
            samples_enabled: true,
            extras_enabled: true,
            time_enabled: true,
            physics_enabled: true,
            dual_trace: false,
        };
        assert_eq!(h.event_size_words(), 3 + 16);
    }

    // -----------------------------------------------------------------------
    // Extras word parsing
    // -----------------------------------------------------------------------

    #[test]
    fn extras_option2_returns_hw_fine_ts() {
        let word = make_extras_word(0x1234, 0x2A, 500);
        match decode_extras_word(word, 2) {
            ExtrasDecoded::HwFineTs {
                extended_time,
                fine_time,
                flags,
            } => {
                assert_eq!(extended_time, 0x1234);
                assert_eq!(fine_time, 500);
                assert_eq!(flags, 0x2A);
            }
            _ => panic!("expected HwFineTs"),
        }
    }

    #[test]
    fn extras_option5_returns_sw_fine_ts() {
        let before_zc: u16 = 100;
        let after_zc: u16 = 200;
        let word: u32 = ((before_zc as u32) << 16) | (after_zc as u32);
        match decode_extras_word(word, 5) {
            ExtrasDecoded::SwFineTs {
                before_zc: b,
                after_zc: a,
            } => {
                assert_eq!(b, before_zc);
                assert_eq!(a, after_zc);
            }
            _ => panic!("expected SwFineTs"),
        }
    }

    #[test]
    fn extras_option1_returns_hw_fine_ts_no_fine() {
        let word: u32 = (0x5678_u32 << 16) | 0x00FF;
        match decode_extras_word(word, 1) {
            ExtrasDecoded::HwFineTs {
                extended_time,
                fine_time,
                flags,
            } => {
                assert_eq!(extended_time, 0x5678);
                assert_eq!(fine_time, 0);
                assert_eq!(flags, 0x00FF);
            }
            _ => panic!("expected HwFineTs"),
        }
    }

    // -----------------------------------------------------------------------
    // Timestamp helper
    // -----------------------------------------------------------------------

    #[test]
    fn calculate_timestamp_basic() {
        let cfg = Dig1Config::default();
        let ts = calculate_timestamp(&cfg, 1000, 0, 0);
        assert!((ts - 2000.0).abs() < 0.001);
    }

    #[test]
    fn calculate_timestamp_extended_only() {
        let cfg = Dig1Config::default();
        let ts = calculate_timestamp(&cfg, 0, 1, 0);
        let expected = (1u64 << 31) as f64 * 2.0;
        assert!((ts - expected).abs() < 1.0);
    }

    #[test]
    fn calculate_timestamp_fine_only() {
        let cfg = Dig1Config::default();
        let ts = calculate_timestamp(&cfg, 0, 0, 512);
        assert!((ts - 1.0).abs() < 0.001);
    }

    // -----------------------------------------------------------------------
    // End-to-end framing via Mock variant
    // -----------------------------------------------------------------------

    #[test]
    fn decode_single_event_end_to_end() {
        let mut dec = default_decoder();
        let event_words = 3; // time + extras + physics
        let ch_size = 2 + event_words;
        let total_size = 4 + ch_size;

        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(1000, false));
        push_u32(&mut data, make_extras_word(0, 0, 100));
        push_u32(&mut data, make_physics_word(5000, 500, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.channel, 0);
        assert_eq!(e.energy, 5000);
        assert_eq!(e.energy_short, 500);
        assert_eq!(e.fine_time, 100);
        assert!(e.waveform.is_none());
    }

    #[test]
    fn decode_odd_channel_uses_channel_flag_bit() {
        let mut dec = default_decoder();
        let ch_size = 2 + 3;
        let total_size = 4 + ch_size;
        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(1000, true)); // odd channel flag
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].channel, 1);
    }

    #[test]
    fn decode_channel_pair_offset_maps_to_higher_channels() {
        let mut dec = default_decoder();
        let ch_size = 2 + 3;
        let total_size = 4 + ch_size;
        // mask = 0x04 → pair index 2 → channel 4 (even flag)
        let mut data = make_board_header(total_size as u32, 0x04, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(1000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].channel, 4);
    }

    #[test]
    fn decode_pileup_sets_flag_bit_15() {
        let mut dec = default_decoder();
        let ch_size = 2 + 3;
        let total_size = 4 + ch_size;
        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(1000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, true)); // pileup

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        assert_ne!(events[0].flags & event_bits::PILEUP_FLAG_BIT, 0);
    }

    #[test]
    fn decode_multiple_events_sorted_by_timestamp() {
        let mut dec = default_decoder();
        let ch_size = 2 + 3 * 2;
        let total_size = 4 + ch_size;
        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        // Later event first
        push_u32(&mut data, make_time_word(5000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(500, 250, false));
        // Earlier event second
        push_u32(&mut data, make_time_word(1000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 2);
        assert!(events[0].timestamp_ns < events[1].timestamp_ns);
        assert_eq!(events[0].energy, 100);
        assert_eq!(events[1].energy, 500);
    }

    #[test]
    fn decode_multiple_board_aggregates() {
        let mut dec = default_decoder();
        let ch_size = 2 + 3;
        let block_size = 4 + ch_size;
        let mut data = Vec::new();

        data.extend(make_board_header(block_size as u32, 0x01, 0, 1));
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(1000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, false));

        data.extend(make_board_header(block_size as u32, 0x01, 0, 2));
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(2000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(200, 100, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].energy, 100);
        assert_eq!(events[1].energy, 200);
    }

    #[test]
    fn decode_skips_waveform_when_samples_enabled() {
        // Mock variant just bumps the offset by num_samples_wave * 4 * WORD_SIZE.
        let mut dec = default_decoder();
        let num_samples_wave: u16 = 1;
        let wf_words = num_samples_wave as usize * 4;
        let ch_size = 2 + 3 + wf_words;
        let total_size = 4 + ch_size;

        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(
            ch_size as u32,
            &ChFlags {
                samples: true,
                num_samples: num_samples_wave,
                ..ChFlags::default()
            },
        ));
        push_u32(&mut data, make_time_word(1000, false));
        for _ in 0..wf_words {
            push_u32(&mut data, 0xCAFE_BABE);
        }
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        assert!(events[0].waveform.is_some()); // mock returns Default Waveform
        assert_eq!(events[0].energy, 100);
    }

    #[test]
    fn decode_charge_only_event_no_time_no_extras() {
        let mut dec = default_decoder();
        let ch_size = 2 + 1;
        let total_size = 4 + ch_size;

        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(
            ch_size as u32,
            &ChFlags {
                time: false,
                extras: false,
                ..ChFlags::default()
            },
        ));
        push_u32(&mut data, make_physics_word(999, 444, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].energy, 999);
        assert_eq!(events[0].energy_short, 444);
        assert_eq!(events[0].channel, 0);
        assert!(events[0].timestamp_ns.abs() < 0.001);
    }

    #[test]
    fn module_id_propagates_to_emitted_events() {
        let mut dec = MockDecoder::new(Dig1Config {
            time_step_ns: 2.0,
            module_id: 7,
            dump_enabled: false,
        });
        let ch_size = 2 + 3;
        let total_size = 4 + ch_size;
        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(1000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, false));

        let events = dec.decode(&RawData::new(data));
        assert_eq!(events[0].module, 7);
    }

    #[test]
    fn reset_for_new_run_clears_tracker_state() {
        let mut dec = default_decoder();
        let ch_size = 2 + 3;
        let total_size = 4 + ch_size;
        let mut data = make_board_header(total_size as u32, 0x01, 0, 1);
        data.extend(make_dual_channel_header(ch_size as u32, &ChFlags::default()));
        push_u32(&mut data, make_time_word(1000, false));
        push_u32(&mut data, make_extras_word(0, 0, 0));
        push_u32(&mut data, make_physics_word(100, 50, false));

        // Run once to advance internal state.
        let _ = dec.decode(&RawData::new(data.clone()));
        assert_ne!(dec.last_aggregate_counter, 0);
        assert_ne!(dec.extended_board_time, 0);

        dec.reset_for_new_run();
        assert_eq!(dec.last_aggregate_counter, 0);
        assert_eq!(dec.extended_board_time, 0);
    }

    #[test]
    fn empty_input_yields_no_events() {
        let mut dec = default_decoder();
        let events = dec.decode(&RawData::new(vec![]));
        assert!(events.is_empty());
    }

    #[test]
    fn invalid_header_yields_no_events() {
        let mut dec = default_decoder();
        let mut data = vec![0u8; 16];
        data[..4].copy_from_slice(&0xB000_0004u32.to_le_bytes());
        let events = dec.decode(&RawData::new(data));
        assert!(events.is_empty());
    }
}

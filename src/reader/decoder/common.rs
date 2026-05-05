//! Common types for decoder module

use serde::{Deserialize, Serialize};

/// Raw data from digitizer
#[derive(Debug, Clone)]
pub struct RawData {
    pub data: Vec<u8>,
    pub size: usize,
    pub n_events: u32,
}

impl RawData {
    /// Create RawData from a byte vector
    pub fn new(data: Vec<u8>) -> Self {
        let size = data.len();
        Self {
            data,
            size,
            n_events: 0,
        }
    }
}

impl From<crate::reader::caen::RawData> for RawData {
    fn from(raw: crate::reader::caen::RawData) -> Self {
        Self {
            data: raw.data,
            size: raw.size,
            n_events: raw.n_events,
        }
    }
}

/// Data type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// Start of run signal
    Start,
    /// End of run signal
    Stop,
    /// Normal event data
    Event,
    /// Unknown or invalid data
    Unknown,
}

/// Decode result for error handling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeResult {
    Success,
    InvalidHeader,
    InsufficientData,
    CorruptedData,
    OutOfBounds,
}

/// Sentinel for "probe type unknown / not parsed". Used by FW that don't
/// carry probe-type info on the wire (PSD1/PSD2/PHA1/AMax/V1743) so the
/// frontend can fall back to a generic "A0" / "D0" label without claiming
/// a specific probe identity.
pub const UNKNOWN_PROBE_TYPE: u8 = 0xFF;

fn default_unknown_analog_probe_types() -> [u8; 2] {
    [UNKNOWN_PROBE_TYPE; 2]
}
fn default_unknown_digital_probe_types() -> [u8; 4] {
    [UNKNOWN_PROBE_TYPE; 4]
}

/// Waveform data from digitizer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waveform {
    /// Analog probe 1 samples
    pub analog_probe1: Vec<i16>,
    /// Analog probe 2 samples
    pub analog_probe2: Vec<i16>,
    /// Digital probe 1 samples (1-bit)
    pub digital_probe1: Vec<u8>,
    /// Digital probe 2 samples (1-bit)
    pub digital_probe2: Vec<u8>,
    /// Digital probe 3 samples (1-bit)
    pub digital_probe3: Vec<u8>,
    /// Digital probe 4 samples (1-bit)
    pub digital_probe4: Vec<u8>,

    /// Time resolution (0=1x, 1=2x, 2=4x, 3=8x)
    pub time_resolution: u8,
    /// Trigger threshold
    pub trigger_threshold: u16,
    /// Nanoseconds per waveform sample (set by decoder)
    #[serde(default)]
    pub ns_per_sample: f64,

    /// True when `analog_probe1` is sign-extended 14-bit data (PHA1
    /// trapezoid / Delta probe etc., range `[-8192, 8191]`). False for
    /// raw-ADC unsigned probes (PSD1 / PSD2 / AMax input, range
    /// `[0, 16383]`). The frontend uses this to decide whether to apply
    /// the +8191 visual centering offset.
    #[serde(default)]
    pub analog_probe1_is_signed: bool,
    /// Same as `analog_probe1_is_signed` for the second analog probe.
    #[serde(default)]
    pub analog_probe2_is_signed: bool,

    /// Probe-type identifier for analog probes 0 and 1 — PHA2 canonical
    /// encoding (CAEN doxygen `legacy/PHA2_Parameters/a00108.html`):
    /// 0=ADCInput, 1=TimeFilter, 2=EnergyFilter, 3=EnergyFilterBaseline,
    /// 4=EnergyFilterMinusBaseline, 0xFF=`UNKNOWN_PROBE_TYPE` (FW that
    /// doesn't carry probe-type info on the wire). The frontend maps
    /// these to display labels like "A0: TimeFilter".
    #[serde(default = "default_unknown_analog_probe_types")]
    pub analog_probe_type: [u8; 2],
    /// Probe-type identifier for digital probes 0..3 — PHA2 canonical
    /// encoding: 0=Trigger, 1=TimeFilterArmed, 2=ReTriggerGuard,
    /// 3=EnergyFilterBaselineFreeze, 4=EnergyFilterPeaking,
    /// 5=EnergyFilterPeakReady, 6=EnergyFilterPileUpGuard, 7=EventPileUp,
    /// 8=ADCSaturation, 9=ADCSaturationProtection, A=PostSaturationEvent,
    /// B=EnergyFilterSaturation, C=SignalInhibit, 0xFF=`UNKNOWN_PROBE_TYPE`.
    #[serde(default = "default_unknown_digital_probe_types")]
    pub digital_probe_type: [u8; 4],
}

impl Default for Waveform {
    fn default() -> Self {
        Self {
            analog_probe1: Vec::new(),
            analog_probe2: Vec::new(),
            digital_probe1: Vec::new(),
            digital_probe2: Vec::new(),
            digital_probe3: Vec::new(),
            digital_probe4: Vec::new(),
            time_resolution: 0,
            trigger_threshold: 0,
            ns_per_sample: 0.0,
            analog_probe1_is_signed: false,
            analog_probe2_is_signed: false,
            analog_probe_type: [UNKNOWN_PROBE_TYPE; 2],
            digital_probe_type: [UNKNOWN_PROBE_TYPE; 4],
        }
    }
}

/// Sign-extend a 14-bit two's complement value to i16.
/// Uses arithmetic shift: left-shift bit 13 to sign position, then right-shift back.
/// Upper bits beyond bit 13 are masked off.
#[inline]
pub fn sign_extend_14bit(value: u32) -> i16 {
    ((value << 18) as i32 >> 18) as i16
}

/// Event data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    /// Timestamp in nanoseconds
    pub timestamp_ns: f64,
    /// Module ID (digitizer number)
    pub module: u8,
    /// Channel number (0-127 for PSD2)
    pub channel: u8,
    /// Energy (long gate integral)
    pub energy: u16,
    /// Energy short (short gate integral)
    pub energy_short: u16,
    /// Fine timestamp (0-1023, /1024 scale)
    pub fine_time: u16,
    /// Flags (high priority + low priority)
    pub flags: u32,
    /// Per-event user info slots (AMax: 4 × u64 from OpenDPP user words).
    /// Slot 0 = AMax peak value (typical), 1 = baseline, 2-3 = FW-specific.
    /// All slots are 0 for non-AMax firmware. Fixed-size to avoid hot-path
    /// heap alloc; serde defaults to [0;4] when the field is absent in
    /// older `.delila` files.
    #[serde(default)]
    pub user_info: [u64; 4],
    /// Waveform data (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waveform: Option<Waveform>,
}

impl EventData {
    // Flag constants (PSD2 specific)
    pub const FLAG_PILEUP: u32 = 0x01;
    pub const FLAG_OVER_SATURATION: u32 = 0x02;
    pub const FLAG_NEGATIVE_OVER_SATURATION: u32 = 0x04;

    pub fn has_pileup(&self) -> bool {
        (self.flags & Self::FLAG_PILEUP) != 0
    }

    /// Format event data for display
    pub fn display(&self) -> String {
        format!(
            "Ch:{:3} T:{:15.3}ns E:{:5} Es:{:5} FT:{:4} F:0x{:05x}{}",
            self.channel,
            self.timestamp_ns,
            self.energy,
            self.energy_short,
            self.fine_time,
            self.flags,
            if self.waveform.is_some() { " [WF]" } else { "" }
        )
    }
}

impl Default for EventData {
    fn default() -> Self {
        Self {
            timestamp_ns: 0.0,
            module: 0,
            channel: 0,
            energy: 0,
            energy_short: 0,
            fine_time: 0,
            flags: 0,
            user_info: [0; 4],
            waveform: None,
        }
    }
}

impl std::fmt::Display for EventData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_extend_14bit() {
        // Zero
        assert_eq!(sign_extend_14bit(0x0000), 0);
        // Maximum positive (bit 13 = 0)
        assert_eq!(sign_extend_14bit(0x1FFF), 8191);
        // Minimum negative (bit 13 = 1, rest 0)
        assert_eq!(sign_extend_14bit(0x2000), -8192);
        // -1 (all 14 bits set)
        assert_eq!(sign_extend_14bit(0x3FFF), -1);
        // -2
        assert_eq!(sign_extend_14bit(0x3FFE), -2);
        // Small positive
        assert_eq!(sign_extend_14bit(0x0001), 1);
        assert_eq!(sign_extend_14bit(0x0064), 100);
        // Upper bits beyond 14 are masked off
        assert_eq!(sign_extend_14bit(0xFFFF_C001), 1); // only lower 14 bits matter
        assert_eq!(sign_extend_14bit(0x4000), 0); // bit 14 set, but masked to 14 bits → 0
    }
}

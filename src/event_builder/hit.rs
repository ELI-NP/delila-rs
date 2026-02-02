//! Hit data structure for event building
//!
//! Corresponds to ELIFANT-Event's RawData_t

use crate::common::EventData;
use serde::{Deserialize, Serialize};

/// A single detector hit for event building
///
/// This is the internal representation used during event building.
/// Converted from delila-rs EventData or read from ROOT TTree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    /// Module ID (0-255, typically 0-10)
    pub module: u8,
    /// Channel ID (0-255, typically 0-15 or 0-31)
    pub channel: u8,
    /// Energy (long gate integration, ADC units)
    pub energy: u16,
    /// Energy short (short gate, for PSD)
    pub energy_short: u16,
    /// Timestamp in nanoseconds (time-calibrated if offsets applied)
    pub timestamp_ns: f64,
    /// AC coincidence flag (set during event building)
    pub with_ac: bool,
}

impl Hit {
    /// Create from delila-rs EventData
    ///
    /// # Example
    /// ```ignore
    /// let event_data = EventData { ... };
    /// let hit = Hit::from_event_data(&event_data);
    /// ```
    #[inline]
    pub fn from_event_data(event: &EventData) -> Self {
        Self {
            module: event.module,
            channel: event.channel,
            energy: event.energy,
            energy_short: event.energy_short,
            timestamp_ns: event.timestamp_ns,
            with_ac: false,
        }
    }

    /// Create a new Hit with the given parameters
    #[inline]
    pub fn new(module: u8, channel: u8, energy: u16, energy_short: u16, timestamp_ns: f64) -> Self {
        Self {
            module,
            channel,
            energy,
            energy_short,
            timestamp_ns,
            with_ac: false,
        }
    }

    /// Apply time offset calibration
    ///
    /// Subtracts the offset from the timestamp (positive offset means
    /// the channel is ahead of the reference, so we subtract to align).
    #[inline]
    pub fn apply_offset(&mut self, offset_ns: f64) {
        self.timestamp_ns -= offset_ns;
    }

    /// Get channel key for lookup (module << 8 | channel)
    ///
    /// This provides a unique identifier for each channel that can be
    /// used as a hash map key.
    #[inline]
    pub fn channel_key(&self) -> u16 {
        ((self.module as u16) << 8) | (self.channel as u16)
    }

    /// Create channel key from module and channel
    #[inline]
    pub fn make_channel_key(module: u8, channel: u8) -> u16 {
        ((module as u16) << 8) | (channel as u16)
    }
}

impl Default for Hit {
    fn default() -> Self {
        Self {
            module: 0,
            channel: 0,
            energy: 0,
            energy_short: 0,
            timestamp_ns: 0.0,
            with_ac: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hit_new() {
        let hit = Hit::new(1, 2, 1000, 500, 12345.0);
        assert_eq!(hit.module, 1);
        assert_eq!(hit.channel, 2);
        assert_eq!(hit.energy, 1000);
        assert_eq!(hit.energy_short, 500);
        assert_eq!(hit.timestamp_ns, 12345.0);
        assert!(!hit.with_ac);
    }

    #[test]
    fn test_hit_apply_offset() {
        let mut hit = Hit::new(0, 0, 0, 0, 1000.0);
        hit.apply_offset(100.0);
        assert_eq!(hit.timestamp_ns, 900.0);

        hit.apply_offset(-50.0);
        assert_eq!(hit.timestamp_ns, 950.0);
    }

    #[test]
    fn test_hit_channel_key() {
        let hit = Hit::new(1, 5, 0, 0, 0.0);
        assert_eq!(hit.channel_key(), (1 << 8) | 5);
        assert_eq!(hit.channel_key(), 261);

        let hit2 = Hit::new(10, 31, 0, 0, 0.0);
        assert_eq!(hit2.channel_key(), (10 << 8) | 31);
        assert_eq!(hit2.channel_key(), 2591);
    }

    #[test]
    fn test_make_channel_key() {
        assert_eq!(Hit::make_channel_key(0, 0), 0);
        assert_eq!(Hit::make_channel_key(1, 0), 256);
        assert_eq!(Hit::make_channel_key(0, 1), 1);
        assert_eq!(Hit::make_channel_key(1, 1), 257);
    }

    #[test]
    fn test_hit_default() {
        let hit = Hit::default();
        assert_eq!(hit.module, 0);
        assert_eq!(hit.channel, 0);
        assert_eq!(hit.energy, 0);
        assert_eq!(hit.timestamp_ns, 0.0);
    }
}

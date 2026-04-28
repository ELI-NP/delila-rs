//! 2D plot axis sources.
//!
//! `AxisSource` enumerates the per-event quantities that can be plotted on
//! either the X or Y axis of a 2D histogram. Each axis source knows how to
//! extract its scalar value from an `EventData` plus the natural histogram
//! range/bin count for use as a default when a plot is created on demand.
//!
//! Adding a new axis means: extend the enum, add a match arm in `extract`,
//! `default_axis`, and `label`. Frontend `histogram.types.ts` mirrors the
//! string variants 1:1 (snake_case via serde).
//!
//! `Psd` is a derived value, not a raw field — `(energy - energy_short) /
//! energy`, undefined when energy == 0.
//!
//! `fine_time` is intentionally absent: it is folded into `timestamp_ns`
//! inside the reader before the event hits the pipeline, so the Monitor
//! never sees it as a separate field.

use crate::common::EventData;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Source of a 2D histogram axis. Variants serialize as snake_case strings
/// (`"energy"`, `"energy_short"`, `"user_info0".."user_info3"`, `"psd"`) for
/// use in REST query parameters and TS code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AxisSource {
    Energy,
    EnergyShort,
    UserInfo0,
    UserInfo1,
    UserInfo2,
    UserInfo3,
    /// Pulse-shape discrimination ratio = (energy - energy_short) / energy.
    /// Undefined (returns `None` from `extract`) when `energy == 0`.
    Psd,
}

impl AxisSource {
    /// All variants, in the order they should appear in UI dropdowns.
    pub const ALL: [AxisSource; 7] = [
        AxisSource::Energy,
        AxisSource::EnergyShort,
        AxisSource::UserInfo0,
        AxisSource::UserInfo1,
        AxisSource::UserInfo2,
        AxisSource::UserInfo3,
        AxisSource::Psd,
    ];

    /// Extract the scalar value for this axis from an event.
    /// Returns `None` for derived quantities that are undefined for the event
    /// (currently only `Psd` when `energy == 0`).
    pub fn extract(self, event: &EventData) -> Option<f64> {
        match self {
            AxisSource::Energy => Some(event.energy as f64),
            AxisSource::EnergyShort => Some(event.energy_short as f64),
            AxisSource::UserInfo0 => Some(event.user_info[0] as f64),
            AxisSource::UserInfo1 => Some(event.user_info[1] as f64),
            AxisSource::UserInfo2 => Some(event.user_info[2] as f64),
            AxisSource::UserInfo3 => Some(event.user_info[3] as f64),
            AxisSource::Psd => {
                if event.energy == 0 {
                    None
                } else {
                    Some(
                        (event.energy as f64 - event.energy_short as f64)
                            / event.energy as f64,
                    )
                }
            }
        }
    }

    /// Sensible default histogram range and bin count for this axis.
    /// Returns `(min, max, num_bins)`.
    pub fn default_axis(self) -> (f32, f32, u32) {
        match self {
            // 16-bit raw ADC counts.
            AxisSource::Energy | AxisSource::EnergyShort => (0.0, 65536.0, 512),
            // amax_viewer convention (matches Phase 1 amax2d default).
            AxisSource::UserInfo0
            | AxisSource::UserInfo1
            | AxisSource::UserInfo2
            | AxisSource::UserInfo3 => (0.0, 16384.0, 512),
            // Psd ratio is bounded [0, 1] in theory but noise pushes events
            // slightly outside both ends — we keep the same -0.2..1.2 / 200-bin
            // window the legacy psd2d_y_config used so existing users see no
            // visible change.
            AxisSource::Psd => (-0.2, 1.2, 200),
        }
    }

    /// Human-readable label for chart axis title / tooltip.
    pub fn label(self) -> &'static str {
        match self {
            AxisSource::Energy => "Energy",
            AxisSource::EnergyShort => "Energy Short",
            AxisSource::UserInfo0 => "UserInfo[0]",
            AxisSource::UserInfo1 => "UserInfo[1]",
            AxisSource::UserInfo2 => "UserInfo[2]",
            AxisSource::UserInfo3 => "UserInfo[3]",
            AxisSource::Psd => "PSD",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::EventData;

    fn event(energy: u16, energy_short: u16, user: [u64; 4]) -> EventData {
        EventData {
            timestamp_ns: 0.0,
            module: 0,
            channel: 0,
            energy,
            energy_short,
            flags: 0,
            user_info: user,
            waveform: None,
        }
    }

    #[test]
    fn extract_basic_axes() {
        let e = event(1000, 200, [123, 456, 789, 1000]);
        assert_eq!(AxisSource::Energy.extract(&e), Some(1000.0));
        assert_eq!(AxisSource::EnergyShort.extract(&e), Some(200.0));
        assert_eq!(AxisSource::UserInfo0.extract(&e), Some(123.0));
        assert_eq!(AxisSource::UserInfo1.extract(&e), Some(456.0));
        assert_eq!(AxisSource::UserInfo2.extract(&e), Some(789.0));
        assert_eq!(AxisSource::UserInfo3.extract(&e), Some(1000.0));
    }

    #[test]
    fn psd_is_undefined_when_energy_is_zero() {
        let e = event(0, 0, [0; 4]);
        assert_eq!(AxisSource::Psd.extract(&e), None);
    }

    #[test]
    fn psd_ratio_matches_definition() {
        // (1000 - 200) / 1000 = 0.8
        let e = event(1000, 200, [0; 4]);
        let psd = AxisSource::Psd.extract(&e).expect("psd defined");
        assert!((psd - 0.8).abs() < 1e-12, "psd = {}", psd);
    }

    #[test]
    fn default_axes_are_nonempty() {
        for src in AxisSource::ALL {
            let (min, max, bins) = src.default_axis();
            assert!(max > min, "{:?}: max <= min ({}, {})", src, min, max);
            assert!(bins > 0, "{:?}: bins == 0", src);
        }
    }

    #[test]
    fn serde_uses_snake_case_strings() {
        // round-trip through JSON.
        for src in AxisSource::ALL {
            let json = serde_json::to_string(&src).unwrap();
            let back: AxisSource = serde_json::from_str(&json).unwrap();
            assert_eq!(src, back);
        }
        // Spot-check the wire format.
        assert_eq!(serde_json::to_string(&AxisSource::UserInfo0).unwrap(), "\"user_info0\"");
        assert_eq!(serde_json::to_string(&AxisSource::EnergyShort).unwrap(), "\"energy_short\"");
        assert_eq!(serde_json::to_string(&AxisSource::Psd).unwrap(), "\"psd\"");
    }
}

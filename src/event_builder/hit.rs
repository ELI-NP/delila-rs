//! Hit data structures for the event builder pipeline.
//!
//! Two concrete types exist:
//! - [`OnlineHit`] (16 B with Rust field reordering): minimal fields used on
//!   the live ZMQ path. Waveform / `flags` / `user_info` are dropped — raw
//!   `.delila` is the safety net for offline reanalysis.
//! - [`OfflineHit`] (56 B): adds `flags` and `user_info[4]` for offline
//!   reanalysis use cases.
//!
//! Both implement [`HitLike`], so pipeline stages (chunk_builder, time_sort,
//! slice_builder, ...) can stay generic and not care which concrete type
//! is flowing through. Only output stages need to branch.
//!
//! `Hit` is retained as an alias of [`OnlineHit`] for backwards compatibility
//! with the rest of the EB pipeline (not yet generic over `HitLike` — see
//! SPEC § 11.4 Phase 4/5).

use crate::common::EventData;
use serde::{Deserialize, Serialize};

/// Common read-only surface implemented by both [`OnlineHit`] and [`OfflineHit`].
///
/// Pipeline core stages (`chunk_builder`, `time_sort`, `slice_builder`)
/// only need these accessors; concrete-type fields stay invisible to them.
pub trait HitLike {
    fn module(&self) -> u8;
    fn channel(&self) -> u8;
    fn energy(&self) -> u16;
    fn energy_short(&self) -> u16;
    fn timestamp_ns(&self) -> f64;
    fn with_ac(&self) -> bool;

    /// `(module << 8) | channel` — a u16 lookup key for per-channel maps.
    #[inline]
    fn channel_key(&self) -> u16 {
        ((self.module() as u16) << 8) | (self.channel() as u16)
    }
}

/// Lean hit used on the online (ZMQ) event-builder path.
///
/// `flags`, `user_info`, and waveforms from the upstream `EventData` are
/// dropped at ingress (see SPEC § 2.1). Raw `.delila` files persist the
/// full record for any offline analysis that needs them.
///
/// Memory layout: 16 B (Rust's default field reordering packs the small
/// fields after the `f64`, so this is tighter than the SPEC's 24 B estimate
/// which assumed C-style declaration order).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OnlineHit {
    pub module: u8,
    pub channel: u8,
    pub energy: u16,
    pub energy_short: u16,
    pub timestamp_ns: f64,
    /// Set during event building when an AC veto channel fires alongside
    /// this hit's trigger anchor. Always `false` at ingress.
    pub with_ac: bool,
}

impl OnlineHit {
    /// Build an `OnlineHit` from an upstream `EventData`, dropping fields
    /// that the online EB does not consume.
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

    /// Subtract a per-channel time offset (see SPEC § 4.3).
    ///
    /// Convention: `aligned_ts = raw_ts - offset_ns`.
    #[inline]
    pub fn apply_offset(&mut self, offset_ns: f64) {
        self.timestamp_ns -= offset_ns;
    }

    /// Static helper for keys when only `(module, channel)` is in hand.
    #[inline]
    pub fn make_channel_key(module: u8, channel: u8) -> u16 {
        ((module as u16) << 8) | (channel as u16)
    }
}

impl Default for OnlineHit {
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

impl HitLike for OnlineHit {
    #[inline]
    fn module(&self) -> u8 {
        self.module
    }
    #[inline]
    fn channel(&self) -> u8 {
        self.channel
    }
    #[inline]
    fn energy(&self) -> u16 {
        self.energy
    }
    #[inline]
    fn energy_short(&self) -> u16 {
        self.energy_short
    }
    #[inline]
    fn timestamp_ns(&self) -> f64 {
        self.timestamp_ns
    }
    #[inline]
    fn with_ac(&self) -> bool {
        self.with_ac
    }
}

/// Backwards-compatible alias used by the rest of the EB pipeline until the
/// generic-over-`HitLike` refactor (SPEC § 11.4 Phase 4/5) lands.
pub type Hit = OnlineHit;

/// Rich hit used on the offline (`.delila` / ROOT) event-builder path.
///
/// Carries `flags` and `user_info[4]` so that downstream offline analyses
/// can read the AMax peak/baseline / FW-specific bits without having to
/// re-join with the raw `.delila` source (see SPEC § 2.2).
///
/// Memory layout: 56 B (Rust field reordering vs the SPEC's 64 B C-style
/// estimate).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OfflineHit {
    pub module: u8,
    pub channel: u8,
    pub energy: u16,
    pub energy_short: u16,
    pub timestamp_ns: f64,
    pub with_ac: bool,
    pub flags: u64,
    pub user_info: [u64; 4],
}

impl OfflineHit {
    /// Build an `OfflineHit` from an upstream `EventData`. Waveforms are
    /// still dropped — the offline EB does not need waveform samples in
    /// its hot path; analyses that want them re-read the raw `.delila`.
    #[inline]
    pub fn from_event_data(event: &EventData) -> Self {
        Self {
            module: event.module,
            channel: event.channel,
            energy: event.energy,
            energy_short: event.energy_short,
            timestamp_ns: event.timestamp_ns,
            with_ac: false,
            flags: event.flags,
            user_info: event.user_info,
        }
    }

    #[inline]
    pub fn apply_offset(&mut self, offset_ns: f64) {
        self.timestamp_ns -= offset_ns;
    }
}

impl Default for OfflineHit {
    fn default() -> Self {
        Self {
            module: 0,
            channel: 0,
            energy: 0,
            energy_short: 0,
            timestamp_ns: 0.0,
            with_ac: false,
            flags: 0,
            user_info: [0; 4],
        }
    }
}

impl HitLike for OfflineHit {
    #[inline]
    fn module(&self) -> u8 {
        self.module
    }
    #[inline]
    fn channel(&self) -> u8 {
        self.channel
    }
    #[inline]
    fn energy(&self) -> u16 {
        self.energy
    }
    #[inline]
    fn energy_short(&self) -> u16 {
        self.energy_short
    }
    #[inline]
    fn timestamp_ns(&self) -> f64 {
        self.timestamp_ns
    }
    #[inline]
    fn with_ac(&self) -> bool {
        self.with_ac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_hit_new() {
        let h = OnlineHit::new(1, 2, 1000, 500, 12345.0);
        assert_eq!(h.module, 1);
        assert_eq!(h.channel, 2);
        assert_eq!(h.energy, 1000);
        assert_eq!(h.energy_short, 500);
        assert_eq!(h.timestamp_ns, 12345.0);
        assert!(!h.with_ac);
    }

    #[test]
    fn online_hit_apply_offset() {
        let mut h = OnlineHit::new(0, 0, 0, 0, 1000.0);
        h.apply_offset(100.0);
        assert_eq!(h.timestamp_ns, 900.0);
        h.apply_offset(-50.0);
        assert_eq!(h.timestamp_ns, 950.0);
    }

    #[test]
    fn online_hit_channel_key() {
        let h = OnlineHit::new(1, 5, 0, 0, 0.0);
        assert_eq!(h.channel_key(), (1u16 << 8) | 5);
        let h2 = OnlineHit::new(10, 31, 0, 0, 0.0);
        assert_eq!(h2.channel_key(), (10u16 << 8) | 31);
    }

    #[test]
    fn make_channel_key_static() {
        assert_eq!(OnlineHit::make_channel_key(0, 0), 0);
        assert_eq!(OnlineHit::make_channel_key(1, 0), 256);
        assert_eq!(OnlineHit::make_channel_key(0, 1), 1);
        assert_eq!(OnlineHit::make_channel_key(1, 1), 257);
    }

    #[test]
    fn online_hit_default() {
        let h = OnlineHit::default();
        assert_eq!(h.module, 0);
        assert_eq!(h.timestamp_ns, 0.0);
    }

    #[test]
    fn online_hit_from_event_data_drops_extras() {
        let mut ev = EventData::new(2, 3, 1234, 56, 9999.0, 0xABCD);
        ev.user_info = [1, 2, 3, 4];
        let h = OnlineHit::from_event_data(&ev);
        assert_eq!(h.module, 2);
        assert_eq!(h.channel, 3);
        assert_eq!(h.energy, 1234);
        assert_eq!(h.energy_short, 56);
        assert_eq!(h.timestamp_ns, 9999.0);
        assert!(!h.with_ac);
        // flags/user_info are not present on OnlineHit, by design.
    }

    #[test]
    fn offline_hit_carries_flags_and_user_info() {
        let mut ev = EventData::new(2, 3, 1234, 56, 9999.0, 0xABCD);
        ev.user_info = [11, 22, 33, 44];
        let h = OfflineHit::from_event_data(&ev);
        assert_eq!(h.module, 2);
        assert_eq!(h.channel, 3);
        assert_eq!(h.timestamp_ns, 9999.0);
        assert_eq!(h.flags, 0xABCD);
        assert_eq!(h.user_info, [11, 22, 33, 44]);
        assert!(!h.with_ac);
    }

    #[test]
    fn offline_hit_apply_offset() {
        let mut h = OfflineHit::from_event_data(&EventData::new(0, 0, 0, 0, 1000.0, 0));
        h.apply_offset(250.0);
        assert_eq!(h.timestamp_ns, 750.0);
    }

    #[test]
    fn hitlike_trait_works_for_both() {
        let on = OnlineHit::new(1, 2, 100, 50, 500.0);
        let off = OfflineHit::from_event_data(&EventData::new(1, 2, 100, 50, 500.0, 0));
        fn check<H: HitLike>(h: &H) {
            assert_eq!(h.module(), 1);
            assert_eq!(h.channel(), 2);
            assert_eq!(h.energy(), 100);
            assert_eq!(h.energy_short(), 50);
            assert_eq!(h.timestamp_ns(), 500.0);
            assert_eq!(h.channel_key(), (1u16 << 8) | 2);
        }
        check(&on);
        check(&off);
    }

    #[test]
    fn online_hit_size_is_16() {
        // Rust reorders small fields after the f64, so the layout is tighter
        // than the SPEC's 24 B C-style estimate.
        assert_eq!(std::mem::size_of::<OnlineHit>(), 16);
    }

    #[test]
    fn offline_hit_size_is_56() {
        // Same reordering benefit as OnlineHit (SPEC's 64 B was a C-style estimate).
        assert_eq!(std::mem::size_of::<OfflineHit>(), 56);
    }
}

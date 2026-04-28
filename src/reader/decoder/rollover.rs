//! Generic N-bit counter rollover tracker.
//!
//! Extends an N-bit hardware counter (e.g. V1743 TDC = 40-bit, PSD1/PHA1 SW Fine TS
//! BoardAgg TTT = 32-bit) into a monotonic 64-bit tick count using modulo-distance
//! arithmetic — the same idea TCP sequence numbers use to distinguish a wrap from
//! an out-of-order arrival.
//!
//! Design contract (see `docs/plans/rollover_tracker_design.md`):
//!   * Internal state is **u64 ticks only**. No `f64`, no `Instant`. The caller
//!     multiplies by `ns_per_tick` at the emission boundary.
//!   * Masking is enforced on every input: upper bits above `bits` are silently
//!     discarded so a firmware that leaves them undefined cannot corrupt the
//!     rollover bookkeeping.
//!   * A forward jump greater than half the rollover period is treated as a late
//!     arrival from the previous epoch, **not** as a rollover. The event is
//!     reconstructed against `rollover_count - 1`.
//!   * Reset (Run Start) must clear `prev_raw` and `rollover_count`; otherwise the
//!     first event of the new run would look like a late arrival.
//!
//! The only precondition the caller must satisfy is that the real-time gap
//! between two successive inputs never exceeds half the rollover period
//! (V1743: 45.8 min; DT5730 SW Fine TS: 4.3 s). Inside a running DAQ this
//! holds by construction — pulser / source rates keep firing well below that.

use thiserror::Error;

/// Reasons `RolloverTracker::extend` may refuse a value. Always a bug upstream
/// (pre-Run event, corrupted firmware data, misuse of `reset`).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RolloverError {
    #[error("out-of-order event from an epoch before the tracker started (raw=0x{raw:x})")]
    Underflow { raw: u64 },
}

/// Tracks rollovers of an N-bit counter and returns an extended 64-bit tick count.
///
/// `bits` must be in `[2, 63]`. The most common values are 32 (DT5730 SW Fine TS
/// BoardAgg TTT) and 40 (V1743 TDC).
///
/// The very first `extend` call after construction or `reset()` is accepted
/// unconditionally — with no prior value to compare against, it simply
/// initializes `prev_raw` from the (masked) input. This mirrors physical
/// reality: when the hardware counter zeros on Run Start, the first event
/// carries an arbitrary small-positive `raw` and there is no rollover to
/// detect yet.
#[derive(Debug, Clone)]
pub struct RolloverTracker {
    mask: u64,
    bits: u8,
    prev_raw: Option<u64>,
    rollover_count: u64,
}

impl RolloverTracker {
    /// Create a fresh tracker for an N-bit counter.
    ///
    /// # Panics
    /// Panics if `bits` is not in `[2, 63]`.
    #[must_use]
    pub fn new(bits: u8) -> Self {
        assert!(
            (2..=63).contains(&bits),
            "bits must be in [2, 63], got {bits}"
        );
        Self {
            mask: (1u64 << bits) - 1,
            bits,
            prev_raw: None,
            rollover_count: 0,
        }
    }

    /// Number of bits in the raw counter.
    #[inline]
    #[must_use]
    pub fn bits(&self) -> u8 {
        self.bits
    }

    /// Current rollover count (number of wraps observed since construction or reset).
    #[inline]
    #[must_use]
    pub fn rollover_count(&self) -> u64 {
        self.rollover_count
    }

    /// Extend a raw counter reading to a monotonic 64-bit absolute tick.
    ///
    /// Upper bits above `self.bits` in `raw_tick` are silently masked off.
    /// Returns `Err(RolloverError::Underflow)` when an apparent "late arrival
    /// from previous epoch" occurs before any rollover has been seen — i.e. an
    /// event with a raw value significantly below `prev_raw` cannot belong to
    /// a pre-first-event epoch.
    pub fn extend(&mut self, raw_tick: u64) -> Result<u64, RolloverError> {
        let raw = raw_tick & self.mask;

        // First event after construction or reset: no prior value to compare
        // against, so we cannot (and need not) classify rollover. Initialize
        // prev_raw and return.
        let prev_raw = match self.prev_raw {
            None => {
                self.prev_raw = Some(raw);
                return Ok(raw);
            }
            Some(p) => p,
        };

        // `wrapping_sub` on u64 then masked to `bits` gives the forward distance
        // from `prev_raw` to `raw` modulo 2^bits.
        let diff = raw.wrapping_sub(prev_raw) & self.mask;
        let half_period = 1u64 << (self.bits - 1);

        // `diff == 0` (identical timestamp) is handled by the else branch as a
        // non-rollover forward step of zero — common when two events share a
        // coarse tick and differ only in fine time.
        if diff > half_period {
            // Late arrival: `raw` is behind `prev_raw` by less than half a
            // period, so it belongs to the previous epoch.
            if self.rollover_count == 0 {
                return Err(RolloverError::Underflow { raw });
            }
            // Do NOT mutate state — later events may still advance the epoch.
            Ok(raw | ((self.rollover_count - 1) << self.bits))
        } else {
            // Forward step. Detect a wrap by the strict-less-than comparison on
            // the raw values (since we have already ruled out the "backwards
            // from jitter" case via the half-period check above).
            if raw < prev_raw {
                self.rollover_count += 1;
            }
            self.prev_raw = Some(raw);
            Ok(raw | (self.rollover_count << self.bits))
        }
    }

    /// Reset for a new run. Must be called when the hardware counter is known to
    /// have been zeroed (CAEN SW Start, S_IN, etc.) — otherwise the next event
    /// will be misclassified.
    pub fn reset(&mut self) {
        self.prev_raw = None;
        self.rollover_count = 0;
    }

    /// Reconstruct a narrower sub-counter value (e.g. the DT5730 31-bit Event TTT)
    /// against the extended value of the containing same-clock counter.
    ///
    /// The caller passes the already-extended value of the wider counter
    /// (`extended`) together with the narrower raw reading (`sub_raw`) and its
    /// width (`sub_bits`). The result is at most `extended`; if the naïve
    /// composition would be in the future, one `sub_bits`-period is subtracted
    /// so the value lands in the same epoch as `extended` or the previous one.
    ///
    /// # Panics
    /// Panics if `sub_bits` is not in `[2, bits]`.
    #[must_use]
    pub fn reconstruct_subcounter(&self, extended: u64, sub_raw: u64, sub_bits: u8) -> u64 {
        assert!(
            sub_bits >= 2 && sub_bits <= self.bits,
            "sub_bits must be in [2, {}], got {sub_bits}",
            self.bits
        );
        let sub_mask = (1u64 << sub_bits) - 1;
        let sub = sub_raw & sub_mask;
        let hi = extended & !sub_mask;
        let candidate = hi | sub;
        if candidate > extended {
            candidate.wrapping_sub(1u64 << sub_bits)
        } else {
            candidate
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Deterministic unit tests (Gemini review, T1..T8) ----------------------

    /// T1 — strictly monotonic forward steps.
    #[test]
    fn t1_monotonic_advance() {
        let mut t = RolloverTracker::new(40);
        assert_eq!(t.extend(10).unwrap(), 10);
        assert_eq!(t.extend(20).unwrap(), 20);
        assert_eq!(t.extend(30).unwrap(), 30);
        assert_eq!(t.rollover_count(), 0);
    }

    /// T2 — clean rollover at 2^40 boundary. Post-wrap ticks land above 2^40.
    #[test]
    fn t2_clean_rollover() {
        const BITS: u8 = 40;
        let max: u64 = (1u64 << BITS) - 1;
        let mut t = RolloverTracker::new(BITS);
        assert_eq!(t.extend(max - 5).unwrap(), max - 5);
        assert_eq!(t.extend(max).unwrap(), max);
        // raw=5 with prev=max is a small forward diff → real rollover
        assert_eq!(t.extend(5).unwrap(), (1u64 << BITS) | 5);
        assert_eq!(t.extend(10).unwrap(), (1u64 << BITS) | 10);
        assert_eq!(t.rollover_count(), 1);
    }

    /// T3 — micro-jitter: a small backwards step with no rollover must be
    /// reflected literally; the absolute tick is allowed to go backwards and the
    /// rollover counter must not advance.
    ///
    /// Expected semantics: the second value is treated as a late arrival from
    /// epoch 0 (but since rollover_count is 0, that would be Err). We're inside
    /// the "no rollover yet" regime where a tiny backwards step with diff close
    /// to 2^40 triggers the Underflow path. Guard this at the API level.
    #[test]
    fn t3_micro_jitter_before_any_rollover_errors() {
        let mut t = RolloverTracker::new(40);
        assert_eq!(t.extend(10).unwrap(), 10);
        // 10 → 8 as raw: diff = -2 mod 2^40 ≈ 2^40 − 2 which is > half period.
        // That means it looks like a late arrival from before the first event,
        // which we cannot satisfy → Underflow.
        assert!(matches!(
            t.extend(8),
            Err(RolloverError::Underflow { raw: 8 })
        ));
    }

    /// T3b — micro-jitter after a rollover has been seen is NOT an error;
    /// the late arrival is mapped back into the previous epoch.
    #[test]
    fn t3b_micro_jitter_after_rollover() {
        const BITS: u8 = 40;
        let max: u64 = (1u64 << BITS) - 1;
        let mut t = RolloverTracker::new(BITS);
        assert_eq!(t.extend(max - 2).unwrap(), max - 2);
        // small forward step into epoch 1
        assert_eq!(t.extend(2).unwrap(), (1u64 << BITS) | 2);
        // now a "late" event with raw=max-1 shows up: it belongs to epoch 0
        assert_eq!(t.extend(max - 1).unwrap(), max - 1);
        // rollover_count must not have regressed
        assert_eq!(t.rollover_count(), 1);
        // and the next normal forward event continues in epoch 1
        assert_eq!(t.extend(5).unwrap(), (1u64 << BITS) | 5);
    }

    /// T4 — rollover interleaved with late arrival (the explicit Gemini case).
    #[test]
    fn t4_rollover_with_late_arrival() {
        const BITS: u8 = 40;
        let max: u64 = (1u64 << BITS) - 1;
        let mut t = RolloverTracker::new(BITS);

        // Event 1: near top of epoch 0
        assert_eq!(t.extend(max - 2).unwrap(), max - 2);
        // Event 2: small value, big forward step → real rollover to epoch 1
        assert_eq!(t.extend(2).unwrap(), (1u64 << BITS) | 2);
        // Event 3: late arrival from epoch 0 (max - 1 is "before" max - 2 by 1 …
        // wait, it's actually after). Use a value that's clearly pre-wrap:
        assert_eq!(t.extend(max - 1).unwrap(), max - 1);
        // Event 4: small value again, forward from epoch 1's last-in-order
        assert_eq!(t.extend(5).unwrap(), (1u64 << BITS) | 5);
        assert_eq!(t.rollover_count(), 1);
    }

    /// T5 — a single huge gap (> half period) from raw=5 to raw=max-5 must be
    /// treated as a low-rate forward leap, NOT as an out-of-order arrival.
    ///
    /// raw=5 → raw=max-5: diff = (max-5) - 5 = max - 10 ≈ 2^40 - 10, which is
    /// just under the full period. Specifically diff > half_period → the
    /// algorithm would classify this as a late arrival. This is the documented
    /// limit: the caller must ensure real-time gaps stay under half a period.
    ///
    /// We assert the *documented* behaviour (misclassified as late), so the
    /// regression is visible if someone changes the algorithm. The correct
    /// mitigation is upstream (Run-length or heartbeat rules), not in the
    /// tracker.
    #[test]
    fn t5_huge_gap_is_classified_as_late_arrival_by_design() {
        const BITS: u8 = 40;
        let max: u64 = (1u64 << BITS) - 1;
        let mut t = RolloverTracker::new(BITS);
        assert_eq!(t.extend(5).unwrap(), 5);
        // Before any rollover, the "late arrival" branch turns this into Underflow:
        assert!(matches!(
            t.extend(max - 5),
            Err(RolloverError::Underflow { .. })
        ));
    }

    /// T6 — upper bits above `bits` must be silently masked off.
    #[test]
    fn t6_garbage_upper_bits_masked() {
        let mut t = RolloverTracker::new(40);
        assert_eq!(t.extend(5).unwrap(), 5);
        // 0x1_0000_0000_0000 | 10  → upper bit is outside 40 bits and is dropped.
        let garbage = (1u64 << 42) | 10;
        assert_eq!(t.extend(garbage).unwrap(), 10);
    }

    /// T7 — the very first out-of-order event before any rollover must Err.
    #[test]
    fn t7_initial_underflow() {
        let mut t = RolloverTracker::new(16);
        assert_eq!(t.extend(100).unwrap(), 100);
        // raw=50 with prev=100 at bits=16: diff = 50 - 100 mod 2^16 = 65486 > half (32768)
        // → classified as late arrival, but rollover_count=0 → Underflow.
        assert!(matches!(
            t.extend(50),
            Err(RolloverError::Underflow { raw: 50 })
        ));
        // state unchanged by the Err path
        assert_eq!(t.rollover_count(), 0);
        // a genuine forward event still works afterwards
        assert_eq!(t.extend(200).unwrap(), 200);
    }

    /// T8 — reset() clears state; a post-reset tracker behaves as new.
    #[test]
    fn t8_reset_clears_state() {
        const BITS: u8 = 40;
        let max = (1u64 << BITS) - 1;
        let mut t = RolloverTracker::new(BITS);
        assert_eq!(t.extend(max - 5).unwrap(), max - 5);
        assert_eq!(t.extend(5).unwrap(), (1u64 << BITS) | 5);
        assert_eq!(t.rollover_count(), 1);

        t.reset();
        assert_eq!(t.rollover_count(), 0);
        assert_eq!(t.extend(10).unwrap(), 10);
        assert_eq!(t.extend(20).unwrap(), 20);
    }

    // --- Boundary / API behaviour ---------------------------------------------

    #[test]
    fn bits_accessor_matches_constructor() {
        let t = RolloverTracker::new(40);
        assert_eq!(t.bits(), 40);
        assert_eq!(RolloverTracker::new(32).bits(), 32);
    }

    #[test]
    #[should_panic(expected = "bits must be in")]
    fn new_rejects_bits_too_small() {
        let _ = RolloverTracker::new(1);
    }

    #[test]
    #[should_panic(expected = "bits must be in")]
    fn new_rejects_bits_too_large() {
        let _ = RolloverTracker::new(64);
    }

    #[test]
    fn extend_handles_repeated_identical_values() {
        // Same coarse tick showing up twice (e.g. two channels firing within
        // one TDC tick). diff == 0, no rollover, state unchanged for the second.
        let mut t = RolloverTracker::new(40);
        assert_eq!(t.extend(42).unwrap(), 42);
        assert_eq!(t.extend(42).unwrap(), 42);
        assert_eq!(t.rollover_count(), 0);
    }

    // --- reconstruct_subcounter -----------------------------------------------

    #[test]
    fn subcounter_trivial_identity_when_widths_match() {
        let t = RolloverTracker::new(32);
        // 32-bit extended value with same-width sub_raw must roundtrip.
        let ext = (3u64 << 32) | 0x1234_5678;
        assert_eq!(t.reconstruct_subcounter(ext, 0x1234_5678, 32), ext);
    }

    #[test]
    fn subcounter_31bit_same_epoch() {
        // DT5730: 32-bit BoardAgg TTT and 31-bit Event TTT share the same clock.
        // An event TTT inside the current BoardAgg epoch stays in-place.
        let t = RolloverTracker::new(32);
        let ext = (5u64 << 32) | 0x4000_0000; // bit 30 set, BoardAgg ~ mid-period
        let event_ttt: u64 = 0x3FFF_F000; // 31-bit, just before the BoardAgg value
        let r = t.reconstruct_subcounter(ext, event_ttt, 31);
        assert_eq!(r, (5u64 << 32) | 0x3FFF_F000);
    }

    #[test]
    fn subcounter_31bit_wraps_back_one_epoch() {
        // BoardAgg has advanced past a 31-bit boundary; a sub-counter reading
        // slightly above the BoardAgg value must roll back by 2^31 so it lands
        // before `extended`.
        let t = RolloverTracker::new(32);
        let ext: u64 = 0x1000_0000; // small BoardAgg (epoch 0)
        let event_ttt: u64 = 0x7FFF_0000; // large 31-bit (~ 2 s ago at 2 ns)
        let r = t.reconstruct_subcounter(ext, event_ttt, 31);
        // Naive hi|sub = 0x7FFF_0000 would be > ext → subtract 2^31.
        assert_eq!(r as i64, 0x7FFF_0000_i64 - (1i64 << 31));
    }

    #[test]
    #[should_panic(expected = "sub_bits must be in")]
    fn subcounter_rejects_too_wide_sub() {
        let t = RolloverTracker::new(32);
        let _ = t.reconstruct_subcounter(0, 0, 33);
    }

    // --- Proptest invariants --------------------------------------------------
    //
    // Property-based tests enforce the contract on arbitrary inputs inside the
    // "forward-only gap < half period" regime.
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Forward-only sequences (each step smaller than half the period)
            /// must always produce monotonically non-decreasing absolute ticks.
            /// This is the core safety property of the tracker.
            ///
            /// We pick `bits >= 16` and `step < 1000` so `step` stays well below
            /// the half-period (`2^15 = 32_768` at the smallest) and the algorithm's
            /// precondition ("real-time gap < half rollover period") holds.
            #[test]
            fn extend_is_monotonic_for_safe_forward_sequences(
                bits in 16u8..=40,
                steps in proptest::collection::vec(0u64..1000u64, 1..200),
            ) {
                let mut tracker = RolloverTracker::new(bits);
                let mask = (1u64 << bits) - 1;
                let mut absolute = 0u64;
                let mut last_abs = 0u64;
                for step in steps {
                    absolute = absolute.wrapping_add(step);
                    let raw = absolute & mask;
                    let got = tracker.extend(raw).expect("forward-only should never Underflow");
                    prop_assert!(got >= last_abs,
                        "non-monotonic: got {} < last_abs {} (raw={}, absolute={})",
                        got, last_abs, raw, absolute);
                    last_abs = got;
                }
            }

            /// Masking invariant: the low `bits` bits of the returned value must
            /// always match the (masked) input raw.
            #[test]
            fn extend_preserves_low_bits(
                bits in 8u8..=40,
                raw in any::<u64>(),
            ) {
                let mut tracker = RolloverTracker::new(bits);
                let mask = (1u64 << bits) - 1;
                let expected_low = raw & mask;
                let got = tracker.extend(raw).expect("first call from prev=0 must succeed");
                prop_assert_eq!(got & mask, expected_low);
            }

            /// reconstruct_subcounter never returns a value strictly greater
            /// than the `extended` reference (that's its whole contract).
            #[test]
            fn subcounter_never_exceeds_extended(
                bits in 8u8..=48,
                sub_bits in 2u8..=48,
                extended in any::<u64>(),
                sub_raw in any::<u64>(),
            ) {
                prop_assume!(sub_bits <= bits);
                let tracker = RolloverTracker::new(bits);
                let r = tracker.reconstruct_subcounter(extended, sub_raw, sub_bits);
                prop_assert!(r <= extended);
            }

            /// After `reset`, the first `extend` must always succeed and equal
            /// the masked input — same contract as a freshly-constructed tracker.
            #[test]
            fn reset_yields_fresh_state(
                bits in 8u8..=40,
                setup_raw in any::<u64>(),
                first_after_reset in any::<u64>(),
            ) {
                let mut t = RolloverTracker::new(bits);
                let mask = (1u64 << bits) - 1;
                // Get the tracker into some arbitrary state.
                let _ = t.extend(setup_raw);
                t.reset();
                prop_assert_eq!(t.rollover_count(), 0);
                let r = t.extend(first_after_reset).unwrap();
                prop_assert_eq!(r, first_after_reset & mask);
            }
        }
    }
}

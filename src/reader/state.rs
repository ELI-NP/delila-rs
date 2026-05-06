//! Reader state-machine helper functions.
//!
//! Extracted from `reader/mod.rs` (R-D4, Phase 1 refactor sprint 2026-Q2).
//! Pure functions only — no async, no I/O, no shared state. Each helper is
//! a leaf utility used by the read loops to:
//! - rank component states for ordering (`state_rank`),
//! - decide what state to surface on `GetStatus` given software/hardware
//!   skew (`effective_state_for`),
//! - compute the next reconnect cooldown with exponential backoff + jitter
//!   (`next_reconnect_cooldown`).

use crate::common::ComponentState;
use rand::Rng;
use std::time::Duration;

/// Map [`ComponentState`] to a rank for ordering comparisons.
/// Transitional/Error states map to 0 (treated as Idle).
pub(crate) fn state_rank(s: ComponentState) -> u8 {
    match s {
        ComponentState::Idle => 0,
        ComponentState::Configured => 1,
        ComponentState::Armed => 2,
        ComponentState::Running => 3,
        _ => 0,
    }
}

/// Decide which state to report to the operator on `GetStatus`.
///
/// `software_state` is the operator-facing state, set immediately by
/// `handle_command` when a command is accepted — i.e. before the
/// hardware has actually moved. `hw_state` is the hardware-confirmed
/// state, updated by the read_loop only after the underlying FELib
/// transition (configure_endpoint / arm / start / disarm+drain+cleardata
/// / reset) actually completes.
///
/// We always trust `hw_state`. Lying about progress in either direction
/// lets the operator race in-flight transitions:
///
///   * **Up direction** (Configure / Arm / Start) — sw advances first,
///     hw catches up. Reporting sw too early would let the operator
///     fire the next command (e.g. Start before Arm finished) against
///     a hardware that isn't ready yet.
///   * **Down direction** (Stop / Reset) — sw drops first, hw lags by
///     the disarm / drain / cleardata window (~hundreds of ms). The
///     classic failure is rapid `Stop → Apply` in Tune Up: the previous
///     `state_rank(hw) < state_rank(sw)` heuristic only handled the up
///     direction, so during Stop it lied with `Configured` while the
///     FELib was still mid-disarm. The next `set_value` then locked
///     the handle at CAEN -15 (COMMUNICATION ERROR), recoverable only
///     by dropping and re-Open-ing the handle. See memory
///     `felib_stuck_after_rapid_stop_apply` for the 2026-05-04 PHA2
///     incident.
///
/// Returning `hw_state` unconditionally collapses both cases into the
/// same simple rule.
pub(crate) fn effective_state_for(
    _software_state: ComponentState,
    hw_state: ComponentState,
) -> ComponentState {
    hw_state
}

/// Reconnection backoff parameters.
/// Exponential backoff (1s→2s→4s→8s→16s→max 30s) + random jitter (±500ms)
/// prevents Thundering Herd when multiple readers reconnect simultaneously
/// after an optical link failure.
pub(crate) const RECONNECT_INITIAL: Duration = Duration::from_millis(1000);
pub(crate) const RECONNECT_MAX: Duration = Duration::from_millis(30000);
pub(crate) const RECONNECT_JITTER_MS: u64 = 500;

/// Compute next reconnect cooldown with exponential backoff + jitter.
/// Returns the jittered cooldown and the next (doubled) base for the caller to store.
pub(crate) fn next_reconnect_cooldown(current_base: Duration) -> (Duration, Duration) {
    let jitter_ms = rand::thread_rng().gen_range(0..=RECONNECT_JITTER_MS * 2);
    let jittered = current_base
        .checked_add(Duration::from_millis(jitter_ms))
        .unwrap_or(RECONNECT_MAX)
        .min(RECONNECT_MAX + Duration::from_millis(RECONNECT_JITTER_MS));
    let next_base = (current_base * 2).min(RECONNECT_MAX);
    (jittered, next_base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: GetStatus must report the *hardware*-confirmed state in
    /// both transition directions, not just the upward (Idle → Running)
    /// direction the original `state_rank(hw) < state_rank(sw)` heuristic
    /// covered.
    ///
    /// The 2026-05-04 PHA2 incident (memory:
    /// `felib_stuck_after_rapid_stop_apply`) was Stop → rapid Apply: sw
    /// dropped to Configured immediately, hw was still Running mid-disarm,
    /// and the old code reported Configured → operator's
    /// `wait_for_state(Configured)` returned in 263 µs → Apply ran during
    /// disarm/drain → FELib permanent CAEN -15.
    #[test]
    fn effective_state_for_stop_reports_running_until_hw_settles() {
        assert_eq!(
            effective_state_for(ComponentState::Configured, ComponentState::Running),
            ComponentState::Running,
        );
    }

    #[test]
    fn effective_state_for_reset_reports_running_until_hw_settles() {
        assert_eq!(
            effective_state_for(ComponentState::Idle, ComponentState::Running),
            ComponentState::Running,
        );
        assert_eq!(
            effective_state_for(ComponentState::Idle, ComponentState::Armed),
            ComponentState::Armed,
        );
    }

    #[test]
    fn effective_state_for_configure_reports_idle_until_hw_settles() {
        assert_eq!(
            effective_state_for(ComponentState::Configured, ComponentState::Idle),
            ComponentState::Idle,
        );
    }

    #[test]
    fn effective_state_for_arm_and_start_report_lower_until_hw_settles() {
        assert_eq!(
            effective_state_for(ComponentState::Armed, ComponentState::Configured),
            ComponentState::Configured,
        );
        assert_eq!(
            effective_state_for(ComponentState::Running, ComponentState::Armed),
            ComponentState::Armed,
        );
    }

    #[test]
    fn effective_state_for_settled_states_pass_through() {
        for s in [
            ComponentState::Idle,
            ComponentState::Configured,
            ComponentState::Armed,
            ComponentState::Running,
        ] {
            assert_eq!(effective_state_for(s, s), s);
        }
    }

    #[test]
    fn effective_state_for_error_hw_is_reported_through() {
        // If the read_loop ever flips hw_state to Error mid-flight, GetStatus
        // must surface that — never mask it behind the operator-facing sw.
        assert_eq!(
            effective_state_for(ComponentState::Running, ComponentState::Error),
            ComponentState::Error,
        );
    }

    #[test]
    fn state_rank_orders_lifecycle_states_monotonically() {
        assert!(state_rank(ComponentState::Idle) < state_rank(ComponentState::Configured));
        assert!(state_rank(ComponentState::Configured) < state_rank(ComponentState::Armed));
        assert!(state_rank(ComponentState::Armed) < state_rank(ComponentState::Running));
    }

    #[test]
    fn state_rank_collapses_transient_states_to_zero() {
        // Error / transitional states must rank as Idle so the read_loop
        // doesn't try to skip past them when comparing target vs. current.
        assert_eq!(state_rank(ComponentState::Error), 0);
    }

    #[test]
    fn next_reconnect_cooldown_doubles_base_until_max() {
        let (_, next) = next_reconnect_cooldown(RECONNECT_INITIAL);
        assert_eq!(next, RECONNECT_INITIAL * 2);

        // 30s is RECONNECT_MAX. Doubling 30s caps at 30s.
        let (_, next_at_cap) = next_reconnect_cooldown(RECONNECT_MAX);
        assert_eq!(next_at_cap, RECONNECT_MAX);
    }

    #[test]
    fn next_reconnect_cooldown_applies_jitter_within_bound() {
        // Jittered value is in [base, base + 2*JITTER_MS]; cap at MAX + JITTER_MS.
        for _ in 0..32 {
            let (jittered, _) = next_reconnect_cooldown(Duration::from_millis(1000));
            assert!(jittered >= Duration::from_millis(1000));
            assert!(jittered <= Duration::from_millis(1000 + 2 * RECONNECT_JITTER_MS));
        }
    }
}

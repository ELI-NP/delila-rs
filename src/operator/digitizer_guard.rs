//! Digitizer guard (TODO 85, always on): stop the run when a digitizer can no
//! longer take data.
//!
//! On pf-qaq (2026-11-08) a loose clock cable took three V1725 boards into
//! Error and run 70 went on for 5 h without them; nothing stopped it and
//! nothing on screen said so. The user's decision (2026-11-09): stop the run.
//!
//! A source counts as failed when, during a run, it reports
//! - PLL `Unlocked` — the board takes no valid data without its clock lock,
//! - PLL `Unreadable` — the board does not answer (a board that lost its
//!   clock usually does this rather than report FALSE), or
//! - state `Error` — e.g. read errors for 30 s (`read_loop_dig1`).
//!
//! The failure must last [`CONFIRM`] on the same source before the guard
//! acts: the DIG1 lock flag is sticky and may drop once during daisy-chain
//! initialization (UM5678 rev5 p.45), and a single unreadable poll can be an
//! optical-link transient that the Reader retries. The stop is drain-first,
//! like a full disk: the other boards' data is good and is written out.
//! Offline components are left alone (a slow status reply is not a dead
//! board).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use crate::common::{ComponentState, PllStatus};
use crate::operator::auto_stop::{self, AutoStopCause};
use crate::operator::routes::AppState;
use crate::operator::ComponentStatus;

/// Status poll period.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long a failure must last on one source before the run is stopped.
pub const CONFIRM: Duration = Duration::from_secs(10);

/// Why a source cannot take data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The digitizer reports its PLL not locked.
    ClockUnlocked,
    /// The PLL status cannot be read: the digitizer does not answer.
    ClockUnreadable,
    /// The Reader is in Error.
    ReaderError,
}

impl Fault {
    fn describe(self) -> &'static str {
        match self {
            Fault::ClockUnlocked => "clock lost (PLL not locked)",
            Fault::ClockUnreadable => "digitizer not answering (PLL status unreadable)",
            Fault::ReaderError => "Reader in Error",
        }
    }
}

/// The fault of one component, if it is an online source that has one.
pub fn fault_of(s: &ComponentStatus) -> Option<Fault> {
    if s.role != "source" || !s.online {
        return None;
    }
    if s.state == ComponentState::Error {
        return Some(Fault::ReaderError);
    }
    match s.metrics.as_ref().and_then(|m| m.pll) {
        Some(PllStatus::Unlocked) => Some(Fault::ClockUnlocked),
        Some(PllStatus::Unreadable) => Some(Fault::ClockUnreadable),
        Some(PllStatus::Locked) | None => None,
    }
}

/// A fault that has lasted at least [`CONFIRM`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedFault {
    pub name: String,
    pub fault: Fault,
    pub seconds: u64,
}

/// Remembers, per source, since when it has been failing (whatever the kind:
/// Unlocked turning into Unreadable is one continuing failure).
pub struct FaultTracker {
    confirm: Duration,
    since: HashMap<String, Instant>,
}

impl FaultTracker {
    pub fn new(confirm: Duration) -> Self {
        Self {
            confirm,
            since: HashMap::new(),
        }
    }

    /// Feed one status poll; returns the sources failing for `confirm` or
    /// longer. A source without a fault (or not in the poll) is forgotten.
    pub fn update(&mut self, statuses: &[ComponentStatus], now: Instant) -> Vec<ConfirmedFault> {
        let faults: Vec<(&str, Fault)> = statuses
            .iter()
            .filter_map(|s| Some((s.name.as_str(), fault_of(s)?)))
            .collect();
        self.since
            .retain(|name, _| faults.iter().any(|(n, _)| n == name));
        let mut confirmed = Vec::new();
        for (name, fault) in faults {
            let since = *self.since.entry(name.to_string()).or_insert(now);
            let lasted = now.duration_since(since);
            if lasted >= self.confirm {
                confirmed.push(ConfirmedFault {
                    name: name.to_string(),
                    fault,
                    seconds: lasted.as_secs(),
                });
            }
        }
        confirmed
    }

    pub fn clear(&mut self) {
        self.since.clear();
    }
}

/// One line for the log, the run record and the UI banner.
pub fn describe(faults: &[ConfirmedFault]) -> String {
    faults
        .iter()
        .map(|f| format!("{}: {} for {} s", f.name, f.fault.describe(), f.seconds))
        .collect::<Vec<_>>()
        .join("; ")
}

pub async fn run_watcher(state: Arc<AppState>) {
    info!(
        poll_interval_secs = POLL_INTERVAL.as_secs(),
        confirm_secs = CONFIRM.as_secs(),
        "Digitizer guard started (PLL not locked / unreadable or Reader in Error \
         during a run → drain-first stop)"
    );
    let mut tracker = FaultTracker::new(CONFIRM);

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        // Same gate as the run guard: only during a run, never in Tune Up.
        if state.current_run.read().await.is_none() || state.tuneup.is_active() {
            tracker.clear();
            continue;
        }

        let statuses = state.client.get_all_status(&state.components).await;
        let confirmed = tracker.update(&statuses, Instant::now());
        if confirmed.is_empty() {
            continue;
        }
        let reason = describe(&confirmed);
        error!(reason = %reason, "Digitizer guard: a digitizer stopped working — drain-first stop");

        // Check-then-act under the run-control lock (TODO 58 H14).
        let _run_guard = state.run_control_lock.lock().await;
        if state.current_run.read().await.is_none() {
            warn!("Run already ended before the digitizer guard could act — skipping");
            tracker.clear();
            continue;
        }
        let run_number = state
            .current_run
            .read()
            .await
            .as_ref()
            .map(|r| r.run_number);
        auto_stop::begin(
            &state.auto_stop,
            run_number,
            AutoStopCause::DigitizerFault,
            &reason,
        )
        .await;
        let timeout_ms = state.config.drain_stop_timeout_secs * 1000;
        let (status, response) =
            crate::operator::routes::status::drain_first_stop(&state, &reason, timeout_ms).await;
        auto_stop::finish(&state.auto_stop, response.0.success, &response.0.message).await;
        tracker.clear();
        info!(
            http_status = %status,
            message = %response.0.message,
            "Digitizer guard stop finished"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ComponentMetrics;

    fn source(name: &str, state: ComponentState, pll: Option<PllStatus>) -> ComponentStatus {
        ComponentStatus {
            name: name.to_string(),
            address: String::new(),
            state,
            run_number: Some(1),
            metrics: Some(ComponentMetrics {
                pll,
                ..Default::default()
            }),
            error: None,
            online: true,
            role: "source".to_string(),
        }
    }

    const RUN: ComponentState = ComponentState::Running;

    #[test]
    fn locked_or_unknown_sources_are_fine() {
        assert_eq!(fault_of(&source("A", RUN, Some(PllStatus::Locked))), None);
        assert_eq!(fault_of(&source("B", RUN, None)), None);
    }

    #[test]
    fn unlocked_unreadable_and_error_are_faults() {
        assert_eq!(
            fault_of(&source("A", RUN, Some(PllStatus::Unlocked))),
            Some(Fault::ClockUnlocked)
        );
        assert_eq!(
            fault_of(&source("A", RUN, Some(PllStatus::Unreadable))),
            Some(Fault::ClockUnreadable)
        );
        // Error outranks the PLL status: the Reader itself has given up.
        assert_eq!(
            fault_of(&source("A", ComponentState::Error, Some(PllStatus::Locked))),
            Some(Fault::ReaderError)
        );
        assert_eq!(
            fault_of(&source("A", ComponentState::Error, None)),
            Some(Fault::ReaderError)
        );
    }

    #[test]
    fn offline_and_pipeline_components_are_ignored() {
        let mut offline = source("A", ComponentState::Error, Some(PllStatus::Unreadable));
        offline.online = false;
        assert_eq!(fault_of(&offline), None);
        let mut merger = source("Merger", ComponentState::Error, None);
        merger.role = "pipeline".to_string();
        assert_eq!(fault_of(&merger), None);
    }

    #[test]
    fn a_fault_must_last_the_confirm_time() {
        let mut t = FaultTracker::new(Duration::from_secs(10));
        let t0 = Instant::now();
        let poll = |ring3: Option<PllStatus>| {
            vec![
                source("E-Sector", RUN, Some(PllStatus::Locked)),
                source("Ring3", RUN, ring3),
            ]
        };
        assert!(t.update(&poll(Some(PllStatus::Unlocked)), t0).is_empty());
        assert!(t
            .update(
                &poll(Some(PllStatus::Unreadable)),
                t0 + Duration::from_secs(9)
            )
            .is_empty());
        // Unlocked → Unreadable is one continuing failure.
        let c = t.update(
            &poll(Some(PllStatus::Unreadable)),
            t0 + Duration::from_secs(10),
        );
        assert_eq!(
            c,
            vec![ConfirmedFault {
                name: "Ring3".into(),
                fault: Fault::ClockUnreadable,
                seconds: 10,
            }]
        );
        assert_eq!(
            describe(&c),
            "Ring3: digitizer not answering (PLL status unreadable) for 10 s"
        );
    }

    #[test]
    fn a_recovered_source_starts_again_from_zero() {
        // The DIG1 sticky flag: one FALSE, then TRUE again.
        let mut t = FaultTracker::new(Duration::from_secs(10));
        let t0 = Instant::now();
        t.update(&[source("A", RUN, Some(PllStatus::Unlocked))], t0);
        t.update(
            &[source("A", RUN, Some(PllStatus::Locked))],
            t0 + Duration::from_secs(2),
        );
        assert!(t
            .update(
                &[source("A", RUN, Some(PllStatus::Unlocked))],
                t0 + Duration::from_secs(11),
            )
            .is_empty());
        assert!(!t
            .update(
                &[source("A", RUN, Some(PllStatus::Unlocked))],
                t0 + Duration::from_secs(21),
            )
            .is_empty());
    }

    #[test]
    fn several_failing_sources_are_all_named() {
        let mut t = FaultTracker::new(Duration::from_secs(10));
        let t0 = Instant::now();
        let poll = vec![
            source("dE-Sector", ComponentState::Error, None),
            source("Ring1", RUN, Some(PllStatus::Unreadable)),
        ];
        t.update(&poll, t0);
        let c = t.update(&poll, t0 + Duration::from_secs(12));
        assert_eq!(
            describe(&c),
            "dE-Sector: Reader in Error for 12 s; \
             Ring1: digitizer not answering (PLL status unreadable) for 12 s"
        );
    }
}

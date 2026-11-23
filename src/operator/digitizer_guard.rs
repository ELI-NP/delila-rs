//! Digitizer guard (TODO 85, always on): stop the run when a digitizer can no
//! longer take data.
//!
//! On pf-qaq (2026-11-08) a loose clock cable took three V1725 boards into
//! Error and run 70 went on for 5 h without them; nothing stopped it and
//! nothing on screen said so. The user's decision (2026-11-09): stop the run.
//!
//! A source counts as failed when, during a run, it reports
//! - PLL `Unlocked` — the board takes no valid data without its clock lock
//!   (DIG1), or a DIG2 board error such as `jesd_unlock` (named in
//!   `pll_detail`, TODO 85 §5.3),
//! - PLL `Unreadable` — the board does not answer (a board that lost its
//!   clock usually does this rather than report FALSE), or
//! - state `Error` — e.g. read errors for 30 s (`read_loop_dig1`), or
//! - no status reply at all — the Reader process is gone or hung (pf-qaq
//!   2026-11-23, TODO 88 §5.11/§5.13: Readers died of segfaults inside the
//!   CAEN library and the runs went on without those boards for hours).
//!
//! The failure must last [`CONFIRM`] on the same source before the guard
//! acts: the DIG1 lock flag is sticky and may drop once during daisy-chain
//! initialization (UM5678 rev5 p.45), and a single unreadable poll can be an
//! optical-link transient that the Reader retries. A missing reply must last
//! the longer [`OFFLINE_CONFIRM`]: one reply can time out (5 s) without the
//! Reader being dead. When no component answers at all, the Operator cannot
//! reach the pipeline: that is not one Reader's fault and is not counted.
//! The stop is drain-first, like a full disk: the other boards' data is good
//! and is written out.

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

/// How long a source must give no status reply before the run is stopped.
pub const OFFLINE_CONFIRM: Duration = Duration::from_secs(30);

/// Why a source cannot take data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The digitizer reports its PLL not locked.
    ClockUnlocked,
    /// The PLL status cannot be read: the digitizer does not answer.
    ClockUnreadable,
    /// The Reader is in Error.
    ReaderError,
    /// The Reader gives no status reply: its process is gone or hung.
    ReaderOffline,
}

impl Fault {
    /// `detail`: the DIG2 error bits behind `ClockUnlocked`, e.g. "jesd_unlock".
    fn describe(self, detail: Option<&str>) -> String {
        match (self, detail) {
            (Fault::ClockUnlocked, Some(d)) => format!("digitizer error ({d})"),
            (Fault::ClockUnlocked, None) => "clock lost (PLL not locked)".to_string(),
            (Fault::ClockUnreadable, _) => {
                "digitizer not answering (PLL status unreadable)".to_string()
            }
            (Fault::ReaderError, _) => "Reader in Error".to_string(),
            (Fault::ReaderOffline, _) => "Reader not answering (process gone or hung)".to_string(),
        }
    }
}

/// The fault of one component, if it is a source that has one.
pub fn fault_of(s: &ComponentStatus) -> Option<Fault> {
    if s.role != "source" {
        return None;
    }
    if !s.online {
        return Some(Fault::ReaderOffline);
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

/// A fault that has lasted at least its confirm time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedFault {
    pub name: String,
    pub fault: Fault,
    pub seconds: u64,
    /// What the digitizer named (DIG2 error bits), from the latest poll.
    pub detail: Option<String>,
}

/// Remembers, per source, since when it has been failing (whatever the kind:
/// Unlocked turning into Unreadable is one continuing failure).
pub struct FaultTracker {
    confirm: Duration,
    offline_confirm: Duration,
    since: HashMap<String, Instant>,
}

impl FaultTracker {
    /// `confirm` for the faults a source reports, `offline_confirm` for a
    /// source that gives no reply.
    pub fn new(confirm: Duration, offline_confirm: Duration) -> Self {
        Self {
            confirm,
            offline_confirm,
            since: HashMap::new(),
        }
    }

    /// Feed one status poll; returns the sources failing for their confirm
    /// time or longer. A source without a fault (or not in the poll) is
    /// forgotten.
    pub fn update(&mut self, statuses: &[ComponentStatus], now: Instant) -> Vec<ConfirmedFault> {
        // Nothing answers: the Operator cannot reach the pipeline. Not one
        // Reader's fault, and a stop would not get through either.
        let reachable = statuses.iter().any(|s| s.online);
        let faults: Vec<(&str, Fault, Option<String>)> = statuses
            .iter()
            .filter_map(|s| {
                let fault = fault_of(s)?;
                if fault == Fault::ReaderOffline && !reachable {
                    return None;
                }
                let detail = s.metrics.as_ref().and_then(|m| m.pll_detail.clone());
                Some((s.name.as_str(), fault, detail))
            })
            .collect();
        self.since
            .retain(|name, _| faults.iter().any(|(n, _, _)| n == name));
        let mut confirmed = Vec::new();
        for (name, fault, detail) in faults {
            let since = *self.since.entry(name.to_string()).or_insert(now);
            let lasted = now.duration_since(since);
            let needed = if fault == Fault::ReaderOffline {
                self.offline_confirm
            } else {
                self.confirm
            };
            if lasted >= needed {
                confirmed.push(ConfirmedFault {
                    name: name.to_string(),
                    fault,
                    seconds: lasted.as_secs(),
                    detail,
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
        .map(|f| {
            format!(
                "{}: {} for {} s",
                f.name,
                f.fault.describe(f.detail.as_deref()),
                f.seconds
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub async fn run_watcher(state: Arc<AppState>) {
    info!(
        poll_interval_secs = POLL_INTERVAL.as_secs(),
        confirm_secs = CONFIRM.as_secs(),
        offline_confirm_secs = OFFLINE_CONFIRM.as_secs(),
        "Digitizer guard started (PLL not locked / unreadable, Reader in Error or \
         not answering during a run → drain-first stop)"
    );
    let mut tracker = FaultTracker::new(CONFIRM, OFFLINE_CONFIRM);

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

    fn offline(mut s: ComponentStatus) -> ComponentStatus {
        s.online = false;
        s.metrics = None;
        s
    }

    fn merger() -> ComponentStatus {
        let mut m = source("Merger", RUN, None);
        m.role = "pipeline".to_string();
        m
    }

    #[test]
    fn pipeline_components_are_ignored() {
        let mut m = merger();
        m.state = ComponentState::Error;
        assert_eq!(fault_of(&m), None);
        assert_eq!(fault_of(&offline(merger())), None);
    }

    /// pf-qaq 2026-11-23 (TODO 88 §5.11, §5.13): a Reader process died
    /// (segfault inside the CAEN library) and the run went on without that
    /// board for hours — an offline source was ignored.
    #[test]
    fn an_offline_source_is_a_reader_that_does_not_answer() {
        assert_eq!(
            fault_of(&offline(source("A", RUN, None))),
            Some(Fault::ReaderOffline)
        );
    }

    #[test]
    fn an_offline_source_must_last_the_offline_confirm_time() {
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
        let t0 = Instant::now();
        let poll = vec![
            source("PSD1-V1730-ThGEM1-3", RUN, Some(PllStatus::Locked)),
            offline(source("PSD1-V1730-ThGEM7-8", RUN, None)),
            merger(),
        ];
        assert!(t.update(&poll, t0).is_empty());
        // One status reply can time out (5 s) without the Reader being dead.
        assert!(t.update(&poll, t0 + Duration::from_secs(10)).is_empty());
        let c = t.update(&poll, t0 + Duration::from_secs(30));
        assert_eq!(
            c,
            vec![ConfirmedFault {
                name: "PSD1-V1730-ThGEM7-8".into(),
                fault: Fault::ReaderOffline,
                seconds: 30,
                detail: None,
            }]
        );
        assert_eq!(
            describe(&c),
            "PSD1-V1730-ThGEM7-8: Reader not answering (process gone or hung) for 30 s"
        );
    }

    #[test]
    fn nothing_answering_is_not_a_dead_reader() {
        // The Operator cannot reach the pipeline at all: not one Reader's
        // fault, and a stop would not get through either.
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
        let t0 = Instant::now();
        let poll = vec![
            offline(source("A", RUN, None)),
            offline(source("B", RUN, None)),
            offline(merger()),
        ];
        assert!(t.update(&poll, t0).is_empty());
        assert!(t.update(&poll, t0 + Duration::from_secs(60)).is_empty());
    }

    #[test]
    fn a_reader_that_answers_again_starts_from_zero() {
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
        let t0 = Instant::now();
        let gone = vec![offline(source("A", RUN, None)), merger()];
        let back = vec![source("A", RUN, Some(PllStatus::Locked)), merger()];
        t.update(&gone, t0);
        t.update(&back, t0 + Duration::from_secs(20));
        assert!(t.update(&gone, t0 + Duration::from_secs(40)).is_empty());
        assert!(!t.update(&gone, t0 + Duration::from_secs(70)).is_empty());
    }

    #[test]
    fn a_fault_must_last_the_confirm_time() {
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
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
                detail: None,
            }]
        );
        assert_eq!(
            describe(&c),
            "Ring3: digitizer not answering (PLL status unreadable) for 10 s"
        );
    }

    /// TODO 85 §5.3: the VX2730 (clock master) was stopped as "clock lost"
    /// on 2026-11-10 for what was a board error; the reason names the bits.
    #[test]
    fn a_dig2_board_error_is_named_not_called_a_clock_loss() {
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
        let t0 = Instant::now();
        let mut vx = source("VX2730", RUN, Some(PllStatus::Unlocked));
        if let Some(ref mut m) = vx.metrics {
            m.pll_detail = Some("jesd_unlock".into());
        }
        assert!(t.update(std::slice::from_ref(&vx), t0).is_empty());
        let c = t.update(std::slice::from_ref(&vx), t0 + Duration::from_secs(10));
        assert_eq!(
            describe(&c),
            "VX2730: digitizer error (jesd_unlock) for 10 s"
        );
        // DIG1 has no bits to name: still the clock.
        let v1725 = source("Ring3", RUN, Some(PllStatus::Unlocked));
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
        assert!(t.update(std::slice::from_ref(&v1725), t0).is_empty());
        let c = t.update(std::slice::from_ref(&v1725), t0 + Duration::from_secs(10));
        assert_eq!(describe(&c), "Ring3: clock lost (PLL not locked) for 10 s");
    }

    #[test]
    fn a_recovered_source_starts_again_from_zero() {
        // The DIG1 sticky flag: one FALSE, then TRUE again.
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
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
        let mut t = FaultTracker::new(Duration::from_secs(10), Duration::from_secs(30));
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

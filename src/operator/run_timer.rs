//! Run timer (TODO 86): the Operator owns it, the pages only show it.
//!
//! Until 2026-11-10 the timer lived in the browser that pressed Start: a
//! `setInterval` took one second off a counter, only the Control page showed
//! it, a hidden tab slowed it down (Chrome runs timers of a page hidden for 5
//! minutes once a minute), and a reload or a closed browser lost it, so the run
//! never stopped. Now:
//!
//! - Every run gets a [`RunClock`] when it starts: its monotonic start (the
//!   moment Start succeeded, as `start_time`) and, optionally, a timer.
//!   Monotonic, so an NTP step neither lengthens nor shortens a run.
//! - `/api/status` reports the remaining time computed here
//!   ([`RunTimerStatus`]); the pages count down from that value and correct
//!   themselves at every poll.
//! - [`run_watcher`] checks once a second. When the time is up it stops the
//!   run exactly like the Stop button (user, 2026-11-10), with the stop reason
//!   `timer (N min)` in the run record and the eLog summary, or, without
//!   auto-stop, only reports it. Either way [`TimerExpiredNotice`] tells every
//!   open page once.
//! - The timer can be set, changed or cancelled during the run
//!   (`PUT` / `DELETE /api/run/timer`); each change is written as a run note.
//!
//! The clock is in memory like `current_run`: an Operator restart is a DAQ
//! restart and never happens during a run (user, 2026-11-10).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::operator::routes::AppState;

/// Longest timer: 24 h, so an overnight run can be timed (user, 2026-11-10).
pub const MAX_MINUTES: u32 = 24 * 60;

/// How often the watcher looks at the clock.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

fn default_true() -> bool {
    true
}

/// A run timer as the UI sends it: with Start, or to `PUT /api/run/timer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RunTimerRequest {
    /// Run length in minutes, counted from the start of the run (1 to 1440).
    pub minutes: u32,
    /// Stop the run when the time is up. False: only tell the open pages.
    #[serde(default = "default_true")]
    pub auto_stop: bool,
}

impl RunTimerRequest {
    /// Refuse a length outside 1 ..= [`MAX_MINUTES`].
    pub fn validate(&self) -> Result<(), String> {
        if self.minutes == 0 || self.minutes > MAX_MINUTES {
            return Err(format!(
                "Run timer must be 1 to {MAX_MINUTES} minutes, got {}",
                self.minutes
            ));
        }
        Ok(())
    }

    fn duration(&self) -> Duration {
        Duration::from_secs(u64::from(self.minutes) * 60)
    }

    /// "120 min, stops the run" / "120 min, alarm only".
    pub fn describe(&self) -> String {
        let what = if self.auto_stop {
            "stops the run"
        } else {
            "alarm only"
        };
        format!("{} min, {what}", self.minutes)
    }
}

/// The current run's clock: when it started and its timer, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunClock {
    pub run_number: i32,
    /// Monotonic start of the run (the moment Start succeeded).
    pub started: Instant,
    pub timer: Option<RunTimerRequest>,
    /// The time is up and was handled (stopped or reported): not again.
    pub expired: bool,
}

impl RunClock {
    pub fn new(run_number: i32, started: Instant, timer: Option<RunTimerRequest>) -> Self {
        Self {
            run_number,
            started,
            timer,
            expired: false,
        }
    }

    /// Milliseconds until the time is up, negative once it is over.
    pub fn remaining_ms(&self, timer: &RunTimerRequest, now: Instant) -> i64 {
        let elapsed = now.saturating_duration_since(self.started);
        let total = timer.duration();
        if elapsed <= total {
            (total - elapsed).as_millis() as i64
        } else {
            -((elapsed - total).as_millis() as i64)
        }
    }

    /// `run_info.timer` of `/api/status`, `None` without a timer.
    pub fn status(&self, now: Instant, now_wall_ms: i64) -> Option<RunTimerStatus> {
        let timer = self.timer?;
        let remaining_ms = self.remaining_ms(&timer, now);
        Some(RunTimerStatus {
            minutes: timer.minutes,
            auto_stop: timer.auto_stop,
            remaining_ms,
            ends_at_ms: now_wall_ms + remaining_ms,
        })
    }
}

/// The run timer in `/api/status` (`run_info.timer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RunTimerStatus {
    pub minutes: u32,
    pub auto_stop: bool,
    /// Milliseconds until the time is up, as the Operator computed it when it
    /// answered. Negative once over (only without auto-stop). The pages count
    /// down from it, so their own clock does not matter.
    pub remaining_ms: i64,
    /// When the time is up, UNIX ms (for display).
    pub ends_at_ms: i64,
}

/// The time of a run is up (TODO 86): every open page alarms once. Kept in
/// `/api/status` until the next run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TimerExpiredNotice {
    pub run_number: i32,
    pub minutes: u32,
    /// True: the Operator stopped the run.
    pub auto_stop: bool,
    /// UNIX ms when the time was up; identifies the notice.
    pub at_ms: i64,
}

/// What the watcher does with the clock on one pass.
#[derive(Debug, PartialEq, Eq)]
pub enum TimerAction {
    /// Nothing to do yet (or no timer, or already handled).
    Wait,
    /// The time is up: report it, and stop the run if `stop`.
    Expired { stop: bool },
    /// The clock belongs to a run that is no longer the current one.
    Forget,
}

/// Decide one watcher pass. `current_run` is the run number of the run in
/// progress, if any.
pub fn decide(clock: &RunClock, current_run: Option<i32>, now: Instant) -> TimerAction {
    if current_run != Some(clock.run_number) {
        return TimerAction::Forget;
    }
    let Some(timer) = clock.timer else {
        return TimerAction::Wait;
    };
    if clock.expired || clock.remaining_ms(&timer, now) > 0 {
        return TimerAction::Wait;
    }
    TimerAction::Expired {
        stop: timer.auto_stop,
    }
}

/// Start the clock of a run that has just started: called by every path that
/// sets `current_run` to a new run. Clears the previous run's notice.
pub async fn begin_run(state: &AppState, run_number: i32, timer: Option<RunTimerRequest>) {
    *state.timer_expired.write().await = None;
    *state.run_clock.write().await = Some(RunClock::new(run_number, Instant::now(), timer));
    if let Some(t) = timer {
        info!(run_number, timer = %t.describe(), "Run timer set at start");
        add_note(state, &format!("Run timer: {}", t.describe())).await;
    }
}

/// Write a note into the current run (MongoDB when configured, and the
/// in-memory run info the pages show).
pub async fn add_note(state: &AppState, text: &str) {
    let Some(info) = state.current_run.read().await.clone() else {
        return;
    };
    let note = match state.run_repo {
        Some(ref repo) => match repo.add_note(info.run_number, &info.exp_name, text).await {
            Ok(note) => note,
            Err(e) => {
                warn!(error = %e, text, "Run note not stored in MongoDB");
                return;
            }
        },
        None => crate::operator::RunNote {
            time: chrono::Utc::now().timestamp_millis(),
            text: text.to_string(),
        },
    };
    if let Some(ref mut current) = *state.current_run.write().await {
        if current.run_number == info.run_number {
            current.notes.push(note);
        }
    }
}

/// One watcher pass at time `now`.
pub async fn poll_once(state: &Arc<AppState>, now: Instant) {
    let Some(clock) = *state.run_clock.read().await else {
        return;
    };
    let current = state
        .current_run
        .read()
        .await
        .as_ref()
        .map(|r| r.run_number);
    match decide(&clock, current, now) {
        TimerAction::Wait => {}
        TimerAction::Forget => {
            let mut slot = state.run_clock.write().await;
            if *slot == Some(clock) {
                *slot = None;
            }
        }
        TimerAction::Expired { stop } => expire(state, clock, stop).await,
    }
}

/// The time is up: under the run-control lock, check that the run and its
/// timer are still the ones decided on (a Stop, a new run or a timer change
/// may have come in between, TODO 58 H14), then report and maybe stop.
async fn expire(state: &Arc<AppState>, clock: RunClock, stop: bool) {
    let _run_guard = state.run_control_lock.lock().await;
    {
        let mut slot = state.run_clock.write().await;
        if *slot != Some(clock) {
            return;
        }
        let current = state
            .current_run
            .read()
            .await
            .as_ref()
            .map(|r| r.run_number);
        if current != Some(clock.run_number) {
            return;
        }
        if let Some(ref mut c) = *slot {
            c.expired = true;
        }
    }
    let Some(timer) = clock.timer else {
        return;
    };
    *state.timer_expired.write().await = Some(TimerExpiredNotice {
        run_number: clock.run_number,
        minutes: timer.minutes,
        auto_stop: stop,
        at_ms: chrono::Utc::now().timestamp_millis(),
    });
    if !stop {
        info!(
            run_number = clock.run_number,
            minutes = timer.minutes,
            "Run timer: the time is up (no auto-stop) — the pages are told, the run goes on"
        );
        return;
    }
    let reason = format!("timer ({} min)", timer.minutes);
    info!(
        run_number = clock.run_number,
        minutes = timer.minutes,
        "Run timer: the time is up — stopping the run like the Stop button"
    );
    let (status, response) =
        crate::operator::routes::status::plain_stop(state, Some(&reason)).await;
    if response.0.success {
        info!(http_status = %status, "Run timer stop finished: {}", response.0.message);
    } else {
        warn!(http_status = %status, "Run timer stop had a problem: {}", response.0.message);
    }
}

/// Check the run clock once a second (spawned by the Operator binary).
pub async fn run_watcher(state: Arc<AppState>) {
    info!(
        poll_interval_secs = POLL_INTERVAL.as_secs(),
        "Run timer watcher started (stops a timed run like the Stop button when its time is up)"
    );
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    loop {
        tick.tick().await;
        // Same gate as the other watchers: never in Tune Up.
        if state.tuneup.is_active() {
            continue;
        }
        poll_once(&state, Instant::now()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer(minutes: u32, auto_stop: bool) -> RunTimerRequest {
        RunTimerRequest { minutes, auto_stop }
    }

    #[test]
    fn timer_length_is_1_to_1440_minutes() {
        assert!(timer(1, true).validate().is_ok());
        assert!(timer(MAX_MINUTES, true).validate().is_ok());
        assert!(timer(0, true).validate().is_err());
        let e = timer(MAX_MINUTES + 1, true).validate().unwrap_err();
        assert!(e.contains("1 to 1440"), "{e}");
    }

    #[test]
    fn auto_stop_defaults_to_true() {
        let t: RunTimerRequest = serde_json::from_str(r#"{"minutes": 120}"#).unwrap();
        assert_eq!(t, timer(120, true));
        let t: RunTimerRequest =
            serde_json::from_str(r#"{"minutes": 5, "auto_stop": false}"#).unwrap();
        assert_eq!(t, timer(5, false));
    }

    #[test]
    fn remaining_counts_from_the_start_and_goes_negative_when_over() {
        let t0 = Instant::now();
        let clock = RunClock::new(7, t0, Some(timer(2, false)));
        let t = clock.timer.unwrap();
        assert_eq!(clock.remaining_ms(&t, t0), 120_000);
        assert_eq!(clock.remaining_ms(&t, t0 + Duration::from_secs(30)), 90_000);
        assert_eq!(
            clock.remaining_ms(&t, t0 + Duration::from_secs(125)),
            -5_000
        );
    }

    #[test]
    fn status_has_the_end_in_wall_clock_time() {
        let t0 = Instant::now();
        let clock = RunClock::new(7, t0, Some(timer(2, true)));
        let s = clock
            .status(t0 + Duration::from_secs(20), 1_000_000)
            .unwrap();
        assert_eq!(
            s,
            RunTimerStatus {
                minutes: 2,
                auto_stop: true,
                remaining_ms: 100_000,
                ends_at_ms: 1_100_000,
            }
        );
        assert!(RunClock::new(7, t0, None).status(t0, 0).is_none());
    }

    #[test]
    fn decide_waits_until_the_time_is_up() {
        let t0 = Instant::now();
        let clock = RunClock::new(7, t0, Some(timer(1, true)));
        assert_eq!(
            decide(&clock, Some(7), t0 + Duration::from_secs(59)),
            TimerAction::Wait
        );
        assert_eq!(
            decide(&clock, Some(7), t0 + Duration::from_secs(60)),
            TimerAction::Expired { stop: true }
        );
        let alarm_only = RunClock::new(7, t0, Some(timer(1, false)));
        assert_eq!(
            decide(&alarm_only, Some(7), t0 + Duration::from_secs(61)),
            TimerAction::Expired { stop: false }
        );
    }

    #[test]
    fn decide_acts_once() {
        let t0 = Instant::now();
        let mut clock = RunClock::new(7, t0, Some(timer(1, false)));
        clock.expired = true;
        assert_eq!(
            decide(&clock, Some(7), t0 + Duration::from_secs(600)),
            TimerAction::Wait
        );
    }

    #[test]
    fn decide_without_a_timer_waits() {
        let t0 = Instant::now();
        let clock = RunClock::new(7, t0, None);
        assert_eq!(
            decide(&clock, Some(7), t0 + Duration::from_secs(86_400)),
            TimerAction::Wait
        );
    }

    #[test]
    fn decide_forgets_the_clock_of_a_run_that_is_over() {
        let t0 = Instant::now();
        let clock = RunClock::new(7, t0, Some(timer(1, true)));
        let late = t0 + Duration::from_secs(600);
        assert_eq!(decide(&clock, None, late), TimerAction::Forget);
        assert_eq!(decide(&clock, Some(8), late), TimerAction::Forget);
    }
}

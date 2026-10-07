//! Run guard (TODO 69, always on): stop the run when the recording path can
//! no longer keep the data.
//!
//! - A component reports `write_failed` — a file write/close failed; the
//!   Recorder closed its file at the last complete block and writes nothing
//!   more this run. **Immediate stop**, sources first: draining into a disk
//!   that just failed saves nothing, and every second of running is lost
//!   data. The run is recorded as Error.
//! - A component reports `disk_level == 2` — free space below its hard
//!   watermark while the disk still takes writes. **Drain-first stop**
//!   (TODO 68 machinery): the backlog is written and the file closed with its
//!   footer before the disk fills.
//!
//! Unlike the backlog autostop (TODO 68, double opt-in because its RAM
//! thresholds depend on the host) there is no switch: a failed write is a
//! fact, and the absolute rule is "stop rather than lose data". The
//! free-space watermarks are Recorder config (`disk_hard_free_mb = 0`
//! disables that half). Start is refused below the hard watermark too
//! ([`start_refusal`]), so a run cannot begin on a disk it would fill.

use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::operator::routes::AppState;
use crate::operator::ComponentStatus;

/// Status poll period. A write failure loses at most this much data (plus
/// the stop itself); the poll is a handful of ZMQ round trips.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// What the guard does about the current component statuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAction {
    /// A write failed: stop now (sources first), record the run as Error.
    ImmediateStop(String),
    /// Free space below the hard watermark: drain-first stop.
    DrainStop(String),
}

fn gb(bytes: Option<u64>) -> String {
    match bytes {
        Some(b) if b >= 1_000_000_000 => format!("{:.1} GB free", b as f64 / 1e9),
        Some(b) => format!("{} MB free", b / 1_000_000),
        None => "free space unknown".to_string(),
    }
}

/// Decide from the statuses of online components. A write failure outranks
/// low disk: the data is already being lost.
pub fn decide(statuses: &[ComponentStatus]) -> Option<GuardAction> {
    let online = || {
        statuses
            .iter()
            .filter(|s| s.online)
            .filter_map(|s| Some((s, s.metrics.as_ref()?)))
    };
    let failed: Vec<String> = online()
        .filter(|(_, m)| m.write_failed)
        .map(|(s, m)| {
            format!(
                "{} ({} events not written so far)",
                s.name, m.unwritten_events
            )
        })
        .collect();
    if !failed.is_empty() {
        return Some(GuardAction::ImmediateStop(format!(
            "write failure on {}",
            failed.join(", ")
        )));
    }
    let low: Vec<String> = online()
        .filter(|(_, m)| m.disk_level >= 2)
        .map(|(s, m)| format!("{} ({})", s.name, gb(m.disk_free_bytes)))
        .collect();
    if !low.is_empty() {
        return Some(GuardAction::DrainStop(format!(
            "disk free space below the hard watermark on {}",
            low.join(", ")
        )));
    }
    None
}

/// Why a run must not start, if it must not: a writer's free space is below
/// its hard watermark — the guard would stop the run seconds later.
pub fn start_refusal(statuses: &[ComponentStatus]) -> Option<String> {
    let low: Vec<String> = statuses
        .iter()
        .filter(|s| s.online)
        .filter_map(|s| Some((s, s.metrics.as_ref()?)))
        .filter(|(_, m)| m.disk_level >= 2)
        .map(|(s, m)| format!("{} ({})", s.name, gb(m.disk_free_bytes)))
        .collect();
    (!low.is_empty()).then(|| {
        format!(
            "Run start refused: disk free space below the hard watermark on {} — free space \
             (or lower disk_hard_free_mb in [network.recorder]) and retry",
            low.join(", ")
        )
    })
}

/// The write-failure latch of a previous run is still set (cleared by
/// Configure/Reset): a plain Start would be stopped again at once.
pub fn latched_write_failure(statuses: &[ComponentStatus]) -> Option<String> {
    let failed: Vec<&str> = statuses
        .iter()
        .filter(|s| s.online)
        .filter(|s| s.metrics.as_ref().is_some_and(|m| m.write_failed))
        .map(|s| s.name.as_str())
        .collect();
    (!failed.is_empty()).then(|| {
        format!(
            "Run start refused: write failure still latched on {} — fix the disk, then \
             Configure (or Reset) before starting",
            failed.join(", ")
        )
    })
}

pub async fn run_watcher(state: Arc<AppState>) {
    info!(
        poll_interval_secs = POLL_INTERVAL.as_secs(),
        "Run guard started (write failure → immediate stop, disk below hard watermark → drain-first stop)"
    );

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        // Same gate as the backlog watcher: only during a run, never during
        // Tune Up (it owns its own stop paths).
        if state.current_run.read().await.is_none() || state.tuneup.is_active() {
            continue;
        }

        let statuses = state.client.get_all_status(&state.components).await;
        let Some(action) = decide(&statuses) else {
            continue;
        };
        match &action {
            GuardAction::ImmediateStop(reason) => {
                error!(reason = %reason, "Run guard: write failure — stopping the run now")
            }
            GuardAction::DrainStop(reason) => {
                error!(reason = %reason, "Run guard: disk nearly full — drain-first stop")
            }
        }

        // Check-then-act under the run-control lock (TODO 58 H14), as in
        // backlog_watch: a concurrent /api/stop afterwards is a no-op.
        let _run_guard = state.run_control_lock.lock().await;
        if state.current_run.read().await.is_none() {
            warn!("Run already ended before the run guard could act — skipping");
            continue;
        }

        let (status, response) = match action {
            GuardAction::ImmediateStop(reason) => {
                crate::operator::routes::status::failure_stop(&state, &reason).await
            }
            GuardAction::DrainStop(reason) => {
                let timeout_ms = state.config.drain_stop_timeout_secs * 1000;
                crate::operator::routes::status::drain_first_stop(&state, &reason, timeout_ms).await
            }
        };
        info!(
            http_status = %status,
            message = %response.0.message,
            "Run guard stop finished"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{ComponentMetrics, ComponentState};

    fn status(name: &str, online: bool, m: Option<ComponentMetrics>) -> ComponentStatus {
        ComponentStatus {
            name: name.to_string(),
            address: String::new(),
            state: ComponentState::Running,
            run_number: Some(1),
            metrics: m,
            error: None,
            online,
            role: "pipeline".to_string(),
        }
    }

    fn metrics(
        write_failed: bool,
        unwritten: u64,
        free: Option<u64>,
        level: u8,
    ) -> ComponentMetrics {
        ComponentMetrics {
            write_failed,
            unwritten_events: unwritten,
            disk_free_bytes: free,
            disk_level: level,
            ..Default::default()
        }
    }

    #[test]
    fn healthy_pipeline_needs_nothing() {
        let s = vec![
            status("Reader", true, Some(ComponentMetrics::default())),
            status(
                "Recorder",
                true,
                Some(metrics(false, 0, Some(5e11 as u64), 0)),
            ),
            status("Monitor", true, None),
        ];
        assert_eq!(decide(&s), None);
        assert_eq!(start_refusal(&s), None);
        assert_eq!(latched_write_failure(&s), None);
    }

    #[test]
    fn free_space_is_readable_at_any_size() {
        assert_eq!(gb(Some(1_500_000_000)), "1.5 GB free");
        assert_eq!(gb(Some(24_000_000)), "24 MB free");
        assert_eq!(gb(None), "free space unknown");
    }

    #[test]
    fn soft_level_only_warns() {
        let s = vec![status(
            "Recorder",
            true,
            Some(metrics(false, 0, Some(10), 1)),
        )];
        assert_eq!(decide(&s), None);
        assert_eq!(start_refusal(&s), None);
    }

    #[test]
    fn write_failure_stops_immediately_and_outranks_low_disk() {
        let s = vec![
            status("Recorder", true, Some(metrics(true, 1234, Some(0), 2))),
            status("Recorder2", true, Some(metrics(false, 0, Some(1), 2))),
        ];
        match decide(&s) {
            Some(GuardAction::ImmediateStop(r)) => {
                assert!(
                    r.contains("Recorder (1234 events not written so far)"),
                    "{r}"
                );
                assert!(!r.contains("Recorder2"), "{r}");
            }
            other => panic!("expected ImmediateStop, got {other:?}"),
        }
        assert!(latched_write_failure(&s).unwrap().contains("Recorder"));
    }

    #[test]
    fn any_one_of_several_writers_failing_stops_the_run() {
        let s = vec![
            status(
                "Recorder",
                true,
                Some(metrics(false, 0, Some(5e11 as u64), 0)),
            ),
            status(
                "Recorder2",
                true,
                Some(metrics(true, 0, Some(5e11 as u64), 0)),
            ),
        ];
        assert!(matches!(decide(&s), Some(GuardAction::ImmediateStop(_))));
    }

    #[test]
    fn hard_disk_level_drains_and_refuses_start() {
        let s = vec![status(
            "Recorder",
            true,
            Some(metrics(false, 0, Some(1_500_000_000), 2)),
        )];
        match decide(&s) {
            Some(GuardAction::DrainStop(r)) => assert!(r.contains("1.5 GB free"), "{r}"),
            other => panic!("expected DrainStop, got {other:?}"),
        }
        assert!(start_refusal(&s)
            .unwrap()
            .contains("Recorder (1.5 GB free)"));
    }

    #[test]
    fn offline_components_are_ignored() {
        // A stale snapshot from an unreachable component must not stop a run.
        let s = vec![status(
            "Recorder",
            false,
            Some(metrics(true, 9, Some(0), 2)),
        )];
        assert_eq!(decide(&s), None);
        assert_eq!(start_refusal(&s), None);
        assert_eq!(latched_write_failure(&s), None);
    }
}

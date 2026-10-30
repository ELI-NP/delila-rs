//! No-event warning (TODO 78 a): a source that reports Running while its
//! event count does not move.
//!
//! A DIG1 S-IN slave whose start signal never arrives still reports Running
//! (its Start is a no-op, TODO 77 §4), so the run records nothing from that
//! board and nothing says so. During a run the Operator polls every second
//! and lists every Running source whose event count has not changed for
//! `[operator] no_event_warn_secs`. `/api/status` carries the list, the UI
//! shows it and the log records each source going quiet and coming back.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::common::ComponentState;
use crate::operator::routes::AppState;
use crate::operator::ComponentStatus;

const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// A Running source whose event count has not moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StalledSource {
    /// Component name
    pub name: String,
    /// Seconds since its event count last changed (or since it was first
    /// seen Running)
    pub seconds: u64,
}

/// Remembers, per source, the last event count and when it changed.
pub struct StallTracker {
    threshold: Duration,
    last_change: HashMap<String, (u64, Instant)>,
}

impl StallTracker {
    pub fn new(threshold: Duration) -> Self {
        Self {
            threshold,
            last_change: HashMap::new(),
        }
    }

    /// Feed one status poll; returns the sources quiet for `threshold` or
    /// longer. A source that is not Running (or offline, or without metrics)
    /// is forgotten, so its clock starts again when it runs.
    pub fn update(&mut self, statuses: &[ComponentStatus], now: Instant) -> Vec<StalledSource> {
        let mut stalled = Vec::new();
        for s in statuses.iter().filter(|s| s.role == "source") {
            let events = s
                .metrics
                .as_ref()
                .filter(|_| s.online && s.state == ComponentState::Running)
                .map(|m| m.events_processed);
            let Some(events) = events else {
                self.last_change.remove(&s.name);
                continue;
            };
            let entry = self
                .last_change
                .entry(s.name.clone())
                .or_insert((events, now));
            if entry.0 != events {
                *entry = (events, now);
            }
            let quiet = now.duration_since(entry.1);
            if quiet >= self.threshold {
                stalled.push(StalledSource {
                    name: s.name.clone(),
                    seconds: quiet.as_secs(),
                });
            }
        }
        stalled
    }

    pub fn clear(&mut self) {
        self.last_change.clear();
    }
}

pub async fn run_watcher(state: Arc<AppState>, threshold: Duration) {
    info!(
        no_event_warn_secs = threshold.as_secs(),
        "No-event watcher started (warns when a Running source delivers no events)"
    );
    let mut tracker = StallTracker::new(threshold);
    let mut previous: Vec<String> = Vec::new();

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        // Same gate as the run guard: only during a run, never in Tune Up.
        let stalled = if state.current_run.read().await.is_some() && !state.tuneup.is_active() {
            let statuses = state.client.get_all_status(&state.components).await;
            tracker.update(&statuses, Instant::now())
        } else {
            tracker.clear();
            Vec::new()
        };

        for s in stalled.iter().filter(|s| !previous.contains(&s.name)) {
            warn!(
                source = %s.name,
                seconds = s.seconds,
                "No events from a Running source — check its trigger; for an S-IN slave, \
                 check that the master's start signal arrives"
            );
        }
        for name in previous
            .iter()
            .filter(|name| !stalled.iter().any(|s| &s.name == *name))
        {
            info!(source = %name, "Source delivers events again (or is no longer running)");
        }
        previous = stalled.iter().map(|s| s.name.clone()).collect();
        *state.stalled_sources.write().await = stalled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ComponentMetrics;

    fn source(name: &str, state: ComponentState, events: u64) -> ComponentStatus {
        ComponentStatus {
            name: name.to_string(),
            address: String::new(),
            state,
            run_number: Some(1),
            metrics: Some(ComponentMetrics {
                events_processed: events,
                ..Default::default()
            }),
            error: None,
            online: true,
            role: "source".to_string(),
        }
    }

    fn names(stalled: &[StalledSource]) -> Vec<&str> {
        stalled.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn a_running_source_without_new_events_is_reported_after_the_threshold() {
        let mut t = StallTracker::new(Duration::from_secs(1));
        let t0 = Instant::now();
        let poll = |n_master: u64| {
            vec![
                source("Master", ComponentState::Running, n_master),
                source("Slave", ComponentState::Running, 0),
            ]
        };

        assert!(t.update(&poll(100), t0).is_empty());
        let stalled = t.update(&poll(2_000), t0 + Duration::from_millis(1_000));
        assert_eq!(names(&stalled), ["Slave"]);
        let stalled = t.update(&poll(4_000), t0 + Duration::from_millis(3_500));
        assert_eq!(stalled[0].seconds, 3);
    }

    #[test]
    fn new_events_restart_the_clock() {
        let mut t = StallTracker::new(Duration::from_secs(1));
        let t0 = Instant::now();
        t.update(&[source("A", ComponentState::Running, 5)], t0);
        let s = t.update(
            &[source("A", ComponentState::Running, 6)],
            t0 + Duration::from_millis(1_500),
        );
        assert!(s.is_empty());
        let s = t.update(
            &[source("A", ComponentState::Running, 6)],
            t0 + Duration::from_millis(2_000),
        );
        assert!(s.is_empty(), "only 0.5 s since the last new event");
    }

    #[test]
    fn non_running_offline_and_pipeline_components_are_ignored() {
        let mut t = StallTracker::new(Duration::from_secs(1));
        let t0 = Instant::now();
        let mut offline = source("Offline", ComponentState::Running, 0);
        offline.online = false;
        let mut merger = source("Merger", ComponentState::Running, 0);
        merger.role = "pipeline".to_string();
        let poll = vec![source("Armed", ComponentState::Armed, 0), offline, merger];
        t.update(&poll, t0);
        assert!(t.update(&poll, t0 + Duration::from_secs(5)).is_empty());
    }

    #[test]
    fn a_source_that_stops_running_is_forgotten() {
        let mut t = StallTracker::new(Duration::from_secs(1));
        let t0 = Instant::now();
        t.update(&[source("A", ComponentState::Running, 0)], t0);
        t.update(
            &[source("A", ComponentState::Configured, 0)],
            t0 + Duration::from_secs(2),
        );
        let s = t.update(
            &[source("A", ComponentState::Running, 0)],
            t0 + Duration::from_millis(2_500),
        );
        assert!(s.is_empty(), "the clock restarts when it runs again");
    }
}

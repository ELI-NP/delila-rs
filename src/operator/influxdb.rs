//! InfluxDB metrics writer for Grafana monitoring
//!
//! Polls every component via ZMQ GetStatus each `interval_secs` and writes
//! InfluxDB v3 Core Line Protocol (schema: `docs/plans/grafana_monitoring.md`):
//!
//! - `system_rate`  — per source module: total events, event rate, bytes.
//! - `channel_rate` — per (module, channel): cumulative `count` and `rate` (Hz).
//!   The rate is computed here from the count delta divided by the elapsed time
//!   measured with `Instant` (not the configured interval, which jitters).
//!   No channel-name tag: names are applied at display time (TODO 74 §3.1).
//! - `backlog` / `disk` — per pipeline component (TODO 68 / TODO 69).
//! - `run_event` — one line on each run start / stop, for Grafana annotations
//!   (TODO 74 P5).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::routes::AppState;
use super::ComponentStatus;
use crate::common::ComponentState;
use crate::config::InfluxDbConfig;

/// Run the InfluxDB writer loop. Never returns unless InfluxDB is unreachable.
pub async fn run_writer(config: InfluxDbConfig, state: Arc<AppState>) {
    let client = reqwest::Client::new();
    let write_url = format!(
        "{}/api/v3/write_lp?db={}",
        config.url.trim_end_matches('/'),
        config.database
    );
    let interval = Duration::from_secs(config.interval_secs);

    info!(
        url = %config.url,
        database = %config.database,
        interval_secs = config.interval_secs,
        "InfluxDB writer started"
    );

    let mut rates = RateTracker::default();
    let mut runs = RunEventDetector::default();

    loop {
        tokio::time::sleep(interval).await;

        // Poll all components for status
        let statuses = state.client.get_all_status(&state.components).await;
        let now = Instant::now();

        // Build Line Protocol lines from Reader metrics
        let mut lines = Vec::new();

        // Run start/stop events (TODO 74 P5), judged from the source components.
        let sources: Vec<&ComponentStatus> = state
            .components
            .iter()
            .zip(statuses.iter())
            .filter(|(comp, _)| comp.source_id.is_some())
            .map(|(_, status)| status)
            .collect();
        for event in runs.update(running_run_number(&sources)) {
            lines.push(run_event_line(&state.config.experiment_name, event));
        }

        for (comp, status) in state.components.iter().zip(statuses.iter()) {
            if !status.online {
                continue;
            }

            let metrics = match &status.metrics {
                Some(m) => m,
                None => continue,
            };

            // Non-source components (Merger/Recorder/...): export the backlog
            // gauge (TODO 68) — before this, their metrics never reached
            // Grafana at all.
            let source_id = match comp.source_id {
                Some(id) => id,
                None => {
                    lines.push(format!(
                        "backlog,component={} queue_bytes={}u,queue_bytes_peak={}u,queue_items={}u,backlog_level={}u,events_processed={}u",
                        comp.name.replace(' ', "\\ "),
                        metrics.queue_bytes,
                        metrics.queue_bytes_peak,
                        metrics.queue_size,
                        metrics.backlog_level,
                        metrics.events_processed,
                    ));
                    // Writers only (TODO 69): free space and write health.
                    if let Some(free) = metrics.disk_free_bytes {
                        lines.push(format!(
                            "disk,component={} free_bytes={}u,disk_level={}u,write_failed={},unwritten_events={}u",
                            comp.name.replace(' ', "\\ "),
                            free,
                            metrics.disk_level,
                            metrics.write_failed,
                            metrics.unwritten_events,
                        ));
                    }
                    continue;
                }
            };

            // System-level metrics for this source
            lines.push(format!(
                "system_rate,module={} total_events={}u,event_rate={},bytes={}u",
                source_id, metrics.events_processed, metrics.event_rate, metrics.bytes_transferred,
            ));

            // Per-channel counts and rates. The rate state is updated for every
            // channel; a line is written only for channels that have counted.
            if let Some(ref counts) = metrics.channel_counts {
                for (ch, &count) in counts.iter().enumerate() {
                    let rate = rates.update((source_id, ch), count, now);
                    if count > 0 {
                        lines.push(channel_rate_line(source_id, ch, count, rate));
                    }
                }
            }
        }

        if lines.is_empty() {
            continue;
        }

        let body = lines.join("\n");

        match client.post(&write_url).body(body).send().await {
            Ok(resp) if resp.status().is_success() => {
                // Success - no log needed at normal operation
            }
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!(
                    status = %status,
                    body = %text,
                    "InfluxDB write failed"
                );
            }
            Err(e) => {
                warn!(error = %e, "InfluxDB connection error");
            }
        }
    }
}

/// Per-(module, channel) event rate from cumulative counts (TODO 74 P2).
///
/// Remembers, for each key, the last count and the `Instant` of the poll in
/// which it was seen.
#[derive(Debug, Default)]
struct RateTracker {
    prev: HashMap<(u32, usize), (u64, Instant)>,
}

impl RateTracker {
    /// Store `count` seen at `now` and return the rate (Hz) since the previous
    /// sample of `key`.
    ///
    /// - first sample of a key → `None` (no rate field)
    /// - count went back (counter reset, new run) → `Some(0.0)`
    /// - zero elapsed time or a non-finite result → `None`
    fn update(&mut self, key: (u32, usize), count: u64, now: Instant) -> Option<f64> {
        let (prev_count, prev_time) = self.prev.insert(key, (count, now))?;
        if count < prev_count {
            return Some(0.0);
        }
        let elapsed = now.saturating_duration_since(prev_time).as_secs_f64();
        if elapsed <= 0.0 {
            return None;
        }
        let rate = (count - prev_count) as f64 / elapsed;
        rate.is_finite().then_some(rate)
    }
}

/// A run boundary to annotate in Grafana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunEvent {
    Start(u32),
    Stop(u32),
}

/// Detects run start/stop from successive polls (TODO 74 P5).
#[derive(Debug, Default)]
struct RunEventDetector {
    /// False until the first poll. That poll only records the state, so an
    /// Operator restarted mid-run does not emit a spurious start.
    initialised: bool,
    /// Run number of the run in progress at the previous poll (`None` = not running).
    prev_run: Option<u32>,
}

impl RunEventDetector {
    /// Feed the run in progress at this poll (`None` = not running) and get the
    /// events since the previous poll.
    fn update(&mut self, current_run: Option<u32>) -> Vec<RunEvent> {
        let prev_run = std::mem::replace(&mut self.prev_run, current_run);
        if !self.initialised {
            self.initialised = true;
            return Vec::new();
        }
        match (prev_run, current_run) {
            (None, Some(new)) => vec![RunEvent::Start(new)],
            (Some(old), None) => vec![RunEvent::Stop(old)],
            (Some(old), Some(new)) if old != new => {
                vec![RunEvent::Stop(old), RunEvent::Start(new)]
            }
            _ => Vec::new(),
        }
    }
}

/// Run number of the run in progress, judged from the source components:
/// `None` unless at least one source is online and Running. If the running
/// sources disagree, the largest run number wins (with a warning).
fn running_run_number(sources: &[&ComponentStatus]) -> Option<u32> {
    let mut numbers = Vec::new();
    for status in sources {
        if !status.online || status.state != ComponentState::Running {
            continue;
        }
        match status.run_number {
            Some(n) => numbers.push(n),
            // Not expected: a Running component always has a run config.
            None => warn!(
                component = %status.name,
                "Running source reports no run number; ignored for run_event"
            ),
        }
    }

    let max = numbers.iter().copied().max()?;
    if numbers.iter().any(|&n| n != max) {
        warn!(
            run_numbers = ?numbers,
            using = max,
            "Running sources disagree on the run number"
        );
    }
    Some(max)
}

/// Escape a Line Protocol tag value: `,` `=` and space get a backslash.
fn escape_tag_value(s: &str) -> String {
    s.replace(',', "\\,")
        .replace('=', "\\=")
        .replace(' ', "\\ ")
}

/// Escape a Line Protocol string field value: `\` and `"` get a backslash.
fn escape_string_field(s: &str) -> String {
    // Backslash first, so the backslashes added for quotes are not doubled.
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `run_event,exp={experiment} run={n}u,event="start"|"stop"`
///
/// An empty experiment name would make `exp=` an invalid tag, so the tag is
/// left out in that case.
fn run_event_line(experiment: &str, event: RunEvent) -> String {
    let (run, kind) = match event {
        RunEvent::Start(n) => (n, "start"),
        RunEvent::Stop(n) => (n, "stop"),
    };
    let tags = if experiment.is_empty() {
        String::new()
    } else {
        format!(",exp={}", escape_tag_value(experiment))
    };
    format!(
        "run_event{} run={}u,event=\"{}\"",
        tags,
        run,
        escape_string_field(kind)
    )
}

/// `channel_rate,module={m},channel={c} count={n}u[,rate={hz}]`
fn channel_rate_line(module: u32, channel: usize, count: u64, rate: Option<f64>) -> String {
    match rate {
        Some(hz) => format!(
            "channel_rate,module={},channel={} count={}u,rate={}",
            module, channel, count, hz
        ),
        None => format!(
            "channel_rate,module={},channel={} count={}u",
            module, channel, count
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- RateTracker (TODO 74 P2) -----------------------------------------

    #[test]
    fn first_sample_for_a_key_has_no_rate() {
        let mut tracker = RateTracker::default();
        assert_eq!(tracker.update((0, 3), 1000, Instant::now()), None);
    }

    #[test]
    fn rate_is_count_delta_over_measured_elapsed() {
        let mut tracker = RateTracker::default();
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(2500);
        let t2 = t1 + Duration::from_millis(2000);

        assert_eq!(tracker.update((0, 3), 1000, t0), None);
        // (4810 - 1000) / 2.5 s = 3810 / 2.5 = 1524.0 Hz — the measured
        // 2.5 s, not a nominal 2 s interval.
        assert_eq!(tracker.update((0, 3), 4810, t1), Some(1524.0));
        // Stored state moved to (4810, t1): (5810 - 4810) / 2.0 s = 500.0 Hz
        assert_eq!(tracker.update((0, 3), 5810, t2), Some(500.0));
    }

    #[test]
    fn counter_reset_gives_zero_and_rebases_on_the_new_count() {
        let mut tracker = RateTracker::default();
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(2);
        let t2 = t1 + Duration::from_secs(2);

        assert_eq!(tracker.update((1, 7), 5000, t0), None);
        // 120 < 5000: counter went back (new run) → 0.0, previous := 120
        assert_eq!(tracker.update((1, 7), 120, t1), Some(0.0));
        // (920 - 120) / 2 s = 400.0 Hz
        assert_eq!(tracker.update((1, 7), 920, t2), Some(400.0));
    }

    #[test]
    fn zero_elapsed_gives_no_rate() {
        let mut tracker = RateTracker::default();
        let t0 = Instant::now();

        assert_eq!(tracker.update((0, 3), 1000, t0), None);
        // Same instant: division by 0 s → omit the field
        assert_eq!(tracker.update((0, 3), 1500, t0), None);
    }

    #[test]
    fn keys_are_independent() {
        let mut tracker = RateTracker::default();
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(2);
        let t2 = t1 + Duration::from_secs(4);

        assert_eq!(tracker.update((0, 3), 1000, t0), None);
        assert_eq!(tracker.update((0, 4), 50, t0), None);
        // Same channel number on another module is a different key
        assert_eq!(tracker.update((1, 3), 9000, t1), None);
        // (0,3): (1600 - 1000) / 2 s = 300.0 Hz
        assert_eq!(tracker.update((0, 3), 1600, t1), Some(300.0));
        // (1,3) first seen at t1: (9800 - 9000) / 4 s = 200.0 Hz
        assert_eq!(tracker.update((1, 3), 9800, t2), Some(200.0));
        // (0,4) last seen at t0, skipped t1: (650 - 50) / (2 s + 4 s) = 100.0 Hz
        assert_eq!(tracker.update((0, 4), 650, t2), Some(100.0));
    }

    // --- RunEventDetector (TODO 74 P5) ------------------------------------

    #[test]
    fn first_poll_only_initialises() {
        // Operator restarted mid-run: no spurious start for run 52
        let mut running = RunEventDetector::default();
        assert_eq!(running.update(Some(52)), vec![]);
        assert_eq!(running.update(Some(52)), vec![]);

        // Operator started while idle: nothing either
        let mut idle = RunEventDetector::default();
        assert_eq!(idle.update(None), vec![]);
        assert_eq!(idle.update(None), vec![]);
    }

    #[test]
    fn not_running_to_running_emits_start() {
        let mut detector = RunEventDetector::default();
        detector.update(None);
        assert_eq!(detector.update(Some(53)), vec![RunEvent::Start(53)]);
        assert_eq!(detector.update(Some(53)), vec![]);
    }

    #[test]
    fn running_to_not_running_emits_stop_for_the_previous_run() {
        let mut detector = RunEventDetector::default();
        detector.update(Some(53));
        assert_eq!(detector.update(None), vec![RunEvent::Stop(53)]);
        assert_eq!(detector.update(None), vec![]);
    }

    #[test]
    fn run_number_change_while_running_emits_stop_then_start() {
        let mut detector = RunEventDetector::default();
        detector.update(Some(53));
        assert_eq!(
            detector.update(Some(54)),
            vec![RunEvent::Stop(53), RunEvent::Start(54)]
        );
    }

    // --- running_run_number -------------------------------------------------

    fn source(state: ComponentState, run_number: Option<u32>, online: bool) -> ComponentStatus {
        ComponentStatus {
            name: "reader".to_string(),
            address: "tcp://localhost:5560".to_string(),
            state,
            run_number,
            metrics: None,
            error: None,
            online,
            role: "source".to_string(),
        }
    }

    #[test]
    fn no_running_source_means_no_run() {
        let configured = source(ComponentState::Configured, Some(53), true);
        let armed = source(ComponentState::Armed, Some(53), true);
        assert_eq!(running_run_number(&[&configured, &armed]), None);
        assert_eq!(running_run_number(&[]), None);
    }

    #[test]
    fn one_running_source_gives_its_run_number() {
        let running = source(ComponentState::Running, Some(53), true);
        let configured = source(ComponentState::Configured, Some(41), true);
        assert_eq!(running_run_number(&[&configured, &running]), Some(53));
    }

    #[test]
    fn disagreeing_sources_give_the_max_run_number() {
        let a = source(ComponentState::Running, Some(54), true);
        let b = source(ComponentState::Running, Some(52), true);
        let c = source(ComponentState::Running, Some(53), true);
        assert_eq!(running_run_number(&[&a, &b, &c]), Some(54));
    }

    #[test]
    fn offline_or_numberless_sources_are_ignored() {
        let offline = source(ComponentState::Running, Some(99), false);
        let numberless = source(ComponentState::Running, None, true);
        let running = source(ComponentState::Running, Some(53), true);
        assert_eq!(running_run_number(&[&offline, &numberless]), None);
        assert_eq!(
            running_run_number(&[&offline, &numberless, &running]),
            Some(53)
        );
    }

    // --- Line protocol ------------------------------------------------------

    #[test]
    fn tag_value_escapes_comma_equals_and_space() {
        assert_eq!(escape_tag_value("N_Wall"), "N_Wall");
        assert_eq!(escape_tag_value("Si n,a=b"), r"Si\ n\,a\=b");
    }

    #[test]
    fn string_field_escapes_backslash_and_quote() {
        assert_eq!(escape_string_field("start"), "start");
        // a"b\c → a\"b\\c (backslash escaped first, so the quote's escape survives)
        assert_eq!(escape_string_field(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    #[test]
    fn run_event_lines() {
        assert_eq!(
            run_event_line("X730_ThGEM_Test", RunEvent::Start(53)),
            r#"run_event,exp=X730_ThGEM_Test run=53u,event="start""#
        );
        assert_eq!(
            run_event_line("Si + n", RunEvent::Stop(7)),
            r#"run_event,exp=Si\ +\ n run=7u,event="stop""#
        );
    }

    #[test]
    fn run_event_line_without_experiment_has_no_exp_tag() {
        // `exp=` with an empty value would be rejected by InfluxDB.
        assert_eq!(
            run_event_line("", RunEvent::Start(1)),
            r#"run_event run=1u,event="start""#
        );
    }

    #[test]
    fn channel_rate_lines() {
        assert_eq!(
            channel_rate_line(0, 3, 45234, Some(1523.5)),
            "channel_rate,module=0,channel=3 count=45234u,rate=1523.5"
        );
        // First sample: count only
        assert_eq!(
            channel_rate_line(1, 12, 800, None),
            "channel_rate,module=1,channel=12 count=800u"
        );
        // A bare number without a suffix is a float in line protocol
        assert_eq!(
            channel_rate_line(2, 0, 120, Some(0.0)),
            "channel_rate,module=2,channel=0 count=120u,rate=0"
        );
    }
}

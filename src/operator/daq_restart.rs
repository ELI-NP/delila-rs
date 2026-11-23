//! Restart the whole DAQ from the UI (TODO 91), with the guards that keep it
//! from losing data.
//!
//! Why: a restart between runs is the fix for a digitizer the Readers cannot
//! re-open (2026-11-10), a VX2730 stuck in `jesd_unlock` (2026-11-20) or a
//! dead FELib connection (−15); until now that needed ssh. A restart while
//! data still waits in a queue loses it (runs 45/46: 85 GB), so the request is
//! refused unless every one of these holds (CLAUDE.md, absolute rule):
//! - no run is active, Tune Up is off, no run-control command (start, stop,
//!   drain-first stop) is in progress, and no component is Armed or Running;
//! - the Recorder and the Merger answer and report `queue_bytes == 0`;
//! - the ROOT sink, when one is configured, answers on its HTTP port and
//!   reports `idle` with its record-path queues (sort, work, write) at 0
//!   (`/QueueStatus`, added to the three sinks for this).
//!
//! The restart itself runs in `scripts/restart_daq.sh`, spawned in its own
//! process group so that `stop_daq.sh` killing this Operator does not kill it.
//! The script checks the config before stopping anything and writes its
//! result to `logs/last_restart.json`, which the next Operator shows once.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use utoipa::ToSchema;

use crate::common::ComponentState;
use crate::operator::ComponentStatus;

/// One restart per this interval (a hardware fault does not go away with
/// more restarts).
pub const MIN_INTERVAL: Duration = Duration::from_secs(120);

/// A finished restart is reported in `/api/status` for this long.
pub const SHOW_RESULT_FOR_MS: i64 = 10 * 60 * 1000;

/// The script writes its result only after the new Operator answers, so the
/// new Operator re-reads the result file for this long after it starts.
pub const STARTUP_WATCH: Duration = Duration::from_secs(180);

/// How long an accepted restart blocks Configure / Arm / Start / Tune Up when
/// it neither stops this Operator nor reports a result (should not happen;
/// the preflight takes seconds).
pub const PENDING_TIMEOUT: Duration = Duration::from_secs(180);

/// What the ROOT sink said about its run file and record-path queues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkQueue {
    /// No `[network.root_sink]` in the config: nothing to check.
    NotConfigured,
    /// Configured, but its state is unknown (no HTTP port, no answer, or an
    /// answer that cannot be read). A restart is refused.
    Unknown(String),
    /// `/QueueStatus` of the sink.
    Report {
        writing: bool,
        sort: u64,
        work: u64,
        write: u64,
        disp: u64,
    },
}

/// Parse the sink's `/QueueStatus` text, e.g. `idle sort=0 work=0 write=0 disp=0`.
pub fn parse_queue_status(text: &str) -> Result<SinkQueue, String> {
    let mut words = text.split_whitespace();
    let writing = match words.next() {
        Some("idle") => false,
        Some("writing") => true,
        other => return Err(format!("unexpected sink state {:?} in {text:?}", other)),
    };
    let (mut sort, mut work, mut write, mut disp) = (None, None, None, None);
    for w in words {
        let (key, value) = w
            .split_once('=')
            .ok_or_else(|| format!("unexpected word {w:?} in {text:?}"))?;
        let n: u64 = value
            .parse()
            .map_err(|_| format!("not a count: {w:?} in {text:?}"))?;
        match key {
            "sort" => sort = Some(n),
            "work" => work = Some(n),
            "write" => write = Some(n),
            "disp" => disp = Some(n),
            _ => {}
        }
    }
    match (sort, work, write, disp) {
        (Some(sort), Some(work), Some(write), Some(disp)) => Ok(SinkQueue::Report {
            writing,
            sort,
            work,
            write,
            disp,
        }),
        _ => Err(format!("missing a queue in {text:?}")),
    }
}

/// Everything the guard looks at.
pub struct RestartCheck<'a> {
    /// Number of the run in progress, if any.
    pub run_active: Option<i32>,
    pub tuneup_active: bool,
    /// A run-control command holds the run-control lock.
    pub run_control_busy: bool,
    pub statuses: &'a [ComponentStatus],
    pub sink: SinkQueue,
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

/// Why the DAQ must not be restarted now; empty = it may.
pub fn refusal_reasons(c: &RestartCheck) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(run) = c.run_active {
        why.push(format!("run {run} is active — stop it first"));
    }
    if c.tuneup_active {
        why.push("Tune Up mode is active — stop Tune Up first".to_string());
    }
    if c.run_control_busy {
        why.push(
            "a run-control command (start, stop or drain-first stop) is still in progress"
                .to_string(),
        );
    }
    let busy: Vec<String> = c
        .statuses
        .iter()
        .filter(|s| matches!(s.state, ComponentState::Running | ComponentState::Armed))
        .map(|s| format!("{} ({})", s.name, s.state))
        .collect();
    if !busy.is_empty() {
        why.push(format!(
            "components still armed or running: {}",
            busy.join(", ")
        ));
    }
    // The writers' queues: data that is not on disk yet.
    for name in ["Merger", "Recorder"] {
        match c.statuses.iter().find(|s| s.name == name) {
            None => {}
            Some(s) if !s.online => why.push(format!(
                "{name} does not answer — cannot confirm that its queue is empty (check it over ssh)"
            )),
            Some(s) => match s.metrics.as_ref() {
                None => why.push(format!(
                    "{name} reports no metrics — cannot confirm that its queue is empty"
                )),
                Some(m) if m.queue_bytes > 0 => why.push(format!(
                    "{name} still has {} queued — wait until it is written",
                    mb(m.queue_bytes)
                )),
                Some(_) => {}
            },
        }
    }
    match &c.sink {
        SinkQueue::NotConfigured => {}
        SinkQueue::Unknown(e) => why.push(format!(
            "the ROOT sink's queue cannot be read ({e}) — cannot confirm that it is empty"
        )),
        SinkQueue::Report {
            writing,
            sort,
            work,
            write,
            ..
        } => {
            if *writing {
                why.push("the ROOT sink still has a run file open".to_string());
            }
            if sort + work + write > 0 {
                why.push(format!(
                    "the ROOT sink still has data queued (sort={sort} work={work} write={write})"
                ));
            }
        }
    }
    why
}

/// How long until another restart may be requested, if it is too soon.
pub fn wait_left(last: Option<Instant>, now: Instant) -> Option<Duration> {
    let last = last?;
    let since = now.saturating_duration_since(last);
    (since < MIN_INTERVAL).then(|| MIN_INTERVAL - since)
}

/// What `scripts/restart_daq.sh` wrote to `logs/last_restart.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RestartResult {
    /// Id given by the Operator that accepted the request.
    pub request_id: String,
    /// When the script started / finished (Unix ms).
    pub started_ms: i64,
    pub finished_ms: i64,
    /// Whether the DAQ is back.
    pub ok: bool,
    /// Where it ended: `preflight` (nothing was stopped), `stop`, `start`, `done`.
    pub stage: String,
    pub message: String,
    /// Tail of `operator.log` when the new Operator did not come up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
}

/// Read the result file; `None` if it is missing or unreadable.
pub fn read_result(path: &Path) -> Option<RestartResult> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Restart result file is not valid JSON — ignored");
            None
        }
    }
}

/// The result to report at `now_ms`: only a recent one.
pub fn recent(result: Option<RestartResult>, now_ms: i64) -> Option<RestartResult> {
    result.filter(|r| now_ms - r.finished_ms <= SHOW_RESULT_FOR_MS)
}

/// How this Operator was started, so the restart uses the same.
#[derive(Debug, Clone)]
pub struct RestartSettings {
    pub script: PathBuf,
    pub config_file: String,
    /// The Operator runs without MongoDB (start_daq.sh `--no-mongo`).
    pub no_mongo: bool,
    pub result_file: PathBuf,
    pub log_file: PathBuf,
    /// `[network.root_sink]`: `None` = no sink; `Some(None)` = a sink with
    /// `http_port = 0` (its queue cannot be read).
    pub sink_http_port: Option<Option<u16>>,
}

impl Default for RestartSettings {
    fn default() -> Self {
        Self {
            script: PathBuf::from("scripts/restart_daq.sh"),
            config_file: "config.toml".to_string(),
            no_mongo: false,
            result_file: PathBuf::from("logs/last_restart.json"),
            log_file: PathBuf::from("logs/restart_daq.log"),
            sink_http_port: None,
        }
    }
}

/// Restart bookkeeping in `AppState`.
pub struct RestartState {
    pub settings: RestartSettings,
    /// When the last restart was accepted (rate limit).
    pub last_request: Mutex<Option<Instant>>,
    /// The last result, shown in `/api/status` while recent.
    pub last_result: RwLock<Option<RestartResult>>,
    /// Set when a restart is accepted, cleared if the script ends while this
    /// Operator still runs (preflight failure). While set, Configure / Arm /
    /// Start / Tune Up are refused: nothing may start between the check and
    /// the stop.
    pub pending: AtomicBool,
}

impl RestartState {
    /// Pick up the result the previous Operator's restart left behind.
    pub fn new(settings: RestartSettings) -> Self {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let last = recent(read_result(&settings.result_file), now_ms);
        if let Some(ref r) = last {
            tracing::info!(ok = r.ok, stage = %r.stage, message = %r.message, "Last DAQ restart");
        }
        Self {
            settings,
            last_request: Mutex::new(None),
            last_result: RwLock::new(last),
            pending: AtomicBool::new(false),
        }
    }
}

impl Default for RestartState {
    fn default() -> Self {
        Self {
            settings: RestartSettings::default(),
            last_request: Mutex::new(None),
            last_result: RwLock::new(None),
            pending: AtomicBool::new(false),
        }
    }
}

/// Pick up the result of the restart that started this Operator: the script
/// writes it after this Operator answers (spawned at startup, ends after
/// [`STARTUP_WATCH`]).
pub async fn watch_result_after_start(state: std::sync::Arc<crate::operator::routes::AppState>) {
    let started = Instant::now();
    while started.elapsed() < STARTUP_WATCH {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let Some(r) = recent(read_result(&state.restart.settings.result_file), now_ms) else {
            continue;
        };
        let mut current = state.restart.last_result.write().await;
        if current.as_ref() != Some(&r) {
            tracing::info!(ok = r.ok, stage = %r.stage, message = %r.message, "DAQ restart result");
            *current = Some(r);
        }
    }
}

/// Ask the ROOT sink for its `/QueueStatus` (THttpServer `item.json`).
pub async fn fetch_sink_queue(port: Option<Option<u16>>) -> SinkQueue {
    let port = match port {
        None => return SinkQueue::NotConfigured,
        Some(None) => {
            return SinkQueue::Unknown(
                "[network.root_sink] http_port = 0: the sink has no HTTP".to_string(),
            )
        }
        Some(Some(p)) => p,
    };
    let url = format!("http://127.0.0.1:{port}/QueueStatus/item.json");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => return SinkQueue::Unknown(format!("HTTP client: {e}")),
    };
    let body: serde_json::Value = match client.get(&url).send().await {
        Ok(resp) => match resp.json().await {
            Ok(v) => v,
            Err(e) => return SinkQueue::Unknown(format!("{url}: {e}")),
        },
        Err(e) => return SinkQueue::Unknown(format!("{url}: {e}")),
    };
    match body.get("value").and_then(|v| v.as_str()) {
        Some(text) => parse_queue_status(text).unwrap_or_else(SinkQueue::Unknown),
        None => SinkQueue::Unknown(format!(
            "{url} has no queue status (sink built before TODO 91?)"
        )),
    }
}

/// Start the restart script in its own process group; returns the child so
/// the caller can wait for it (a preflight failure ends it while this
/// Operator still runs).
pub fn spawn(settings: &RestartSettings, request_id: &str) -> std::io::Result<std::process::Child> {
    use std::process::{Command, Stdio};
    if let Some(dir) = settings.log_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&settings.log_file)?;
    let mut cmd = Command::new("bash");
    cmd.arg(&settings.script)
        .arg("--config")
        .arg(&settings.config_file)
        .arg("--request-id")
        .arg(request_id)
        .arg("--result")
        .arg(&settings.result_file);
    if settings.no_mongo {
        cmd.arg("--no-mongo");
    }
    cmd.stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group: stop_daq.sh's pkill of this Operator, and signals
        // to its group, do not reach the script. The script ignores SIGHUP.
        cmd.process_group(0);
    }
    cmd.spawn()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ComponentMetrics;

    fn status(
        name: &str,
        state: ComponentState,
        online: bool,
        queue: Option<u64>,
    ) -> ComponentStatus {
        ComponentStatus {
            name: name.to_string(),
            address: String::new(),
            state,
            run_number: None,
            metrics: queue.map(|q| ComponentMetrics {
                queue_bytes: q,
                ..Default::default()
            }),
            error: None,
            online,
            role: "pipeline".to_string(),
        }
    }

    fn healthy() -> Vec<ComponentStatus> {
        vec![
            status("Reader-0", ComponentState::Configured, true, Some(0)),
            status("Merger", ComponentState::Configured, true, Some(0)),
            status("Recorder", ComponentState::Configured, true, Some(0)),
            status("Monitor", ComponentState::Configured, true, Some(0)),
        ]
    }

    fn idle_sink() -> SinkQueue {
        SinkQueue::Report {
            writing: false,
            sort: 0,
            work: 0,
            write: 0,
            disp: 0,
        }
    }

    fn check(statuses: &[ComponentStatus], sink: SinkQueue) -> Vec<String> {
        refusal_reasons(&RestartCheck {
            run_active: None,
            tuneup_active: false,
            run_control_busy: false,
            statuses,
            sink,
        })
    }

    #[test]
    fn a_quiet_daq_may_restart() {
        let s = healthy();
        assert!(check(&s, idle_sink()).is_empty());
        assert!(check(&s, SinkQueue::NotConfigured).is_empty());
    }

    #[test]
    fn an_active_run_tune_up_or_run_control_is_refused() {
        let s = healthy();
        let why = refusal_reasons(&RestartCheck {
            run_active: Some(12),
            tuneup_active: true,
            run_control_busy: true,
            statuses: &s,
            sink: idle_sink(),
        });
        assert_eq!(why.len(), 3, "{why:?}");
        assert!(why[0].contains("run 12"), "{why:?}");
        assert!(why[1].contains("Tune Up"), "{why:?}");
        assert!(why[2].contains("in progress"), "{why:?}");
    }

    #[test]
    fn armed_or_running_components_are_refused() {
        let mut s = healthy();
        s[0].state = ComponentState::Running;
        s[3].state = ComponentState::Armed;
        let why = check(&s, idle_sink());
        assert_eq!(why.len(), 1, "{why:?}");
        assert!(why[0].contains("Reader-0 (Running)"), "{why:?}");
        assert!(why[0].contains("Monitor (Armed)"), "{why:?}");
    }

    /// The absolute rule: data still queued is never thrown away by a restart.
    #[test]
    fn queued_data_in_the_recorder_or_merger_is_refused() {
        let mut s = healthy();
        s[2].metrics = Some(ComponentMetrics {
            queue_bytes: 12_300_000_000,
            ..Default::default()
        });
        let why = check(&s, idle_sink());
        assert_eq!(why.len(), 1, "{why:?}");
        assert!(
            why[0].contains("Recorder still has 12300.0 MB queued"),
            "{why:?}"
        );

        let mut s = healthy();
        s[1].metrics = Some(ComponentMetrics {
            queue_bytes: 1,
            ..Default::default()
        });
        assert!(check(&s, idle_sink())[0].starts_with("Merger"));
    }

    #[test]
    fn a_writer_whose_queue_is_unknown_is_refused() {
        let mut s = healthy();
        s[2].online = false;
        let why = check(&s, idle_sink());
        assert!(why[0].contains("Recorder does not answer"), "{why:?}");

        let mut s = healthy();
        s[1].metrics = None;
        let why = check(&s, idle_sink());
        assert!(why[0].contains("Merger reports no metrics"), "{why:?}");
    }

    #[test]
    fn the_monitor_and_sources_queues_do_not_block() {
        // The Monitor's queue is display-only (allowed to drop, CLAUDE.md).
        let mut s = healthy();
        s[3].metrics = Some(ComponentMetrics {
            queue_bytes: 5_000_000,
            ..Default::default()
        });
        s[0].metrics = Some(ComponentMetrics {
            queue_bytes: 1,
            ..Default::default()
        });
        assert!(check(&s, idle_sink()).is_empty());
    }

    #[test]
    fn a_sink_with_an_open_file_or_queued_data_is_refused() {
        let s = healthy();
        let why = check(
            &s,
            SinkQueue::Report {
                writing: true,
                sort: 0,
                work: 3,
                write: 1,
                disp: 9,
            },
        );
        assert_eq!(why.len(), 2, "{why:?}");
        assert!(why[0].contains("run file open"), "{why:?}");
        assert!(why[1].contains("sort=0 work=3 write=1"), "{why:?}");
        // The display queue alone does not block (display drops are allowed).
        let why = check(
            &s,
            SinkQueue::Report {
                writing: false,
                sort: 0,
                work: 0,
                write: 0,
                disp: 40,
            },
        );
        assert!(why.is_empty(), "{why:?}");
    }

    #[test]
    fn a_configured_sink_that_cannot_be_read_is_refused() {
        let s = healthy();
        let why = check(&s, SinkQueue::Unknown("connection refused".to_string()));
        assert_eq!(why.len(), 1, "{why:?}");
        assert!(why[0].contains("connection refused"), "{why:?}");
    }

    #[test]
    fn queue_status_text_is_parsed_strictly() {
        assert_eq!(
            parse_queue_status("idle sort=0 work=0 write=0 disp=0"),
            Ok(idle_sink())
        );
        assert_eq!(
            parse_queue_status("writing sort=1 work=2 write=3 disp=4"),
            Ok(SinkQueue::Report {
                writing: true,
                sort: 1,
                work: 2,
                write: 3,
                disp: 4
            })
        );
        assert!(parse_queue_status("starting").is_err());
        assert!(parse_queue_status("idle sort=0 work=0").is_err());
        assert!(parse_queue_status("idle sort=x work=0 write=0 disp=0").is_err());
        assert!(parse_queue_status("").is_err());
    }

    #[tokio::test]
    async fn no_sink_section_skips_the_check_and_no_port_refuses() {
        assert_eq!(fetch_sink_queue(None).await, SinkQueue::NotConfigured);
        assert!(matches!(
            fetch_sink_queue(Some(None)).await,
            SinkQueue::Unknown(_)
        ));
        // Nothing listens on this port: unknown, never "empty".
        assert!(matches!(
            fetch_sink_queue(Some(Some(1))).await,
            SinkQueue::Unknown(_)
        ));
    }

    #[test]
    fn one_restart_per_two_minutes() {
        let t0 = Instant::now();
        assert_eq!(wait_left(None, t0), None);
        let left = wait_left(Some(t0), t0 + Duration::from_secs(30));
        assert_eq!(left, Some(Duration::from_secs(90)));
        assert_eq!(wait_left(Some(t0), t0 + MIN_INTERVAL), None);
    }

    #[test]
    fn only_a_recent_result_is_reported() {
        let r = RestartResult {
            request_id: "a".into(),
            started_ms: 1_000,
            finished_ms: 2_000,
            ok: true,
            stage: "done".into(),
            message: "DAQ restarted".into(),
            log_tail: None,
        };
        assert_eq!(recent(Some(r.clone()), 2_000 + 60_000), Some(r.clone()));
        assert_eq!(recent(Some(r), 2_000 + SHOW_RESULT_FOR_MS + 1), None);
        assert_eq!(recent(None, 0), None);
    }

    #[test]
    fn the_result_file_round_trips_and_bad_files_are_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("last_restart.json");
        assert_eq!(read_result(&path), None);
        std::fs::write(
            &path,
            r#"{"request_id":"r1","started_ms":5,"finished_ms":9,"ok":false,"stage":"preflight","message":"config file does not load","log_tail":"line"}"#,
        )
        .expect("write");
        let r = read_result(&path).expect("result");
        assert!(!r.ok);
        assert_eq!(r.stage, "preflight");
        assert_eq!(r.log_tail.as_deref(), Some("line"));
        std::fs::write(&path, "not json").expect("write");
        assert_eq!(read_result(&path), None);
    }
}

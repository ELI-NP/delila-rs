//! `POST /api/daq/restart` — restart the whole DAQ from the UI (TODO 91).
//! The guard is `daq_restart::refusal_reasons`; the work is
//! `scripts/restart_daq.sh`, spawned in its own process group.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    Json,
};

use super::{ApiResponse, AppState};
use crate::operator::daq_restart::{self, RestartCheck};

/// Refusal for Configure / Arm / Start / Tune Up while a restart is pending.
pub(super) fn refuse_while_restarting(state: &AppState) -> Option<(StatusCode, Json<ApiResponse>)> {
    state.restart.pending.load(Ordering::Acquire).then(|| {
        (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "A DAQ restart is in progress — wait until the Operator is back",
            )),
        )
    })
}

/// Restart all DAQ components (stop_daq.sh + start_daq.sh, same config)
///
/// Refused unless no run is active, Tune Up is off, no run-control command is
/// in progress, no component is Armed or Running, the Recorder and the Merger
/// report empty queues, and the ROOT sink (if configured) reports `idle` with
/// empty record-path queues. At most one restart per 2 minutes. The Operator
/// itself restarts: poll `/api/status` until it answers again; the result is
/// reported there as `last_restart`.
#[utoipa::path(
    post,
    path = "/api/daq/restart",
    tag = "DAQ Control",
    responses(
        (status = 202, description = "Accepted: every component stops and starts again, the Operator too", body = ApiResponse),
        (status = 409, description = "Refused: a run, Tune Up, a command in progress, or data still queued", body = ApiResponse),
        (status = 429, description = "Refused: the last restart was less than 2 minutes ago", body = ApiResponse),
        (status = 500, description = "The restart script could not be started", body = ApiResponse)
    )
)]
pub(super) async fn restart_daq(
    State(state): State<Arc<AppState>>,
    client: Option<ConnectInfo<std::net::SocketAddr>>,
) -> (StatusCode, Json<ApiResponse>) {
    let from = client.map_or_else(|| "unknown".to_string(), |ConnectInfo(a)| a.to_string());
    let mut last = state.restart.last_request.lock().await;
    if let Some(left) = daq_restart::wait_left(*last, Instant::now()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::error(format!(
                "A DAQ restart was requested less than 2 minutes ago — wait {} s. \
                 If it did not help, check the hardware (clock cable, crate) before restarting again.",
                left.as_secs() + 1
            ))),
        );
    }
    if state.restart.pending.load(Ordering::Acquire) {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error("A DAQ restart is already in progress")),
        );
    }

    // Check-then-act under the run-control lock (TODO 58 H14): a concurrent
    // Start cannot slip in while the guard looks; `pending` covers the time
    // after this handler returns until stop_daq.sh stops this Operator.
    let run_guard = state.run_control_lock.try_lock();
    let statuses = state.client.get_all_status(&state.components).await;
    let sink = daq_restart::fetch_sink_queue(state.restart.settings.sink_http_port).await;
    let run_active = state
        .current_run
        .read()
        .await
        .as_ref()
        .map(|r| r.run_number);
    let why = daq_restart::refusal_reasons(&RestartCheck {
        run_active,
        tuneup_active: state.tuneup.is_active(),
        run_control_busy: run_guard.is_err(),
        statuses: &statuses,
        sink,
    });
    if !why.is_empty() {
        let message = format!("DAQ restart refused: {}", why.join("; "));
        tracing::warn!(%from, reason = %message, "DAQ restart refused");
        return (StatusCode::CONFLICT, Json(ApiResponse::error(message)));
    }

    let settings = &state.restart.settings;
    let request_id = chrono::Utc::now().timestamp_millis().to_string();
    let child = match daq_restart::spawn(settings, &request_id) {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(error = %e, script = %settings.script.display(), "Cannot start the DAQ restart script");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error(format!(
                    "Cannot start {}: {e}",
                    settings.script.display()
                ))),
            );
        }
    };
    *last = Some(Instant::now());
    state.restart.pending.store(true, Ordering::Release);
    tracing::warn!(
        %from,
        request_id,
        pid = child.id(),
        config = %settings.config_file,
        no_mongo = settings.no_mongo,
        "DAQ restart accepted from the UI: stop_daq.sh + start_daq.sh"
    );

    // The script re-launches itself detached and returns at once (reap it).
    // If a result for this request appears while this Operator still runs,
    // the restart did not happen (preflight failure): report it and allow runs
    // again. A restart that goes ahead stops this Operator first.
    let st = state.clone();
    let id = request_id.clone();
    tokio::spawn(async move {
        let mut child = child;
        let _ = tokio::task::spawn_blocking(move || child.wait()).await;
        let deadline = Instant::now() + daq_restart::PENDING_TIMEOUT;
        let result = loop {
            if let Some(r) = daq_restart::read_result(&st.restart.settings.result_file)
                .filter(|r| r.request_id == id)
            {
                break Some(r);
            }
            if Instant::now() >= deadline {
                break None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        };
        match &result {
            Some(r) => {
                tracing::warn!(ok = r.ok, stage = %r.stage, message = %r.message, "DAQ restart reported while this Operator still runs")
            }
            None => tracing::error!(
                request_id = %id,
                log = %st.restart.settings.log_file.display(),
                "DAQ restart neither stopped this Operator nor reported a result — see the restart log"
            ),
        }
        *st.restart.last_result.write().await = result;
        st.restart.pending.store(false, Ordering::Release);
    });

    (
        StatusCode::ACCEPTED,
        Json(ApiResponse::success(format!(
            "DAQ restart accepted (request {request_id}): every component stops and starts again \
             with {}. This page reconnects when the Operator is back.",
            settings.config_file
        ))),
    )
}

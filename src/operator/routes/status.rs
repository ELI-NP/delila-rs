//! DAQ control handlers (status, configure, arm, start, stop, reset, run_start)

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};

use crate::common::{Command, ComponentState, RunConfig};
use crate::config::FirmwareType;

use super::super::{
    ApiResponse, CommandResult, ConfigureRequest, CurrentRunInfo, RunStats, RunStatus,
    StartRequest, SystemState, SystemStatus,
};
use super::AppState;

/// A run-control phase did not complete in time, or a component went
/// offline / into Error while waiting (TODO 78).
///
/// Never 408: Chromium silently resends a POST answered with 408 on a reused
/// keep-alive connection (crbug 377581). On 10/22 that turned one failed
/// start into eight Reset → Configure → Arm → Start cycles, with no error in
/// the UI, and the resent request overwrote a Force Reset 5 ms after it ran.
/// 504 (a downstream component did not answer in time) is not retried, so
/// the error reaches the UI.
fn phase_failed(phase: &str, error: &str) -> (StatusCode, Json<ApiResponse>) {
    tracing::error!(phase, error, "Run control phase failed");
    (
        StatusCode::GATEWAY_TIMEOUT,
        Json(ApiResponse::error(format!(
            "{} phase failed: {}",
            phase, error
        ))),
    )
}

/// Components rejected a run-control phase's command (TODO 78: these
/// returns used to be silent in the Operator log).
fn phase_rejected(phase: &str, results: Vec<CommandResult>) -> (StatusCode, Json<ApiResponse>) {
    let failed = results
        .iter()
        .filter(|r| !r.success)
        .map(|r| format!("{}: {}", r.name, r.message))
        .collect::<Vec<_>>()
        .join("; ");
    tracing::error!(phase, failed = %failed, "Run control phase rejected by components");
    (
        StatusCode::BAD_REQUEST,
        Json(
            ApiResponse::error(format!("{} phase failed: {}", phase, failed)).with_results(results),
        ),
    )
}

/// Undo a run start that failed after the components left Idle (TODO 78 b).
///
/// A half-started system (Recorder / Merger / master Running, a slave still
/// Armed with its Reader retrying the hardware start every 5 s) is "Mixed",
/// which the UI could not act on at all. Reset — the same command as Force
/// Reset and run_start's Phase 0 — brings every component back to Idle and
/// stops the retrying Reader.
async fn roll_back_to_idle(
    state: &AppState,
    (status, Json(mut response)): (StatusCode, Json<ApiResponse>),
) -> (StatusCode, Json<ApiResponse>) {
    let outcome = match state
        .client
        .reset_all_sync(&state.components, state.config.reset_timeout_ms)
        .await
    {
        Ok(results) if results.iter().all(|r| r.success) => {
            tracing::warn!("Failed run start rolled back: all components reset to Idle");
            "all components were reset to Idle — fix the cause and press Start again".to_string()
        }
        Ok(results) => {
            let failed = results
                .iter()
                .filter(|r| !r.success)
                .map(|r| format!("{}: {}", r.name, r.message))
                .collect::<Vec<_>>()
                .join("; ");
            tracing::error!(failed = %failed, "Rollback Reset after a failed run start was rejected");
            format!("rollback Reset rejected ({}) — use Force Reset", failed)
        }
        Err(e) => {
            tracing::error!(error = %e, "Rollback Reset after a failed run start failed");
            format!("rollback Reset failed ({}) — use Force Reset", e)
        }
    };
    response.message = format!("{} — {}", response.message, outcome);
    (status, Json(response))
}

/// Start phase failed: say which sources never started, then roll back.
async fn start_phase_failed(
    state: &AppState,
    run_number: u32,
    (status, Json(mut response)): (StatusCode, Json<ApiResponse>),
) -> (StatusCode, Json<ApiResponse>) {
    let stuck: Vec<String> = state
        .client
        .get_all_status(&state.components)
        .await
        .into_iter()
        .filter(|s| s.role == "source" && s.state == ComponentState::Armed)
        .map(|s| s.name)
        .collect();
    if !stuck.is_empty() {
        response.message = format!(
            "{} — {} stayed Armed: its acquisition never started. For an S-IN / GPIO slave \
             this usually means the master's start signal is not arriving (check the \
             master's Run output and the cable); the CAEN error is in that Reader's log",
            response.message,
            stuck.join(", ")
        );
    }
    // TODO 78 (d): the Recorder may already have opened a file for this run.
    // It is kept (the Recorder never overwrites; a retry with the same
    // number writes a timestamped name).
    response.message = format!(
        "{} — run {} is not in the run history (a partial data file may exist and is kept)",
        response.message, run_number
    );
    roll_back_to_idle(state, (status, Json(response))).await
}

/// Get system and component status
#[utoipa::path(
    get,
    path = "/api/status",
    tag = "DAQ Control",
    responses(
        (status = 200, description = "System status", body = SystemStatus)
    )
)]
pub(super) async fn get_status(State(state): State<Arc<AppState>>) -> Json<SystemStatus> {
    let components = state.client.get_all_status(&state.components).await;
    let system_state = SystemState::from_components(&components);

    // Get current run info and update real-time values
    let run_info = {
        let cached = state.current_run.read().await.clone();
        if let Some(mut info) = cached {
            if info.status == RunStatus::Running {
                // Update elapsed time
                let now_ms = chrono::Utc::now().timestamp_millis();
                info.elapsed_secs = (now_ms - info.start_time) / 1000;

                // Update stats from Recorder metrics (authoritative source for recorded data)
                let recorder_metrics = components
                    .iter()
                    .find(|c| c.name == "Recorder")
                    .and_then(|c| c.metrics.as_ref());
                let (total_events, total_bytes) = recorder_metrics
                    .map(|m| (m.events_processed as i64, m.bytes_transferred as i64))
                    .unwrap_or((0, 0));
                let trigger_loss_count: i64 = components
                    .iter()
                    .filter(|c| c.role == "source")
                    .filter_map(|c| c.metrics.as_ref())
                    .map(|m| m.trigger_loss_count as i64)
                    .sum();
                let average_rate = if info.elapsed_secs > 0 {
                    total_events as f64 / info.elapsed_secs as f64
                } else {
                    0.0
                };

                info.stats = RunStats {
                    total_events,
                    total_bytes,
                    average_rate,
                    trigger_loss_count,
                };
            }
            Some(info)
        } else {
            None
        }
    };

    // Get next run number and last run info from MongoDB (for multi-client sync)
    let (next_run_number, last_run_info) = if let Some(ref repo) = state.run_repo {
        let next = repo
            .get_next_run_number_for_experiment(&state.config.experiment_name)
            .await
            .ok();
        let last = match repo
            .get_last_run_info_for_experiment(&state.config.experiment_name)
            .await
        {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!("Failed to get last_run_info: {}", e);
                None
            }
        };
        (next, last)
    } else {
        (None, None)
    };

    // Single atomic load — both fields agree by construction (no torn read).
    let (tuneup_mode, tuneup_digitizer_id) = state.tuneup.snapshot();

    Json(SystemStatus {
        components,
        system_state,
        run_info,
        experiment_name: state.config.experiment_name.clone(),
        next_run_number,
        last_run_info,
        tuneup_mode,
        tuneup_digitizer_id,
        monitor_http_port: state.config.monitor_http_port,
        stalled_sources: state.stalled_sources.read().await.clone(),
    })
}

/// Configure all components for a run
#[utoipa::path(
    post,
    path = "/api/configure",
    tag = "DAQ Control",
    request_body = ConfigureRequest,
    responses(
        (status = 200, description = "Configuration result", body = ApiResponse),
        (status = 400, description = "Invalid request", body = ApiResponse),
        (status = 504, description = "Reset phase did not complete", body = ApiResponse)
    )
)]
pub(super) async fn configure(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfigureRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    // Serialize run control — all guards below are check-then-act (TODO 58 H14).
    let _run_guard = state.run_control_lock.lock().await;
    // Guard: reject if Tune Up mode is active (TODO 58 M15 — configuring under
    // an active tuneup re-resets the digitizer mid-stream and strands the
    // tuneup flag).
    if state.tuneup.is_active() {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Cannot configure while Tune Up mode is active. Stop Tune Up first.",
            )),
        );
    }
    let run_config: RunConfig = request.into();
    let run_number = run_config.run_number;

    // Reset to Idle first (mirrors run_start Phase 0). Without this, a Configure
    // issued while already in the Configured state is rejected by the state machine
    // (Configured -> Configured is not a valid transition), which makes the aggregate
    // response.success false and silently skips the disk reload + ApplyDigitizerConfig
    // push below. Command::Reset is idempotent from any state.
    match state
        .client
        .reset_all_sync(&state.components, state.config.reset_timeout_ms)
        .await
    {
        Err(e) => return phase_failed("Reset", &e),
        Ok(results) if results.iter().any(|r| !r.success) => {
            return phase_rejected("Reset", results)
        }
        Ok(_) => {}
    }

    let results = state
        .client
        .configure_all(&state.components, run_config)
        .await;
    if results.iter().any(|r| !r.success) {
        // Left as is (possibly Mixed): Configure is a pre-flight, not a run.
        // The UI offers Reset in Mixed (TODO 78 c).
        return phase_rejected("Configure", results);
    }

    let mut response =
        ApiResponse::success(format!("Configure command sent for run {}", run_number))
            .with_results(results);

    let status = if response.success {
        // Reload configs from disk so edits since Operator start take effect
        state.reload_digitizer_configs();
        // Push digitizer configs to remote Readers. TODO 58 M13: a Reader/FW
        // rejection here used to be warn-only + HTTP 200 — completely silent
        // in the UI while the digitizer ran with the wrong settings.
        let mut apply_failures: Vec<String> = Vec::new();
        // Non-fatal: the Reader clamps an AMax sweep wider than the firmware's
        // channel-page map. Configure is the automatic path, so without this the
        // skipped channels would only ever appear in the Reader's own log.
        let mut amax_notes: Vec<String> = Vec::new();
        for comp in &state.components {
            if comp.is_digitizer {
                if let Some(source_id) = comp.source_id {
                    let Some(config) = state.digitizer_configs.get(&source_id) else {
                        // TODO 58 M13: this skip was silent.
                        tracing::warn!(
                            source_id,
                            name = %comp.name,
                            "No digitizer config loaded for this source — Apply skipped"
                        );
                        continue;
                    };
                    let cfg = config.value().clone();
                    drop(config);
                    // X743Std: skip the redundant push — `configure_all` above already
                    // applied this config via the Reader's Configure path (the Reader
                    // keeps its `dig_config` in sync, so Configure re-applies the current
                    // config). A second apply within ~2s destabilizes libCAENDigitizer
                    // and can SIGSEGV after Start. Same guard as run_start Phase 1.5
                    // (see TODO/48). Use POST /api/digitizers/{id}/apply to push a
                    // direct on-disk edit to an X743 reader without a full reconfigure.
                    if cfg.firmware == FirmwareType::X743Std {
                        tracing::info!(
                            source_id,
                            name = %comp.name,
                            "X743Std: skipping redundant Apply (configure_all already applied identical config)"
                        );
                        continue;
                    }
                    if let Some(note) = crate::reader::caen::amax_registers::channel_clamp_note(
                        cfg.firmware,
                        cfg.num_channels,
                    ) {
                        tracing::warn!(source_id, name = %comp.name, note = %note, "AMax clamp");
                        amax_notes.push(format!("{}: {}", comp.name, note));
                    }
                    tracing::info!(
                        source_id,
                        name = %comp.name,
                        "Pushing digitizer config to Reader"
                    );
                    match state
                        .client
                        .send_command(&comp.address, &Command::ApplyDigitizerConfig(Box::new(cfg)))
                        .await
                    {
                        Ok(resp) if resp.success => {}
                        Ok(resp) => {
                            tracing::error!(
                                source_id,
                                error = %resp.message,
                                "Reader rejected digitizer config"
                            );
                            apply_failures.push(format!("{}: {}", comp.name, resp.message));
                        }
                        Err(e) => {
                            tracing::error!(
                                source_id,
                                error = %e,
                                "Failed to send ApplyDigitizerConfig command"
                            );
                            apply_failures.push(format!("{}: {}", comp.name, e));
                        }
                    }
                }
            }
        }
        if apply_failures.is_empty() {
            if !amax_notes.is_empty() {
                response.message = format!("{} — {}", response.message, amax_notes.join("; "));
            }
            StatusCode::OK
        } else {
            response.success = false;
            response.message = format!(
                "Configure succeeded but digitizer config apply FAILED: {}",
                apply_failures.join("; ")
            );
            StatusCode::BAD_GATEWAY
        }
    } else {
        StatusCode::BAD_REQUEST
    };

    (status, Json(response))
}

/// Arm all components
#[utoipa::path(
    post,
    path = "/api/arm",
    tag = "DAQ Control",
    responses(
        (status = 200, description = "Arm result", body = ApiResponse),
        (status = 400, description = "Invalid state transition", body = ApiResponse)
    )
)]
pub(super) async fn arm(State(state): State<Arc<AppState>>) -> (StatusCode, Json<ApiResponse>) {
    // Serialize run control (TODO 58 H14).
    let _run_guard = state.run_control_lock.lock().await;
    // Guard: reject if Tune Up mode is active (TODO 58 M15).
    if state.tuneup.is_active() {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Cannot arm while Tune Up mode is active. Stop Tune Up first.",
            )),
        );
    }
    let results = state.client.arm_all(&state.components).await;

    let response = ApiResponse::success("Arm command sent").with_results(results);

    let status = if response.success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    (status, Json(response))
}

/// Start data acquisition
///
/// If the system is in Configured state, this will automatically arm first,
/// then start. If already Armed, it will just start.
/// The run_number is passed at start time to allow changing it without re-configuring hardware.
#[utoipa::path(
    post,
    path = "/api/start",
    tag = "DAQ Control",
    request_body = StartRequest,
    responses(
        (status = 200, description = "Start result", body = ApiResponse),
        (status = 400, description = "Invalid state transition", body = ApiResponse),
        (status = 504, description = "Arm or Start did not complete — all components were reset to Idle", body = ApiResponse),
        (status = 507, description = "Disk below the hard free-space watermark, or a write failure still latched", body = ApiResponse)
    )
)]
pub(super) async fn start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    // Serialize run control (TODO 58 H14).
    let _run_guard = state.run_control_lock.lock().await;
    // Guard: reject if Tune Up mode is active
    if state.tuneup.is_active() {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Cannot start a run while Tune Up mode is active. Stop Tune Up first.",
            )),
        );
    }

    let run_number = request.run_number;
    let comment = request.comment;

    // Check current state
    let components = state.client.get_all_status(&state.components).await;
    let system_state = SystemState::from_components(&components);

    // TODO 69: no run on a disk below its hard watermark, and none while a
    // previous write failure is still latched — the run guard would stop it
    // within seconds.
    if let Some(msg) = crate::operator::run_guard::start_refusal(&components)
        .or_else(|| crate::operator::run_guard::latched_write_failure(&components))
    {
        tracing::error!("{}", msg);
        return (
            StatusCode::INSUFFICIENT_STORAGE,
            Json(ApiResponse::error(msg)),
        );
    }

    // If Configured, arm first. A failure from here on rolls back to Idle
    // (TODO 78 b) — a partly armed / partly running system is Mixed.
    if system_state == SystemState::Configured {
        match state
            .client
            .arm_all_sync(&state.components, state.config.arm_timeout_ms)
            .await
        {
            Err(e) => return roll_back_to_idle(&state, phase_failed("Auto-arm", &e)).await,
            Ok(results) if results.iter().any(|r| !r.success) => {
                return roll_back_to_idle(&state, phase_rejected("Auto-arm", results)).await
            }
            Ok(_) => {}
        }
    }

    // Start with the run number
    let start_result = state
        .client
        .start_all_sync(&state.components, run_number, state.config.start_timeout_ms)
        .await;

    let mut response = match start_result {
        Err(e) => return start_phase_failed(&state, run_number, phase_failed("Start", &e)).await,
        Ok(results) if results.iter().any(|r| !r.success) => {
            return start_phase_failed(&state, run_number, phase_rejected("Start", results)).await
        }
        Ok(results) => ApiResponse::success(format!("Start command sent for run {}", run_number))
            .with_results(results),
    };

    let status = if response.success {
        let exp_name = &state.config.experiment_name;
        // TODO 58 M16: MongoDB persistence failures were warn-only + HTTP 200 —
        // run-history gaps were invisible to the operator. The run itself still
        // starts (DAQ > bookkeeping), but the response must say so.
        let mut mongo_warnings: Vec<String> = Vec::new();

        // Record run start in MongoDB and update current_run
        if let Some(ref repo) = state.run_repo {
            let mongo_start = std::time::Instant::now();
            match repo
                .start_run(run_number as i32, exp_name, &comment, None)
                .await
            {
                Ok(doc) => {
                    tracing::info!("MongoDB start_run took {:?}", mongo_start.elapsed());
                    let info = CurrentRunInfo::from_document(&doc);
                    *state.current_run.write().await = Some(info);
                }
                Err(e) => {
                    tracing::error!("Failed to record run start in MongoDB: {}", e);
                    mongo_warnings.push(format!("run start not persisted: {}", e));
                    // Still set current_run for in-memory tracking
                    *state.current_run.write().await = Some(CurrentRunInfo {
                        run_number: run_number as i32,
                        exp_name: exp_name.clone(),
                        comment: comment.clone(),
                        start_time: chrono::Utc::now().timestamp_millis(),
                        elapsed_secs: 0,
                        status: RunStatus::Running,
                        stats: RunStats::default(),
                        notes: Vec::new(),
                    });
                }
            }
        } else {
            // TODO 58 M17: without MongoDB the in-memory current_run tracking
            // used to hang off the digitizer_repo branch below — with run
            // history disabled but a digitizer repo present, current_run was
            // never set and the UI showed no active run. Mirror run_start.
            *state.current_run.write().await = Some(CurrentRunInfo {
                run_number: run_number as i32,
                exp_name: exp_name.clone(),
                comment: comment.clone(),
                start_time: chrono::Utc::now().timestamp_millis(),
                elapsed_secs: 0,
                status: RunStatus::Running,
                stats: RunStats::default(),
                notes: Vec::new(),
            });
        }

        // Create digitizer config snapshot for this run
        if let Some(ref digitizer_repo) = state.digitizer_repo {
            let configs: Vec<_> = state
                .digitizer_configs
                .iter()
                .map(|r| r.value().clone())
                .collect();
            if !configs.is_empty() {
                if let Err(e) = digitizer_repo
                    .create_run_snapshot(run_number as i32, exp_name, configs)
                    .await
                {
                    tracing::error!("Failed to create config snapshot: {}", e);
                    mongo_warnings.push(format!("config snapshot not persisted: {}", e));
                }
            } else {
                tracing::warn!(
                    run_number,
                    "digitizer_configs is empty — config snapshot skipped. \
                     Check that config_file paths in TOML are correct and \
                     operator is started from the project root."
                );
            }
        }
        // TODO 58 M16: surface persistence failures in the HTTP response.
        if !mongo_warnings.is_empty() {
            response.message = format!(
                "{} — WARNING, run history incomplete: {}",
                response.message,
                mongo_warnings.join("; ")
            );
        }
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    (status, Json(response))
}

/// Stop data acquisition
#[utoipa::path(
    post,
    path = "/api/stop",
    tag = "DAQ Control",
    responses(
        (status = 200, description = "Stop result", body = ApiResponse),
        (status = 400, description = "Invalid state transition", body = ApiResponse)
    )
)]
pub(super) async fn stop(State(state): State<Arc<AppState>>) -> (StatusCode, Json<ApiResponse>) {
    // Serialize run control (TODO 58 H14).
    let _run_guard = state.run_control_lock.lock().await;
    // Guard: reject if Tune Up mode is active (TODO 58 M15 — /api/stop during a
    // tuneup used to strand the tuneup flag, deadlocking later starts on 409).
    if state.tuneup.is_active() {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Tune Up mode is active — use the Tune Up stop endpoint instead of /api/stop.",
            )),
        );
    }
    let results = state.client.stop_all(&state.components).await;

    let mut response = ApiResponse::success("Stop command sent").with_results(results);
    finish_run_bookkeeping(&state, &mut response, None).await;

    let status = if response.success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    (status, Json(response))
}

/// Run-end bookkeeping shared by `/api/stop` and the drain-first stop
/// (TODO 68): Mongo run record, ELOG post, clearing `current_run`. Always
/// runs to completion even after partial component failures — the UI must
/// not think the run is still active.
async fn finish_run_bookkeeping(
    state: &Arc<AppState>,
    response: &mut ApiResponse,
    stop_reason: Option<&str>,
) {
    let current_run = state.current_run.read().await.clone();

    if let (Some(ref repo), Some(mut run_info)) = (&state.run_repo, current_run) {
        // Calculate final elapsed time
        let now_ms = chrono::Utc::now().timestamp_millis();
        run_info.elapsed_secs = (now_ms - run_info.start_time) / 1000;

        // Get final stats from components. Recorder is the authoritative source
        // for recorded events/bytes (TODO 58 H12) — summing every component
        // counts the same events ~4x (Reader+Merger+Recorder+Monitor) and that
        // inflated number was persisted to MongoDB/ELOG. Mirrors get_status.
        let components = state.client.get_all_status(&state.components).await;
        let recorder_metrics = components
            .iter()
            .find(|c| c.name == "Recorder")
            .and_then(|c| c.metrics.as_ref());
        let (total_events, total_bytes) = recorder_metrics
            .map(|m| (m.events_processed as i64, m.bytes_transferred as i64))
            .unwrap_or((0, 0));
        let trigger_loss_count: i64 = components
            .iter()
            .filter(|c| c.role == "source")
            .filter_map(|c| c.metrics.as_ref())
            .map(|m| m.trigger_loss_count as i64)
            .sum();
        let average_rate = if run_info.elapsed_secs > 0 {
            total_events as f64 / run_info.elapsed_secs as f64
        } else {
            0.0
        };

        let stats = RunStats {
            total_events,
            total_bytes,
            average_rate,
            trigger_loss_count,
        };

        // TODO 58 L9: a partial stop failure used to be recorded as Completed —
        // record Error so the run history reflects the abnormal end.
        let final_status = if response.success {
            RunStatus::Completed
        } else {
            RunStatus::Error
        };

        if let Err(e) = repo
            .end_run(
                run_info.run_number,
                &run_info.exp_name,
                final_status,
                stats.clone(),
                stop_reason.map(str::to_owned),
            )
            .await
        {
            // TODO 58 M16: persistence failure must reach the operator, not
            // just the log.
            tracing::error!("Failed to record run end in MongoDB: {}", e);
            response.message = format!(
                "{} — WARNING, run history incomplete: run end not persisted: {}",
                response.message, e
            );
        }

        // Post to ELOG (fire-and-forget, must not block stop)
        if let Some(ref elog_config) = state.config.elog {
            let elog_config = elog_config.clone();
            let run_number = run_info.run_number;
            let exp_name = run_info.exp_name.clone();
            let comment = match stop_reason {
                Some(reason) => format!("{} [stop: {}]", run_info.comment, reason),
                None => run_info.comment.clone(),
            };
            let elapsed = run_info.elapsed_secs;
            let stats = stats.clone();
            tokio::spawn(async move {
                crate::operator::elog::post_run_summary(
                    &elog_config,
                    run_number,
                    &exp_name,
                    &comment,
                    elapsed,
                    &stats,
                )
                .await;
            });
        }
    }

    // Always clear current run
    *state.current_run.write().await = None;
}

/// Drain-first stop (TODO 68): stop the sources, let the downstream
/// components drain their backlog to disk while still Running, then stop
/// them. A plain `/api/stop` would move Merger/Recorder out of Running with
/// the backlog still queued — and both discard non-Running data as an
/// accepted Stop tail, which is exactly the data this path exists to save.
///
/// Caller must hold `state.run_control_lock`.
pub(crate) async fn drain_first_stop(
    state: &Arc<AppState>,
    reason: &str,
    timeout_ms: u64,
) -> (StatusCode, Json<ApiResponse>) {
    // Same Tune Up guard as /api/stop (TODO 58 M15).
    if state.tuneup.is_active() {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Tune Up mode is active — use the Tune Up stop endpoint instead.",
            )),
        );
    }

    tracing::warn!(reason, timeout_ms, "Drain-first stop initiated");

    let (sources, sinks): (Vec<_>, Vec<_>) = state
        .components
        .iter()
        .cloned()
        .partition(|c| c.source_id.is_some());

    // 1. Close the tap: stop the readers (HW disarm), wait until they are
    //    actually out of Running — a Reader acks Stop before its pipeline
    //    flushes the final EOS, so this wait must precede the drain wait.
    let mut results = state.client.stop_all(&sources).await;
    let sources_stopped = results.iter().all(|r| r.success);
    if let Err(e) = state
        .client
        .wait_for_state(&sources, crate::common::ComponentState::Configured, 10_000)
        .await
    {
        tracing::error!(error = %e, "Sources did not reach Configured after stop — draining anyway");
    }

    // 2. Drain: Merger/Recorder stay Running, so the backlog flows to disk
    //    through the normal path and the readers' EOS closes the file.
    let drain_result = state
        .client
        .wait_for_backlog_drained(&sinks, timeout_ms)
        .await;

    // 3. Stop the downstream (ascending pipeline order preserved).
    let sink_results = state.client.stop_all(&sinks).await;
    let sinks_stopped = sink_results.iter().all(|r| r.success);
    results.extend(sink_results);

    let mut response = match &drain_result {
        Ok(()) => ApiResponse::success(format!(
            "Drain-first stop complete ({reason}) — backlog fully written"
        )),
        Err(residual) => {
            tracing::error!(residual = %residual.summary(), "Drain-first stop lost residual backlog");
            ApiResponse::error(format!(
                "Drain-first stop ({reason}) — DRAIN INCOMPLETE: {}",
                residual.summary()
            ))
        }
    }
    .with_results(results);
    // with_results() recomputes `success` from the component stop results
    // alone — an incomplete drain must stay a failure even when every Stop
    // command itself succeeded.
    if drain_result.is_err() || !(sources_stopped && sinks_stopped) {
        response.success = false;
    }

    // 4. Bookkeeping with the reason (and residual, if any) — a timed-out
    //    drain records the run as Error via response.success = false.
    let stop_reason = match &drain_result {
        Ok(()) => reason.to_string(),
        Err(residual) => format!("{reason}; {}", residual.summary()),
    };
    finish_run_bookkeeping(state, &mut response, Some(&stop_reason)).await;

    let status = if response.success {
        StatusCode::OK
    } else {
        StatusCode::BAD_GATEWAY
    };
    (status, Json(response))
}

/// Immediate stop after a write failure (TODO 69): nothing is drained —
/// the writer has already stopped writing this run, so the only thing that
/// limits the loss is how fast the sources stop. `stop_all` stops them first
/// (ascending pipeline order). The run is recorded as Error with the reason
/// and the final count of events that never reached the disk.
///
/// Caller must hold `state.run_control_lock`.
pub(crate) async fn failure_stop(
    state: &Arc<AppState>,
    reason: &str,
) -> (StatusCode, Json<ApiResponse>) {
    // Same Tune Up guard as /api/stop (TODO 58 M15).
    if state.tuneup.is_active() {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Tune Up mode is active — use the Tune Up stop endpoint instead.",
            )),
        );
    }

    tracing::error!(reason, "Failure stop initiated — data is not being written");
    let results = state.client.stop_all(&state.components).await;

    // Final loss, now that the sources are stopped.
    let unwritten: u64 = state
        .client
        .get_all_status(&state.components)
        .await
        .iter()
        .filter_map(|s| s.metrics.as_ref())
        .map(|m| m.unwritten_events)
        .sum();
    let stop_reason = format!("{reason}; {unwritten} events not written in total");

    let mut response = ApiResponse::error(format!("Run stopped by the run guard: {stop_reason}"))
        .with_results(results);
    // with_results() recomputes `success` from the stop results alone; a run
    // that lost data is an Error in the run history whatever the stop did.
    response.success = false;
    finish_run_bookkeeping(state, &mut response, Some(&stop_reason)).await;

    (StatusCode::BAD_GATEWAY, Json(response))
}

/// Stop the run by draining the pipeline first (TODO 68)
#[utoipa::path(
    post,
    path = "/api/stop_drain",
    tag = "DAQ Control",
    responses(
        (status = 200, description = "Drain-first stop complete, backlog fully written", body = ApiResponse),
        (status = 409, description = "Tune Up active", body = ApiResponse),
        (status = 502, description = "Drain incomplete or component stop failed", body = ApiResponse)
    )
)]
pub(super) async fn stop_drain(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<ApiResponse>) {
    // Serialize run control (TODO 58 H14).
    let _run_guard = state.run_control_lock.lock().await;
    let timeout_ms = state.config.drain_stop_timeout_secs * 1000;
    drain_first_stop(&state, "operator drain-stop request", timeout_ms).await
}

/// Reset all components to Idle state
#[utoipa::path(
    post,
    path = "/api/reset",
    tag = "DAQ Control",
    responses(
        (status = 200, description = "Reset result", body = ApiResponse)
    )
)]
pub(super) async fn reset(State(state): State<Arc<AppState>>) -> (StatusCode, Json<ApiResponse>) {
    // Serialize run control (TODO 58 H14). Reset itself stays unguarded by
    // state on purpose — it is the operator's escape hatch (UI "Force Reset").
    let _run_guard = state.run_control_lock.lock().await;

    // If a run is active, close its record as Aborted BEFORE resetting —
    // otherwise the MongoDB doc stays "running" forever and current_run
    // leaks into the next session (TODO 58 H13).
    let aborted_run = state.current_run.write().await.take();
    if let (Some(ref repo), Some(mut run_info)) = (&state.run_repo, aborted_run) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        run_info.elapsed_secs = (now_ms - run_info.start_time) / 1000;
        if let Err(e) = repo
            .end_run(
                run_info.run_number,
                &run_info.exp_name,
                RunStatus::Aborted,
                run_info.stats.clone(),
                None,
            )
            .await
        {
            tracing::warn!(
                run_number = run_info.run_number,
                "Failed to record aborted run in MongoDB: {}",
                e
            );
        } else {
            tracing::info!(
                run_number = run_info.run_number,
                "Run aborted by reset — recorded as Aborted"
            );
        }
    }

    let results = state.client.reset_all(&state.components).await;

    let response = ApiResponse::success("Reset command sent").with_results(results);

    let status = if response.success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    (status, Json(response))
}

/// Start a run with full auto-cycle synchronization
///
/// This endpoint performs the complete run startup sequence:
/// 0. Reset all components (ensures clean state)
/// 1. Configure all components (with sync)
/// 2. Arm all components (with sync - waits for all to be Armed)
/// 3. Start all components (with sync)
///
/// Each phase waits for all components to reach the expected state
/// before proceeding, with configurable timeouts.
/// Config is always freshly applied, guaranteeing parameter consistency.
#[utoipa::path(
    post,
    path = "/api/run/start",
    tag = "DAQ Control",
    request_body = ConfigureRequest,
    responses(
        (status = 200, description = "Run started successfully", body = ApiResponse),
        (status = 400, description = "Failed to start run", body = ApiResponse),
        (status = 409, description = "System is Running or in Tune Up mode", body = ApiResponse),
        (status = 502, description = "A Reader rejected its digitizer config (before Arm)", body = ApiResponse),
        (status = 504, description = "A phase did not complete — after Reset, all components were reset to Idle", body = ApiResponse),
        (status = 507, description = "Disk below the hard free-space watermark", body = ApiResponse)
    )
)]
pub(super) async fn run_start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfigureRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    // Serialize run control (TODO 58 H14).
    let _run_guard = state.run_control_lock.lock().await;
    // Guard: reject if Tune Up mode is active
    if state.tuneup.is_active() {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Cannot start a run while Tune Up mode is active. Stop Tune Up first.",
            )),
        );
    }

    // Guard: reject if system is Running
    let components = state.client.get_all_status(&state.components).await;
    let system_state = SystemState::from_components(&components);
    if system_state == SystemState::Running {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "System is currently Running. Stop the run first before starting a new one.",
            )),
        );
    }

    // TODO 69: no run on a disk below its hard watermark (the run guard
    // would stop it within seconds). A latched write failure is no reason to
    // refuse here: the Reset/Configure below clears it.
    if let Some(msg) = crate::operator::run_guard::start_refusal(&components) {
        tracing::error!("{}", msg);
        return (
            StatusCode::INSUFFICIENT_STORAGE,
            Json(ApiResponse::error(msg)),
        );
    }

    let run_config: RunConfig = request.clone().into();
    let run_number = run_config.run_number;
    let comment = request.comment.clone();

    // Phase 0: Reset (ensure clean state — idempotent if already Idle)
    let reset_result = state
        .client
        .reset_all_sync(&state.components, state.config.reset_timeout_ms)
        .await;

    match reset_result {
        Err(e) => return phase_failed("Reset", &e),
        Ok(results) if results.iter().any(|r| !r.success) => {
            return phase_rejected("Reset", results)
        }
        Ok(_) => {}
    }

    // Phase 1: Configure
    let configure_result = state
        .client
        .configure_all_sync(
            &state.components,
            run_config,
            state.config.configure_timeout_ms,
        )
        .await;

    // From here on a failure rolls back to Idle (TODO 78 b): the components
    // are no longer in one state, and the UI could not act on "Mixed".
    match configure_result {
        Err(e) => return roll_back_to_idle(&state, phase_failed("Configure", &e)).await,
        Ok(results) if results.iter().any(|r| !r.success) => {
            return roll_back_to_idle(&state, phase_rejected("Configure", results)).await
        }
        Ok(_) => {}
    }

    // Reload digitizer configs from disk so Phase 1.5 uses fresh values.
    // Without this, edits to JSON files (e.g. fine_ts_mode) after Operator start
    // would be overwritten by stale in-memory configs.
    state.reload_digitizer_configs();

    // Phase 1.5: Apply digitizer configs to remote Readers
    // This pushes configs over ZMQ so remote Readers don't need local config files
    //
    // X743Std: skipped — same reason as tuneup_start. `configure_all_sync` above
    // already invoked apply_config_standard() via the Reader's Configure path
    // with identical config. A second apply within ~2s (Reset + full reconfigure
    // + ADC cal) destabilizes libCAENDigitizer.so and triggers SIGSEGV after
    // SWStartAcquisition. See TODO/48_v1743_tuneup_double_apply_crash.md.
    {
        for comp in &state.components {
            if comp.is_digitizer {
                if let Some(source_id) = comp.source_id {
                    // digitizer_id == source_id by convention.
                    // Clone the config so we don't hold a DashMap shard guard
                    // across the .send_command().await below.
                    let config = match state
                        .digitizer_configs
                        .get(&source_id)
                        .map(|r| r.value().clone())
                    {
                        Some(c) => c,
                        None => {
                            // TODO 58 M13: this skip was silent.
                            tracing::warn!(
                                source_id,
                                name = %comp.name,
                                "No digitizer config loaded for this source — Phase 1.5 Apply skipped"
                            );
                            continue;
                        }
                    };
                    if config.firmware == FirmwareType::X743Std {
                        tracing::info!(
                            source_id,
                            name = %comp.name,
                            "X743Std: skipping redundant Phase 1.5 Apply (configure_all_sync already applied identical config)"
                        );
                        continue;
                    }
                    tracing::info!(
                        source_id,
                        name = %comp.name,
                        "Pushing digitizer config to Reader"
                    );
                    match state
                        .client
                        .send_command(
                            &comp.address,
                            &Command::ApplyDigitizerConfig(Box::new(config)),
                        )
                        .await
                    {
                        Ok(resp) if resp.success => {
                            tracing::info!(
                                source_id,
                                params = resp.message,
                                "Digitizer config applied successfully"
                            );
                        }
                        // TODO 58 M13: an FW rejection here was warn-only and
                        // the sequence continued into Arm/Start — the run then
                        // took data with the wrong (post-reset) settings.
                        // Abort before Arm instead.
                        Ok(resp) => {
                            tracing::error!(
                                source_id,
                                error = %resp.message,
                                "Reader rejected digitizer config — aborting run start"
                            );
                            return (
                                StatusCode::BAD_GATEWAY,
                                Json(ApiResponse::error(format!(
                                    "{} rejected its digitizer config: {} — run start aborted \
                                     before Arm. Fix the config and retry.",
                                    comp.name, resp.message
                                ))),
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                source_id,
                                error = %e,
                                "Failed to send ApplyDigitizerConfig command — aborting run start"
                            );
                            return (
                                StatusCode::BAD_GATEWAY,
                                Json(ApiResponse::error(format!(
                                    "Failed to push digitizer config to {}: {} — run start \
                                     aborted before Arm.",
                                    comp.name, e
                                ))),
                            );
                        }
                    }
                }
            }
        }
    }

    // Register channels with Monitor (best-effort, after configure success)
    {
        let registrations =
            AppState::build_channel_registrations_from(&state.digitizer_configs, None);
        state.send_register_channels(registrations).await;
    }

    // Phase 2: Arm (sync point)
    let arm_result = state
        .client
        .arm_all_sync(&state.components, state.config.arm_timeout_ms)
        .await;

    match arm_result {
        Err(e) => return roll_back_to_idle(&state, phase_failed("Arm", &e)).await,
        Ok(results) if results.iter().any(|r| !r.success) => {
            return roll_back_to_idle(&state, phase_rejected("Arm", results)).await
        }
        Ok(_) => {}
    }

    // Phase 3: Start (with run_number)
    let start_result = state
        .client
        .start_all_sync(&state.components, run_number, state.config.start_timeout_ms)
        .await;

    match start_result {
        Err(e) => start_phase_failed(&state, run_number, phase_failed("Start", &e)).await,
        Ok(results) if results.iter().any(|r| !r.success) => {
            start_phase_failed(&state, run_number, phase_rejected("Start", results)).await
        }
        Ok(results) => {
            let exp_name = &state.config.experiment_name;
            // TODO 58 M16: persistence failures must reach the response.
            let mut mongo_warnings: Vec<String> = Vec::new();

            // Record run start in MongoDB and update current_run
            if let Some(ref repo) = state.run_repo {
                match repo
                    .start_run(run_number as i32, exp_name, &comment, None)
                    .await
                {
                    Ok(doc) => {
                        let info = CurrentRunInfo::from_document(&doc);
                        *state.current_run.write().await = Some(info);
                    }
                    Err(e) => {
                        tracing::error!("Failed to record run start in MongoDB: {}", e);
                        mongo_warnings.push(format!("run start not persisted: {}", e));
                        *state.current_run.write().await = Some(CurrentRunInfo {
                            run_number: run_number as i32,
                            exp_name: exp_name.clone(),
                            comment: comment.clone(),
                            start_time: chrono::Utc::now().timestamp_millis(),
                            elapsed_secs: 0,
                            status: RunStatus::Running,
                            stats: RunStats::default(),
                            notes: Vec::new(),
                        });
                    }
                }
            } else {
                *state.current_run.write().await = Some(CurrentRunInfo {
                    run_number: run_number as i32,
                    exp_name: exp_name.clone(),
                    comment,
                    start_time: chrono::Utc::now().timestamp_millis(),
                    elapsed_secs: 0,
                    status: RunStatus::Running,
                    stats: RunStats::default(),
                    notes: Vec::new(),
                });
            }

            // Create digitizer config snapshot for this run
            if let Some(ref digitizer_repo) = state.digitizer_repo {
                let configs: Vec<_> = state
                    .digitizer_configs
                    .iter()
                    .map(|r| r.value().clone())
                    .collect();
                if !configs.is_empty() {
                    if let Err(e) = digitizer_repo
                        .create_run_snapshot(run_number as i32, exp_name, configs)
                        .await
                    {
                        tracing::error!("Failed to create config snapshot: {}", e);
                        mongo_warnings.push(format!("config snapshot not persisted: {}", e));
                    }
                } else {
                    tracing::warn!(
                        run_number,
                        "digitizer_configs is empty — config snapshot skipped. \
                         Check that config_file paths in TOML are correct and \
                         operator is started from the project root."
                    );
                }
            }

            let mut message = format!(
                "Run {} started successfully (all components synchronized)",
                run_number
            );
            // TODO 58 M16: surface persistence failures in the HTTP response.
            if !mongo_warnings.is_empty() {
                message = format!(
                    "{} — WARNING, run history incomplete: {}",
                    message,
                    mongo_warnings.join("; ")
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse::success(message).with_results(results)),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    //! Run-control handlers against fake components on ZMQ ipc sockets
    //! (TODO 78: start failure → no 408, logged, rolled back to Idle).

    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::{broadcast, watch, Mutex};

    use super::*;
    use crate::common::{run_command_task, CommandResponse, ComponentState};
    use crate::operator::routes::RouterBuilder;
    use crate::operator::{ComponentConfig, OperatorConfig};

    /// Commanded state of a fake component. GetStatus answers like a Reader:
    /// a source whose hardware start never happens (a slave whose start
    /// signal is missing) keeps reporting Armed although it was told to run.
    struct Fake {
        target: ComponentState,
        starts: bool,
    }

    fn fake_handle(f: &mut Fake, cmd: Command) -> CommandResponse {
        use ComponentState::*;
        let next = match cmd {
            Command::GetStatus => {
                let reported = if f.target == Running && !f.starts {
                    Armed
                } else {
                    f.target
                };
                return CommandResponse::success(reported, "status");
            }
            Command::Reset => Idle,
            Command::Configure(_) if f.target == Idle => Configured,
            Command::Arm if f.target == Configured => Armed,
            Command::Start { .. } if f.target == Armed => Running,
            Command::Stop if f.target == Running => Configured,
            other => {
                return CommandResponse::error(
                    f.target,
                    format!("{} not valid in {}", other, f.target),
                )
            }
        };
        f.target = next;
        CommandResponse::success(next, "ok")
    }

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    /// One fake per `(name, pipeline_order, source_id, hardware start happens)`.
    fn spawn_fakes(
        specs: &[(&'static str, u32, Option<u32>, bool)],
        shutdown: &broadcast::Sender<()>,
    ) -> Vec<ComponentConfig> {
        specs
            .iter()
            .map(|&(name, pipeline_order, source_id, starts)| {
                let address = format!(
                    "ipc:///tmp/delila-op-test-{}-{}",
                    std::process::id(),
                    NEXT_ID.fetch_add(1, Ordering::Relaxed)
                );
                let fake = Arc::new(Mutex::new(Fake {
                    target: ComponentState::Idle,
                    starts,
                }));
                let (state_tx, _) = watch::channel(ComponentState::Idle);
                tokio::spawn(run_command_task(
                    address.clone(),
                    fake,
                    state_tx,
                    shutdown.subscribe(),
                    |f: &mut Fake, _: &watch::Sender<ComponentState>, cmd| fake_handle(f, cmd),
                    name,
                ));
                ComponentConfig {
                    name: name.to_string(),
                    address,
                    pipeline_order,
                    source_id,
                    is_digitizer: false,
                    config_file: None,
                    source_type: None,
                }
            })
            .collect()
    }

    /// Recorder + Merger + two sources; `slave_starts = false` is the 10/22
    /// case (S-IN slave never receives the master's Run signal).
    fn pipeline(slave_starts: bool, shutdown: &broadcast::Sender<()>) -> Vec<ComponentConfig> {
        spawn_fakes(
            &[
                ("Recorder", 3, None, true),
                ("Merger", 2, None, true),
                ("Master", 1, Some(0), true),
                ("Slave", 1, Some(1), slave_starts),
            ],
            shutdown,
        )
    }

    fn app_state(components: Vec<ComponentConfig>, dir: &std::path::Path) -> Arc<AppState> {
        let config = OperatorConfig {
            reset_timeout_ms: 1000,
            configure_timeout_ms: 1000,
            arm_timeout_ms: 1000,
            start_timeout_ms: 500,
            ..OperatorConfig::default()
        };
        RouterBuilder::new(components)
            .config(config)
            .config_dir(dir.to_path_buf())
            .build()
            .1
    }

    fn run_request(run_number: u32) -> ConfigureRequest {
        ConfigureRequest {
            run_number,
            comment: String::new(),
            exp_name: "test".to_string(),
        }
    }

    async fn system_state(state: &AppState) -> SystemState {
        SystemState::from_components(&state.client.get_all_status(&state.components).await)
    }

    #[tokio::test]
    async fn run_start_reaches_running_with_healthy_fakes() {
        let (shutdown, _) = broadcast::channel(1);
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(pipeline(true, &shutdown), dir.path());

        let (status, Json(resp)) = run_start(State(state.clone()), Json(run_request(1))).await;

        assert_eq!(status, StatusCode::OK, "{}", resp.message);
        assert_eq!(system_state(&state).await, SystemState::Running);
        let _ = shutdown.send(());
    }

    /// 408 made Chromium silently resend the POST (10/22: eight run/starts,
    /// Force Reset overwritten 5 ms later). A failed start must answer with a
    /// status the browser does not retry, name the board that did not start,
    /// and leave the system in one consistent state instead of Mixed.
    #[tokio::test]
    async fn run_start_failure_is_504_names_the_board_and_rolls_back_to_idle() {
        let (shutdown, _) = broadcast::channel(1);
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(pipeline(false, &shutdown), dir.path());

        let (status, Json(resp)) = run_start(State(state.clone()), Json(run_request(1))).await;

        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{}", resp.message);
        assert!(!resp.success);
        assert!(
            resp.message.contains("Slave stayed Armed"),
            "{}",
            resp.message
        );
        assert!(resp.message.contains("reset to Idle"), "{}", resp.message);
        assert_eq!(system_state(&state).await, SystemState::Idle);
        assert!(state.current_run.read().await.is_none());
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn start_failure_after_configure_is_504_and_rolls_back_to_idle() {
        let (shutdown, _) = broadcast::channel(1);
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(pipeline(false, &shutdown), dir.path());
        let (status, Json(resp)) = configure(State(state.clone()), Json(run_request(1))).await;
        assert_eq!(status, StatusCode::OK, "{}", resp.message);

        let (status, Json(resp)) = start(
            State(state.clone()),
            Json(StartRequest {
                run_number: 1,
                comment: String::new(),
            }),
        )
        .await;

        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{}", resp.message);
        assert!(resp.message.contains("Slave"), "{}", resp.message);
        assert_eq!(system_state(&state).await, SystemState::Idle);
        let _ = shutdown.send(());
    }
}

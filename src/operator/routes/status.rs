//! DAQ control handlers (status, configure, arm, start, stop, reset, run_start)

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};

use crate::common::{Command, RunConfig};

use super::super::{
    ApiResponse, ConfigureRequest, CurrentRunInfo, RunStats, RunStatus, StartRequest, SystemState,
    SystemStatus,
};
use super::AppState;

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

    let tuneup_mode = *state.tuneup_mode.read().await;
    let tuneup_digitizer_id = *state.tuneup_digitizer_id.read().await;

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
        (status = 400, description = "Invalid request", body = ApiResponse)
    )
)]
pub(super) async fn configure(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfigureRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    let run_config: RunConfig = request.into();
    let run_number = run_config.run_number;
    let results = state
        .client
        .configure_all(&state.components, run_config)
        .await;

    let response = ApiResponse::success(format!("Configure command sent for run {}", run_number))
        .with_results(results);

    let status = if response.success {
        // Reload configs from disk so edits since Operator start take effect
        state.reload_digitizer_configs().await;
        // Push digitizer configs to remote Readers
        let configs = state.digitizer_configs.read().await;
        for comp in &state.components {
            if comp.is_digitizer {
                if let Some(source_id) = comp.source_id {
                    if let Some(config) = configs.get(&source_id) {
                        tracing::info!(
                            source_id,
                            name = %comp.name,
                            "Pushing digitizer config to Reader"
                        );
                        if let Err(e) = state
                            .client
                            .send_command(
                                &comp.address,
                                &Command::ApplyDigitizerConfig(Box::new(config.clone())),
                            )
                            .await
                        {
                            tracing::warn!(
                                source_id,
                                error = %e,
                                "Failed to send ApplyDigitizerConfig command"
                            );
                        }
                    }
                }
            }
        }
        StatusCode::OK
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
        (status = 400, description = "Invalid state transition", body = ApiResponse)
    )
)]
pub(super) async fn start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    // Guard: reject if Tune Up mode is active
    if *state.tuneup_mode.read().await {
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

    // If Configured, arm first
    if system_state == SystemState::Configured {
        match state
            .client
            .arm_all_sync(&state.components, state.config.arm_timeout_ms)
            .await
        {
            Ok(arm_results) => {
                let arm_response =
                    ApiResponse::success("Arm command sent").with_results(arm_results);
                if !arm_response.success {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse::error("Auto-arm failed before start")),
                    );
                }
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse::error(format!("Auto-arm failed: {}", e))),
                );
            }
        }
    }

    // Start with the run number
    let start_result = state
        .client
        .start_all_sync(&state.components, run_number, state.config.start_timeout_ms)
        .await;

    let response = match start_result {
        Ok(results) => ApiResponse::success(format!("Start command sent for run {}", run_number))
            .with_results(results),
        Err(e) => {
            return (
                StatusCode::REQUEST_TIMEOUT,
                Json(ApiResponse::error(format!("Start failed: {}", e))),
            );
        }
    };

    let status = if response.success {
        let exp_name = &state.config.experiment_name;

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
                    tracing::warn!("Failed to record run start in MongoDB: {}", e);
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
        }

        // Create digitizer config snapshot for this run
        if let Some(ref digitizer_repo) = state.digitizer_repo {
            let configs: Vec<_> = state
                .digitizer_configs
                .read()
                .await
                .values()
                .cloned()
                .collect();
            if !configs.is_empty() {
                if let Err(e) = digitizer_repo
                    .create_run_snapshot(run_number as i32, exp_name, configs)
                    .await
                {
                    tracing::warn!("Failed to create config snapshot: {}", e);
                }
            } else {
                tracing::warn!(
                    run_number,
                    "digitizer_configs is empty — config snapshot skipped. \
                     Check that config_file paths in TOML are correct and \
                     operator is started from the project root."
                );
            }
        } else {
            // No MongoDB, just track in memory
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
    // Get current run info before stopping
    let current_run = state.current_run.read().await.clone();

    let results = state.client.stop_all(&state.components).await;

    let response = ApiResponse::success("Stop command sent").with_results(results);

    // Always record run end and clear current_run, even if some components
    // failed to stop. Stop is best-effort — partial failure must not leave
    // the UI thinking the run is still active.
    if let (Some(ref repo), Some(mut run_info)) = (&state.run_repo, current_run) {
        // Calculate final elapsed time
        let now_ms = chrono::Utc::now().timestamp_millis();
        run_info.elapsed_secs = (now_ms - run_info.start_time) / 1000;

        // Get final stats from components
        let components = state.client.get_all_status(&state.components).await;
        let total_events: i64 = components
            .iter()
            .filter_map(|c| c.metrics.as_ref())
            .map(|m| m.events_processed as i64)
            .sum();
        let total_bytes: i64 = components
            .iter()
            .filter_map(|c| c.metrics.as_ref())
            .map(|m| m.bytes_transferred as i64)
            .sum();
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

        if let Err(e) = repo
            .end_run(
                run_info.run_number,
                &run_info.exp_name,
                RunStatus::Completed,
                stats.clone(),
            )
            .await
        {
            tracing::warn!("Failed to record run end in MongoDB: {}", e);
        }

        // Post to ELOG (fire-and-forget, must not block stop)
        if let Some(ref elog_config) = state.config.elog {
            let elog_config = elog_config.clone();
            let run_number = run_info.run_number;
            let exp_name = run_info.exp_name.clone();
            let comment = run_info.comment.clone();
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

    let status = if response.success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    (status, Json(response))
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
        (status = 408, description = "Timeout during synchronization", body = ApiResponse),
        (status = 409, description = "System is Running or in Tune Up mode", body = ApiResponse)
    )
)]
pub(super) async fn run_start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfigureRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    // Guard: reject if Tune Up mode is active
    if *state.tuneup_mode.read().await {
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

    let run_config: RunConfig = request.clone().into();
    let run_number = run_config.run_number;
    let comment = request.comment.clone();

    // Phase 0: Reset (ensure clean state — idempotent if already Idle)
    let reset_result = state
        .client
        .reset_all_sync(&state.components, state.config.reset_timeout_ms)
        .await;

    match reset_result {
        Err(e) => {
            return (
                StatusCode::REQUEST_TIMEOUT,
                Json(ApiResponse::error(format!("Reset phase failed: {}", e))),
            );
        }
        Ok(results) if results.iter().any(|r| !r.success) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::error("Reset phase failed").with_results(results)),
            );
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

    match configure_result {
        Err(e) => {
            return (
                StatusCode::REQUEST_TIMEOUT,
                Json(ApiResponse::error(format!("Configure phase failed: {}", e))),
            );
        }
        Ok(results) if results.iter().any(|r| !r.success) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::error("Configure phase failed").with_results(results)),
            );
        }
        Ok(_) => {}
    }

    // Reload digitizer configs from disk so Phase 1.5 uses fresh values.
    // Without this, edits to JSON files (e.g. fine_ts_mode) after Operator start
    // would be overwritten by stale in-memory configs.
    state.reload_digitizer_configs().await;

    // Phase 1.5: Apply digitizer configs to remote Readers
    // This pushes configs over ZMQ so remote Readers don't need local config files
    {
        let configs = state.digitizer_configs.read().await;
        for comp in &state.components {
            if comp.is_digitizer {
                if let Some(source_id) = comp.source_id {
                    // digitizer_id == source_id by convention
                    if let Some(config) = configs.get(&source_id) {
                        tracing::info!(
                            source_id,
                            name = %comp.name,
                            "Pushing digitizer config to Reader"
                        );
                        match state
                            .client
                            .send_command(
                                &comp.address,
                                &Command::ApplyDigitizerConfig(Box::new(config.clone())),
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
                            Ok(resp) => {
                                tracing::warn!(
                                    source_id,
                                    error = %resp.message,
                                    "Failed to apply digitizer config"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    source_id,
                                    error = %e,
                                    "Failed to send ApplyDigitizerConfig command"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Register channels with Monitor (best-effort, after configure success)
    {
        let configs = state.digitizer_configs.read().await;
        let registrations = AppState::build_channel_registrations_from(&configs, None);
        state.send_register_channels(registrations).await;
    }

    // Phase 2: Arm (sync point)
    let arm_result = state
        .client
        .arm_all_sync(&state.components, state.config.arm_timeout_ms)
        .await;

    match arm_result {
        Err(e) => {
            return (
                StatusCode::REQUEST_TIMEOUT,
                Json(ApiResponse::error(format!("Arm phase failed: {}", e))),
            );
        }
        Ok(results) if results.iter().any(|r| !r.success) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::error("Arm phase failed").with_results(results)),
            );
        }
        Ok(_) => {}
    }

    // Phase 3: Start (with run_number)
    let start_result = state
        .client
        .start_all_sync(&state.components, run_number, state.config.start_timeout_ms)
        .await;

    match start_result {
        Err(e) => (
            StatusCode::REQUEST_TIMEOUT,
            Json(ApiResponse::error(format!("Start phase failed: {}", e))),
        ),
        Ok(results) if results.iter().any(|r| !r.success) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error("Start phase failed").with_results(results)),
        ),
        Ok(results) => {
            let exp_name = &state.config.experiment_name;

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
                        tracing::warn!("Failed to record run start in MongoDB: {}", e);
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
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                if !configs.is_empty() {
                    if let Err(e) = digitizer_repo
                        .create_run_snapshot(run_number as i32, exp_name, configs)
                        .await
                    {
                        tracing::warn!("Failed to create config snapshot: {}", e);
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

            (
                StatusCode::OK,
                Json(
                    ApiResponse::success(format!(
                        "Run {} started successfully (all components synchronized)",
                        run_number
                    ))
                    .with_results(results),
                ),
            )
        }
    }
}

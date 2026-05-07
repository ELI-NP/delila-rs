//! ReadLoop for the **OpenDPP endpoint** (DIG2 / AMax firmware).
//!
//! Extracted from `reader/mod.rs` (R-D2, 2026-05-06) — pure mechanical move
//! of `Reader::read_loop_opendpp`, no logic changes. Reads pre-decoded
//! events from the FELib OpenDPP endpoint (so no software decoding step is
//! needed) and forwards them as `ReadLoopOutput::Decoded`.
//!
//! Currently used by AMax / DPP_OPEN firmware. Mirrors the connection +
//! state-machine + back-pressure shape of [`super::read_loop_dig1`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

use super::state::{next_reconnect_cooldown, state_rank, RECONNECT_INITIAL};
use super::{
    caen, check_firmware_match, devtree, get_enabled_channels_from_config, opendpp_to_event_data,
    send_arm_command, send_start_command, try_connect_opendpp, FirmwareType, ReadLoopOutput,
    ReadLoopRequest, ReaderConfig, ReaderError, ReaderMetrics,
};
use crate::common::ComponentState;

/// ReadLoop task for the OpenDPP endpoint (AMax) — runs in `spawn_blocking`.
/// Each event is already decoded by the firmware, so we forward it as
/// `Decoded` directly without going through the software decoder. Uses
/// lazy connection: if the digitizer is not available at startup, the
/// loop stays alive and retries on demand.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    config: ReaderConfig,
    tx: mpsc::Sender<ReadLoopOutput>,
    state_rx: watch::Receiver<ComponentState>,
    state_tx: watch::Sender<ComponentState>,
    metrics: Arc<ReaderMetrics>,
    shutdown: Arc<AtomicBool>,
    request_rx: std::sync::mpsc::Receiver<ReadLoopRequest>,
    hw_state: Arc<Mutex<ComponentState>>,
) -> Result<(), ReaderError> {
    info!(url = %config.url, "ReadLoop (OpenDPP) starting");

    // Lazy connection: try initial connect (non-fatal). The endpoint
    // gets reconfigured with the right waveform flag once we transition
    // to Configured (see the `configure_opendpp_endpoint` call below).
    let mut connection = try_connect_opendpp(&config.url, false);
    let mut last_connect_attempt = Instant::now();
    let mut reconnect_backoff = RECONNECT_INITIAL;

    // Buffer for user info words (FW caenlist max len = 1024)
    let mut user_info_buffer = [0u64; 1024];
    // Buffer for OpenDPP waveform samples (used only when the endpoint
    // was configured with `include_waveform=true`). Sized to amax_viewer's
    // 8192-sample limit; FW writes up to its current record_length.
    let mut waveform_buffer = [0u16; 8192];

    // Track consecutive read errors for retry logic (same as RAW loop)
    let mut read_error_since: Option<Instant> = None;
    const READ_ERROR_TIMEOUT: Duration = Duration::from_secs(30);

    loop {
        // Check shutdown flag
        if shutdown.load(Ordering::Relaxed) {
            info!("ReadLoop (OpenDPP) received shutdown signal");
            break;
        }

        // --- Connection management: periodic retry with exponential backoff ---
        if connection.is_none() {
            let (cooldown, next_base) = next_reconnect_cooldown(reconnect_backoff);
            if last_connect_attempt.elapsed() > cooldown {
                last_connect_attempt = Instant::now();
                connection = try_connect_opendpp(&config.url, false);
                if connection.is_some() {
                    info!("Reconnected successfully, resetting backoff");
                    reconnect_backoff = RECONNECT_INITIAL;
                } else {
                    warn!(
                        backoff_ms = next_base.as_millis() as u64,
                        "Reconnect failed, increasing backoff"
                    );
                    reconnect_backoff = next_base;
                }
            }
        }

        // Get target state from Operator
        let target_state = *state_rx.borrow();

        // --- Target state synchronization ---
        // Ensures hardware catches up to target state after (re)connection.
        if let Some(ref mut conn) = connection {
            let target_rank = state_rank(target_state);

            // Configure needed?
            if target_rank >= state_rank(ComponentState::Configured) && !conn.hw_configured {
                // Reset digitizer to factory defaults first — ensures clean slate
                // regardless of prior state (e.g. CoMPASS register changes)
                match conn.handle.send_command(devtree::cmd::RESET) {
                    Ok(()) => info!("Digitizer reset to factory defaults"),
                    Err(e) => warn!(error = %e, "Digitizer reset failed (non-fatal)"),
                }

                // Pre-load the digitizer config so we can pick up
                // `waveforms_enabled` before configuring the endpoint —
                // the OpenDPP format JSON is locked in at endpoint setup
                // time (configure_opendpp_endpoint), not by per-channel
                // registers.
                let preload = config
                    .config_file
                    .as_deref()
                    .and_then(|p| crate::config::digitizer::DigitizerConfig::load(p).ok());
                let include_waveform = preload
                    .as_ref()
                    .and_then(|c| c.board.waveforms_enabled)
                    .unwrap_or(false);

                // Re-configure endpoint after reset (/cmd/reset invalidates
                // activeendpoint and data format — read_data returns DISABLED without this)
                match conn.handle.configure_opendpp_endpoint(include_waveform) {
                    Ok(ep) => {
                        conn.endpoint = ep;
                        conn.include_waveform = include_waveform;
                        info!(include_waveform, "Endpoint reconfigured after reset");
                    }
                    Err(e) => error!(error = %e, "Failed to reconfigure endpoint after reset"),
                }

                if let Some(ref config_path) = config.config_file {
                    info!(path = %config_path, "Loading digitizer configuration");
                    match crate::config::digitizer::DigitizerConfig::load(config_path) {
                        Ok(dig_config) => {
                            // Same firmware mismatch check as the explicit
                            // ApplyConfig path (see check_firmware_match in
                            // reader/mod.rs). The Configure auto-load path
                            // bypasses ApplyConfig and was the actual route
                            // that leaked the 2026-05-07 silent miswire.
                            match check_firmware_match(conn, &config.url, dig_config.firmware) {
                                Ok(()) => {
                                    let apply_result =
                                        conn.handle.apply_config(&dig_config).and_then(|n| {
                                            // AMax: also program per-channel user registers
                                            if dig_config.firmware == FirmwareType::AMax {
                                                conn.handle
                                                    .apply_amax_channel_config(&dig_config)
                                                    .map(|m| n + m)
                                            } else {
                                                Ok(n)
                                            }
                                        });
                                    match apply_result {
                                        Ok(count) => {
                                            info!(count, "Digitizer configuration applied");
                                        }
                                        Err(e) => {
                                            warn!(error = %e, "Auto-configure from JSON failed — \
                                                awaiting Operator ApplyDigitizerConfig");
                                            conn.auto_config_failed = true;
                                        }
                                    }
                                }
                                Err(msg) => {
                                    error!("{}", msg);
                                    conn.auto_config_failed = true;
                                }
                            }
                        }
                        Err(e) => {
                            error!(error = %e, path = %config_path, "Failed to load config file");
                            // Mark as configured anyway — digitizer keeps its current settings
                        }
                    }
                } else {
                    info!("No config_file specified, using current digitizer settings");
                }

                // ADC calibration (DIG1 only) — final Configure step, before
                // marking hw_configured. Prevents Arm delay / S_IN race.
                if config.firmware.is_dig1() {
                    match conn.handle.send_command(devtree::cmd::CALIBRATE_ADC) {
                        Ok(()) => info!("ADC calibration completed"),
                        Err(e) => warn!(error = %e, "ADC calibration failed (non-fatal)"),
                    }
                }

                conn.hw_configured = true;
                *hw_state.lock().unwrap() = ComponentState::Configured;
            }

            // Arm needed?
            if target_rank >= state_rank(ComponentState::Armed) && !conn.hw_armed {
                if conn.auto_config_failed {
                    warn!(
                        "Cannot arm: auto-configure from JSON failed and no valid \
                        config received from Operator. Run Detect or fix the JSON config."
                    );
                } else {
                    if let Err(e) = send_arm_command(&conn.handle, config.firmware) {
                        error!(error = %e, "Failed to arm digitizer");
                    }
                    conn.hw_armed = true;
                    *hw_state.lock().unwrap() = ComponentState::Armed;
                }
            }

            // Start needed?
            if target_rank >= state_rank(ComponentState::Running) && !conn.hw_running {
                if let Err(e) = send_start_command(&conn.handle, config.firmware) {
                    error!(error = %e, "Failed to start acquisition");
                }
                conn.hw_running = true;
                *hw_state.lock().unwrap() = ComponentState::Running;
            }

            // Stop needed? (target dropped below Running)
            if target_rank < state_rank(ComponentState::Running) && conn.hw_running {
                info!("Stopping digitizer acquisition");
                let _ = conn.handle.send_command(devtree::cmd::DISARM_ACQUISITION);
                // Drain remaining buffered events before clearing
                let mut drained = 0u64;
                loop {
                    let drain = if conn.include_waveform {
                        conn.endpoint.read_opendpp_event_with_waveform(
                            100,
                            &mut user_info_buffer,
                            &mut waveform_buffer,
                        )
                    } else {
                        conn.endpoint.read_opendpp_event(100, &mut user_info_buffer)
                    };
                    match drain {
                        Ok(Some(evt)) => {
                            drained += 1;
                            let event_data = opendpp_to_event_data(&evt, config.module_id);
                            let _ = tx.try_send(ReadLoopOutput::Decoded(event_data));
                        }
                        _ => break,
                    }
                }
                if drained > 0 {
                    info!(drained, "Drained remaining events after stop");
                }
                let _ = tx.try_send(ReadLoopOutput::Stop);
                let _ = conn.handle.send_command(devtree::cmd::CLEAR_DATA);
                conn.hw_armed = false;
                conn.hw_running = false;
                read_error_since = None; // Clear stale error timer across runs
                *hw_state.lock().unwrap() = ComponentState::Configured;
            }

            // Reset needed? (target is Idle, but we have armed/configured state)
            if target_state == ComponentState::Idle && (conn.hw_armed || conn.hw_configured) {
                info!("Resetting digitizer");
                let _ = conn.handle.send_command(devtree::cmd::DISARM_ACQUISITION);
                let _ = conn.handle.send_command(devtree::cmd::CLEAR_DATA);
                conn.hw_armed = false;
                conn.hw_running = false;
                conn.hw_configured = false;
                conn.auto_config_failed = false;
                read_error_since = None;
                *hw_state.lock().unwrap() = ComponentState::Idle;
            }
        }

        // --- Handle requests from command handler (Detect / ApplyConfig) ---
        if let Ok(req) = request_rx.try_recv() {
            match req {
                ReadLoopRequest::Detect { response_tx } => {
                    // Try to connect on-demand for Detect
                    if connection.is_none() {
                        connection = try_connect_opendpp(&config.url, false);
                        last_connect_attempt = Instant::now();
                    }
                    let result = match connection.as_ref() {
                        Some(conn) => conn
                            .handle
                            .get_device_info()
                            .map(|info| serde_json::to_value(&info).unwrap_or_default())
                            .map_err(|e| format!("Failed to read device info: {}", e)),
                        None => Err("Not connected to digitizer".to_string()),
                    };
                    let _ = response_tx.send(result);
                }
                ReadLoopRequest::ApplyConfig {
                    config: dig_config,
                    response_tx,
                } => {
                    if connection.is_none() {
                        connection = try_connect_opendpp(&config.url, false);
                        last_connect_attempt = Instant::now();
                    }
                    let result = match connection.as_ref() {
                        Some(conn) => {
                            // Hard-fail Apply if the digitizer's reported firmware
                            // doesn't match the config's declared firmware. See
                            // `check_firmware_match` in reader/mod.rs for the
                            // 2026-05-07 silent miswire context. AMax-specific
                            // `apply_amax_channel_config` is naturally protected
                            // because we short-circuit before reaching it.
                            match check_firmware_match(conn, &config.url, dig_config.firmware) {
                                Ok(()) => {
                                    let felib = if let Some(ref cache) = conn.param_cache {
                                        conn.handle
                                            .apply_config_validated(&dig_config, cache)
                                            .map(|r| r.ok + r.adjusted)
                                            .map_err(|e| format!("Failed to apply config: {}", e))
                                    } else {
                                        conn.handle
                                            .apply_config(&dig_config)
                                            .map_err(|e| format!("Failed to apply config: {}", e))
                                    };
                                    // AMax: also program per-channel user registers after
                                    // the FELib step. Sums into the same params_applied
                                    // count so the operator UI reports both sources.
                                    if dig_config.firmware == FirmwareType::AMax {
                                        felib.and_then(|n| {
                                            conn.handle
                                                .apply_amax_channel_config(&dig_config)
                                                .map(|m| n + m)
                                                .map_err(|e| {
                                                    format!(
                                                        "Failed to apply AMax registers: {}",
                                                        e
                                                    )
                                                })
                                        })
                                    } else {
                                        felib
                                    }
                                }
                                Err(msg) => Err(msg),
                            }
                        }
                        None => Err("Not connected to digitizer".to_string()),
                    };
                    if result.is_ok() {
                        if let Some(ref mut conn) = connection {
                            conn.auto_config_failed = false;
                            conn.enabled_channels = get_enabled_channels_from_config(&dig_config);
                        }
                    }
                    let _ = response_tx.send(result);
                }
                ReadLoopRequest::ApplyConfigRunning {
                    config: dig_config,
                    response_tx,
                } => {
                    let result = match connection.as_ref() {
                        Some(conn) => {
                            if let Some(ref cache) = conn.param_cache {
                                conn.handle
                                    .apply_config_running_validated(&dig_config, cache)
                                    .map(|r| r.ok + r.adjusted)
                                    .map_err(|e| {
                                        format!("Failed to apply SetInRun config: {}", e)
                                    })
                            } else {
                                conn.handle.apply_config_running(&dig_config).map_err(|e| {
                                    format!("Failed to apply SetInRun config: {}", e)
                                })
                            }
                        }
                        None => Err("Not connected to digitizer".to_string()),
                    };
                    let _ = response_tx.send(result);
                }
            }
        }

        // --- Data reading (Running only) ---
        if target_state != ComponentState::Running {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        if let Some(ref conn) = connection {
            if !conn.hw_running {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }

            let read_result = if conn.include_waveform {
                conn.endpoint.read_opendpp_event_with_waveform(
                    config.read_timeout_ms,
                    &mut user_info_buffer,
                    &mut waveform_buffer,
                )
            } else {
                conn.endpoint
                    .read_opendpp_event(config.read_timeout_ms, &mut user_info_buffer)
            };
            match read_result {
                Ok(Some(event)) => {
                    if let Some(since) = read_error_since.take() {
                        info!(
                            elapsed_ms = since.elapsed().as_millis() as u64,
                            "Read recovered (OpenDPP) after transient error"
                        );
                    }
                    metrics
                        .bytes_read
                        .fetch_add(event.event_size as u64, Ordering::Relaxed);

                    // Convert OpenDPP event to EventData
                    let event_data = opendpp_to_event_data(&event, config.module_id);
                    let output = ReadLoopOutput::Decoded(event_data);

                    // Update queue length metric
                    metrics.queue_length.fetch_add(1, Ordering::Relaxed);

                    // Send to decode channel with back-pressure retry
                    let mut pending = output;
                    let mut channel_closed = false;
                    loop {
                        match tx.try_send(pending) {
                            Ok(()) => break,
                            Err(mpsc::error::TrySendError::Full(returned)) => {
                                if shutdown.load(Ordering::Relaxed)
                                    || *state_rx.borrow() != ComponentState::Running
                                {
                                    warn!("Dropping pending event during shutdown/stop");
                                    break;
                                }
                                pending = returned;
                                std::thread::sleep(Duration::from_millis(1));
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                warn!("Decode channel closed, stopping read loop");
                                channel_closed = true;
                                break;
                            }
                        }
                    }
                    if channel_closed {
                        break;
                    }
                }
                Ok(None) => {
                    // Timeout - no data available, continue polling.
                    if let Some(since) = read_error_since.take() {
                        info!(
                            elapsed_ms = since.elapsed().as_millis() as u64,
                            "Read recovered (OpenDPP, timeout) after transient error"
                        );
                    }
                }
                Err(e) => {
                    if e.code == caen::error::codes::STOP {
                        if shutdown.load(Ordering::Relaxed) {
                            info!("Received STOP signal during shutdown");
                            break;
                        }
                        info!("Received STOP signal from digitizer, waiting for state change");
                        continue;
                    }
                    if target_state == ComponentState::Running {
                        // Transient error — retry (same logic as RAW loop)
                        let started = *read_error_since.get_or_insert_with(|| {
                            warn!(error = %e, code = e.code,
                                "Read error during acquisition (OpenDPP), will retry for {:?}",
                                READ_ERROR_TIMEOUT);
                            Instant::now()
                        });
                        if started.elapsed() > READ_ERROR_TIMEOUT {
                            error!(
                                timeout_secs = READ_ERROR_TIMEOUT.as_secs(),
                                error = %e, code = e.code,
                                "Read errors persisting (OpenDPP), transitioning to Error"
                            );
                            let _ = state_tx.send(ComponentState::Error);
                            connection = None;
                            read_error_since = None;
                        } else {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    } else {
                        // Not running — safe to reconnect
                        error!(error = %e, code = e.code, "Read error (OpenDPP), dropping connection");
                        connection = None;
                    }
                }
            }
        } else {
            // Running but no connection — wait for reconnect at loop top
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // Cleanup
    if let Some(conn) = connection {
        if conn.hw_armed || conn.hw_running {
            let _ = conn.handle.send_command(devtree::cmd::DISARM_ACQUISITION);
        }
    }
    info!("ReadLoop (OpenDPP) stopped");
    Ok(())
}

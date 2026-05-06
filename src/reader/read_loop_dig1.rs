//! ReadLoop for the **DIG1 RAW endpoint** (DT5730 / VX17xx / VX27xx running
//! the legacy DIG1-style RAW format used by PSD1 / PSD2 / PHA1 / PHA2).
//!
//! Extracted from `reader/mod.rs` (R-D1, 2026-05-06) — pure mechanical move
//! of `Reader::read_loop_raw`, no logic changes. The function still runs
//! inside `tokio::task::spawn_blocking` and relies on the same connection
//! state machine + back-pressure retry shape as before.
//!
//! See `docs/component_architecture.md` for the Receiver/Main/Sender pattern
//! this slots into.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

use super::state::{next_reconnect_cooldown, state_rank, RECONNECT_INITIAL};
use super::{
    caen, decoder, devtree, get_enabled_channels_from_config, poll_dig2_counters,
    send_arm_command, send_start_command, try_connect_raw, Dig2PollState, ReadLoopOutput,
    ReadLoopRequest, ReaderConfig, ReaderError, ReaderMetrics,
};
use crate::common::ComponentState;

/// ReadLoop task for the RAW endpoint (DIG1/DIG2 RAW format) — runs in
/// `spawn_blocking`. Reads raw bytes from CAEN digitizer and forwards them
/// to the decode pipeline. Uses lazy connection: if the digitizer is not
/// available at startup, the loop stays alive and retries on demand.
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
    info!(url = %config.url, "ReadLoop (RAW) starting");

    let include_n_events = config.firmware.includes_n_events();

    // Lazy connection: try initial connect (non-fatal)
    let mut connection = try_connect_raw(&config.url, include_n_events);
    let mut last_connect_attempt = Instant::now();
    let mut reconnect_backoff = RECONNECT_INITIAL;

    // Pre-allocate reusable read buffer.
    // CAEN FELib does NOT check buffer bounds — undersized buffers cause SIGBUS.
    let mut read_buffer: Vec<u8> = vec![0u8; config.buffer_size];
    info!(
        buffer_size = config.buffer_size,
        "ReadLoop buffer allocated"
    );

    // Track consecutive read errors for retry logic.
    // Optical link transients (e.g. A3818 RX timeout) are recoverable —
    // the digitizer keeps buffering data internally.
    let mut read_error_since: Option<Instant> = None;
    const READ_ERROR_TIMEOUT: Duration = Duration::from_secs(30);

    // DIG2 trigger counter polling state
    let mut dig2_poll = Dig2PollState::new();
    let mut last_dig2_poll = Instant::now();
    let mut last_dig2_warn = Instant::now();
    const DIG2_POLL_INTERVAL: Duration = Duration::from_secs(5);

    loop {
        // Check shutdown flag
        if shutdown.load(Ordering::Relaxed) {
            info!("ReadLoop received shutdown signal");
            break;
        }

        // --- Connection management: periodic retry with exponential backoff ---
        if connection.is_none() {
            let (cooldown, next_base) = next_reconnect_cooldown(reconnect_backoff);
            if last_connect_attempt.elapsed() > cooldown {
                last_connect_attempt = Instant::now();
                connection = try_connect_raw(&config.url, include_n_events);
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

                // Re-configure endpoint after reset (/cmd/reset invalidates
                // activeendpoint and data format — read_data returns DISABLED without this)
                match conn.handle.configure_endpoint(include_n_events) {
                    Ok(ep) => {
                        conn.endpoint = ep;
                        info!("Endpoint reconfigured after reset");
                    }
                    Err(e) => error!(error = %e, "Failed to reconfigure endpoint after reset"),
                }

                if let Some(ref config_path) = config.config_file {
                    info!(path = %config_path, "Loading digitizer configuration");
                    match crate::config::digitizer::DigitizerConfig::load(config_path) {
                        Ok(dig_config) => match conn.handle.apply_config(&dig_config) {
                            Ok(count) => {
                                info!(count, "Digitizer configuration applied");
                            }
                            Err(e) => {
                                warn!(error = %e, "Auto-configure from JSON failed — \
                                    awaiting Operator ApplyDigitizerConfig");
                                conn.auto_config_failed = true;
                            }
                        },
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
                // Reset DIG2 poll state for new run
                dig2_poll.reset();
                // Signal decode_loop to reset decoder state (RolloverTracker etc.)
                // DIG1 has no Start signal in the data stream, so we must send it here.
                let _ = tx.try_send(ReadLoopOutput::Start);
            }

            // Stop needed? (target dropped below Running)
            if target_rank < state_rank(ComponentState::Running) && conn.hw_running {
                info!("Stopping digitizer acquisition");
                let _ = conn.handle.send_command(devtree::cmd::DISARM_ACQUISITION);

                // Drain remaining buffered data before clearing (with limits)
                let mut drained = 0u64;
                let drain_start = Instant::now();
                const MAX_DRAIN_EVENTS: u64 = 1000;
                const MAX_DRAIN_TIME: Duration = Duration::from_secs(1);
                while let Ok(Some(raw)) = conn.endpoint.read_data(100, &mut read_buffer) {
                    drained += 1;
                    let decoder_raw = decoder::RawData::from(raw);
                    let _ = tx.try_send(ReadLoopOutput::Raw(decoder_raw));
                    if drained >= MAX_DRAIN_EVENTS || drain_start.elapsed() > MAX_DRAIN_TIME {
                        warn!(drained, "Drain limit reached, clearing remaining");
                        break;
                    }
                }
                if drained > 0 {
                    info!(drained, "Drained remaining data after stop");
                }

                // Send Stop signal with retry to guarantee EOS delivery
                let stop_deadline = Instant::now() + Duration::from_secs(3);
                let mut stop_signal = ReadLoopOutput::Stop;
                loop {
                    match tx.try_send(stop_signal) {
                        Ok(()) => {
                            info!("Stop signal sent to decode pipeline");
                            break;
                        }
                        Err(mpsc::error::TrySendError::Full(returned)) => {
                            if Instant::now() > stop_deadline {
                                error!("Failed to send Stop signal: channel full for 3s");
                                break;
                            }
                            stop_signal = returned;
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            warn!("Decode channel closed, Stop signal not needed");
                            break;
                        }
                    }
                }

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
                        connection = try_connect_raw(&config.url, include_n_events);
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
                        connection = try_connect_raw(&config.url, include_n_events);
                        last_connect_attempt = Instant::now();
                    }
                    let result = match connection.as_ref() {
                        Some(conn) => {
                            if let Some(ref cache) = conn.param_cache {
                                conn.handle
                                    .apply_config_validated(&dig_config, cache)
                                    .map(|r| r.ok + r.adjusted)
                                    .map_err(|e| format!("Failed to apply config: {}", e))
                            } else {
                                conn.handle
                                    .apply_config(&dig_config)
                                    .map_err(|e| format!("Failed to apply config: {}", e))
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

            match conn
                .endpoint
                .read_data(config.read_timeout_ms, &mut read_buffer)
            {
                Ok(Some(raw)) => {
                    if let Some(since) = read_error_since.take() {
                        info!(
                            elapsed_ms = since.elapsed().as_millis() as u64,
                            "Read recovered after transient error"
                        );
                    }
                    metrics
                        .bytes_read
                        .fetch_add(raw.size as u64, Ordering::Relaxed);

                    let decoder_raw = decoder::RawData::from(raw);
                    let output = ReadLoopOutput::Raw(decoder_raw);
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
                                    warn!("Dropping pending data during shutdown/stop");
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
                    // Also clears error state: read_data call succeeded.
                    if let Some(since) = read_error_since.take() {
                        info!(
                            elapsed_ms = since.elapsed().as_millis() as u64,
                            "Read recovered (timeout) after transient error"
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
                        // Transient error during acquisition — retry instead of
                        // dropping connection. The digitizer continues buffering
                        // data internally; we just need to wait for the optical
                        // link / driver to recover.
                        let started = *read_error_since.get_or_insert_with(|| {
                            warn!(error = %e, code = e.code,
                                "Read error during acquisition, will retry for {:?}",
                                READ_ERROR_TIMEOUT);
                            Instant::now()
                        });
                        if started.elapsed() > READ_ERROR_TIMEOUT {
                            error!(
                                timeout_secs = READ_ERROR_TIMEOUT.as_secs(),
                                error = %e, code = e.code,
                                "Read errors persisting, transitioning to Error"
                            );
                            let _ = state_tx.send(ComponentState::Error);
                            connection = None;
                            read_error_since = None;
                        } else {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    } else {
                        // Not running — safe to reconnect
                        error!(error = %e, code = e.code, "Read error, dropping connection");
                        connection = None;
                    }
                }
            }
        } else {
            // Running but no connection — wait for reconnect at loop top
            std::thread::sleep(Duration::from_millis(100));
        }

        // DIG2: Periodic trigger counter polling (separate borrow scope)
        if !config.firmware.is_dig1() && last_dig2_poll.elapsed() >= DIG2_POLL_INTERVAL {
            if let Some(ref conn) = connection {
                if conn.hw_running {
                    poll_dig2_counters(conn, &mut dig2_poll, &metrics, &mut last_dig2_warn);
                }
            }
            last_dig2_poll = Instant::now();
        }
    }

    // Cleanup
    if let Some(conn) = connection {
        if conn.hw_armed || conn.hw_running {
            let _ = conn.handle.send_command(devtree::cmd::DISARM_ACQUISITION);
        }
    }
    info!("ReadLoop (RAW) stopped");
    Ok(())
}

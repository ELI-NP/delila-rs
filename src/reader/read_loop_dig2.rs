//! ReadLoop for the **OpenDPP endpoint** (DIG2 / AMax firmware).
//!
//! Extracted from `reader/mod.rs` (R-D2, 2026-05-06). Reads events from the
//! FELib OpenDPP endpoint and forwards them **untranslated** as
//! `ReadLoopOutput::OpenDpp` — the CPU-heavy 4-lane debug unpack + EventData
//! conversion runs on the decode workers, keeping this thread read-only
//! (2026-06-11; it was the 103 kHz ceiling when done inline here).
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
    caen, check_firmware_match, decoder, devtree, get_enabled_channels_from_config,
    send_arm_command, send_start_command, try_connect_opendpp, try_connect_rawudp, FirmwareType,
    ReadLoopOutput, ReadLoopRequest, ReaderConfig, ReaderError, ReaderMetrics,
};
use crate::common::ComponentState;

/// ReadLoop task for the OpenDPP endpoint (AMax) — runs in `spawn_blocking`.
/// Events are framed by the firmware; we forward them untranslated as
/// `OpenDpp` and the decode workers do the 4-lane unpack + conversion. Uses
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
    // EXPERIMENTAL (2026-06-11): route AMax through the bulk RAW endpoint
    // instead of the per-event OpenDPP decoded endpoint. The OpenDPP path
    // does one FELib `ReadData` call + one waveform heap alloc per event
    // (the ~177 kHz decode ceiling). RAW reads whole event-aggregate
    // buffers in one call and the parallel decode workers parse them with
    // `AMaxDecoder` (big-endian word0/word1 + user words + waveform — the
    // decode side already supports `DecoderKind::AMax`). Toggle with
    // `DELILA_AMAX_RAW_READOUT=1` so we can A/B against OpenDPP live.
    let raw_readout = std::env::var("DELILA_AMAX_RAW_READOUT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    info!(url = %config.url, raw_readout, "ReadLoop (OpenDPP/RAW) starting");

    // Connect helper: RAW endpoint (bulk, DIG2 N_EVENTS format) when the
    // experimental flag is set, else the OpenDPP decoded endpoint. AMax
    // register programming is endpoint-independent, so the rest of the
    // Configure path is unchanged.
    let connect_dev = |url: &str| {
        if raw_readout {
            try_connect_rawudp(url)
        } else {
            try_connect_opendpp(url, false)
        }
    };

    // Bulk read buffer for the RAW path (unused in OpenDPP mode). Sizing it
    // controls decode parallelism: each `read_data` buffer becomes ONE work
    // unit handled by ONE decode worker, so an oversized buffer (e.g. 4 MB ≈
    // 600+ AMax events) serializes the whole chunk on a single worker while
    // the others idle and the queue/memory balloon. A smaller buffer yields
    // many small units that round-robin across all workers (mirrors the
    // OpenDPP COALESCE_MAX=256 batch granularity). Tunable live via
    // `DELILA_AMAX_RAW_BUF_KB` (default 512 KB ≈ ~120 events).
    let raw_buf_kb: usize = std::env::var("DELILA_AMAX_RAW_BUF_KB")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&kb| kb > 0)
        .unwrap_or(512);
    if raw_readout {
        info!(raw_buf_kb, "RAW readout bulk buffer sized");
    }
    let mut read_buffer: Vec<u8> = Vec::with_capacity(raw_buf_kb * 1024);

    // Lazy connection: try initial connect (non-fatal). The endpoint
    // gets reconfigured with the right format once we transition to
    // Configured (see the endpoint reconfigure in the Configure block).
    let mut connection = connect_dev(&config.url);
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
                connection = connect_dev(&config.url);
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
                // activeendpoint and data format — read_data returns DISABLED without this).
                // RAW path: bulk endpoint (DIG2 N_EVENTS format); the FW always
                // emits waveforms on the wire, so `include_waveform` is moot here
                // (the AMax decoder reads the per-event WAVEFORM flag).
                let ep_result = if raw_readout {
                    conn.handle.configure_rawudp_endpoint()
                } else {
                    conn.handle.configure_opendpp_endpoint(include_waveform)
                };
                match ep_result {
                    Ok(ep) => {
                        conn.endpoint = ep;
                        conn.include_waveform = include_waveform;
                        info!(include_waveform, raw_readout, "Endpoint reconfigured after reset");
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
                                            // Track ENABLE_ACQ so the OpenDPP read loop
                                            // can dispatch the 4-lane debug-waveform
                                            // unpack on ch0 events.
                                            conn.amax_enable_acq =
                                                crate::reader::amax_enable_acq_from_config(
                                                    &dig_config,
                                                );
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
                // AMax 11june2026+ FW: close the acquisition gate first by
                // dropping the START_DAQ register (CAEN-list Run port) — the
                // mirror of the START_DAQ=1 write in `send_start_command`.
                if config.firmware == FirmwareType::AMax {
                    match conn
                        .handle
                        .set_user_register(super::AMAX_START_DAQ_BYTE_ADDR, 0)
                    {
                        Ok(()) => info!("AMax START_DAQ=0 written (acquisition gate closed)"),
                        Err(e) => warn!(error = %e, "Failed to clear AMax START_DAQ register"),
                    }
                }
                let _ = conn.handle.send_command(devtree::cmd::DISARM_ACQUISITION);
                // Drain remaining buffered events before clearing
                let mut drained = 0u64;
                if raw_readout {
                    while let Ok(Some(raw)) = conn.endpoint.read_data(100, &mut read_buffer) {
                        drained += 1;
                        // Pair with the dispatcher's `fetch_sub` on Raw so the
                        // queue-length counter stays balanced (no underflow).
                        metrics.queue_length.fetch_add(1, Ordering::Relaxed);
                        let _ = tx.try_send(ReadLoopOutput::Raw(decoder::RawData::from(raw)));
                    }
                } else {
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
                                let _ = tx.try_send(ReadLoopOutput::OpenDpp {
                                    event: Box::new(evt),
                                    enable_acq: conn.amax_enable_acq,
                                });
                            }
                            _ => break,
                        }
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
                        connection = connect_dev(&config.url);
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
                        connection = connect_dev(&config.url);
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
                                                    format!("Failed to apply AMax registers: {}", e)
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
                            // Refresh ENABLE_ACQ tracking so the OpenDPP read
                            // hot path picks up the new value on the next event.
                            conn.amax_enable_acq =
                                crate::reader::amax_enable_acq_from_config(&dig_config);
                            // Hot-swap OpenDPP waveform delivery when the user
                            // toggles `waveforms_enabled` during Tune Up. The
                            // FELib data format JSON is locked at endpoint
                            // setup, so per-channel ApplyConfig alone cannot
                            // change waveform inclusion — we must rebind the
                            // active endpoint here while acquisition is stopped.
                            // RAW path carries waveforms in the raw bytes
                            // regardless of `waveforms_enabled`, so there is no
                            // endpoint to rebind on toggle — skip the hot-swap.
                            let target_iw = dig_config.board.waveforms_enabled.unwrap_or(false);
                            if !raw_readout && target_iw != conn.include_waveform {
                                match conn.handle.configure_opendpp_endpoint(target_iw) {
                                    Ok(ep) => {
                                        conn.endpoint = ep;
                                        conn.include_waveform = target_iw;
                                        info!(
                                            include_waveform = target_iw,
                                            "OpenDPP endpoint reconfigured during ApplyConfig"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            error = %e,
                                            "Failed to reconfigure OpenDPP endpoint during Apply"
                                        );
                                    }
                                }
                            }
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
                                    .map_err(|e| format!("Failed to apply SetInRun config: {}", e))
                            } else {
                                conn.handle
                                    .apply_config_running(&dig_config)
                                    .map_err(|e| format!("Failed to apply SetInRun config: {}", e))
                            }
                        }
                        None => Err("Not connected to digitizer".to_string()),
                    };
                    let _ = response_tx.send(result);
                }
                ReadLoopRequest::ReadAmaxBoardRegisters { response_tx } => {
                    use crate::reader::caen::amax_registers as r;
                    let result = match connection.as_ref() {
                        Some(conn) => {
                            // Codegen-emitted (addr, name) list of every
                            // AMax board-level register — auto-extends
                            // when fw_params.json grows new `board_params`.
                            let entries = r::all_board_registers();
                            let mut out = Vec::with_capacity(entries.len());
                            let mut err: Option<String> = None;
                            for (addr, name) in entries {
                                match conn.handle.get_user_register(addr) {
                                    Ok(v) => out.push((name.to_string(), v)),
                                    Err(e) => {
                                        err = Some(format!(
                                            "Failed to read {} (addr 0x{:08X}): {}",
                                            name, addr, e
                                        ));
                                        break;
                                    }
                                }
                            }
                            err.map_or(Ok(out), Err)
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

            // Read one unit of work and fold it into a `ReadLoopOutput`:
            //  - RAW path: a bulk event-aggregate buffer, decoded later by
            //    `AMaxDecoder` on the workers (`ReadLoopOutput::Raw`).
            //  - OpenDPP path: a single FELib-decoded event
            //    (`ReadLoopOutput::OpenDpp`), unchanged behaviour.
            // `bytes_read` is accumulated here in both cases; the send /
            // error / retry handling below is shared.
            let read_result = if raw_readout {
                match conn.endpoint.read_data(config.read_timeout_ms, &mut read_buffer) {
                    Ok(Some(raw)) => {
                        metrics
                            .bytes_read
                            .fetch_add(raw.size as u64, Ordering::Relaxed);
                        Ok(Some(ReadLoopOutput::Raw(decoder::RawData::from(raw))))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                }
            } else {
                let opendpp = if conn.include_waveform {
                    conn.endpoint.read_opendpp_event_with_waveform(
                        config.read_timeout_ms,
                        &mut user_info_buffer,
                        &mut waveform_buffer,
                    )
                } else {
                    conn.endpoint
                        .read_opendpp_event(config.read_timeout_ms, &mut user_info_buffer)
                };
                match opendpp {
                    Ok(Some(event)) => {
                        metrics
                            .bytes_read
                            .fetch_add(event.event_size as u64, Ordering::Relaxed);
                        // Forward untranslated — the 4-lane unpack + EventData
                        // conversion runs on the decode workers (keeps this
                        // thread read-only; see ReadLoopOutput::OpenDpp).
                        Ok(Some(ReadLoopOutput::OpenDpp {
                            event: Box::new(event),
                            enable_acq: conn.amax_enable_acq,
                        }))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                }
            };
            match read_result {
                Ok(Some(output)) => {
                    if let Some(since) = read_error_since.take() {
                        info!(
                            elapsed_ms = since.elapsed().as_millis() as u64,
                            "Read recovered after transient error"
                        );
                    }

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

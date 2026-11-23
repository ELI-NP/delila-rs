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

use super::eos::{self, OwedEos};
use super::state::{next_reconnect_cooldown, state_rank, RECONNECT_INITIAL};
use super::{
    caen, check_firmware_match, decoder, devtree, get_enabled_channels_from_config, pll,
    poll_dig2_counters, send_arm_command, send_start_command, try_connect_raw, Dig2PollState,
    ReadLoopOutput, ReadLoopRequest, ReaderConfig, ReaderError, ReaderMetrics,
};
use crate::common::{ComponentState, PllStatus};
use crate::config::digitizer::{DigitizerConfig, DIG1_COUPLE_LOCAL_BUFFER_SAMPLES};

/// Warn (once per successful apply) when the DIG1 couple memory arbiter is
/// active — the record length forces each channel couple (2m, 2m+1) onto the
/// shared SRAM, so simultaneous triggers inside a couple lose one event.
/// See [`DigitizerConfig::dig1_couple_arbiter_samples`].
fn warn_if_couple_arbiter_active(dig_config: &DigitizerConfig, time_step_ns: f64) {
    let Some(samples) = dig_config.dig1_couple_arbiter_samples(time_step_ns) else {
        return;
    };
    let record_length_ns = dig_config.board.record_length.unwrap_or_default();
    let limit_ns = f64::from(DIG1_COUPLE_LOCAL_BUFFER_SAMPLES) * time_step_ns;
    warn!(
        record_length_ns,
        samples,
        time_step_ns,
        limit_ns,
        "DIG1 couple memory arbiter active: when both channels of a couple (2m, 2m+1) \
         trigger together the firmware keeps only ONE event (UM5678 §1.2). Use a record \
         length below {limit_ns} ns or disable waveforms for coincidence data."
    );
}

/// Periodic forced flush of the DIG1 aggregates while running.
///
/// Why: a DIG1 board holds each couple's events until its aggregate is full
/// (`eventaggr`, 1023 here) or flushed (UM4380 rev6 p.11 / p.14). The
/// library's autoflush only fires when the board has nothing else to send, so
/// on a busy board a slow couple waits for 1023 events: pf-qaq DAQ_Test run 16
/// (2026-11-21), V1730 mod3 with one channel at 20 kHz — ch4 at 780 Hz came up
/// to 1.22 s behind the newest hit, past thgem_sink's 1 s horizon (180 hits
/// missing from the .root; the .delila had them). A 10 Hz trigger channel on
/// such a board would wait ~100 s. Flushing every [`Self::INTERVAL`] bounds
/// the wait without shrinking the aggregates (memory depth and readout
/// efficiency stay as they are).
struct Dig1Flusher {
    last: Instant,
    failures: u64,
    last_warn: Option<Instant>,
}

impl Dig1Flusher {
    const INTERVAL: Duration = Duration::from_millis(100);
    const WARN_EVERY: Duration = Duration::from_secs(10);

    fn new(now: Instant) -> Self {
        Self {
            last: now,
            failures: 0,
            last_warn: None,
        }
    }

    /// Whether a flush is due at `now`; if so, the next one is counted from `now`.
    fn due(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.last) < Self::INTERVAL {
            return false;
        }
        self.last = now;
        true
    }

    /// Count a failed flush; true when it should be logged (first failure,
    /// then at most once per [`Self::WARN_EVERY`]).
    fn failed(&mut self, now: Instant) -> bool {
        self.failures += 1;
        let log = self
            .last_warn
            .is_none_or(|t| now.saturating_duration_since(t) >= Self::WARN_EVERY);
        if log {
            self.last_warn = Some(now);
        }
        log
    }
}

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

    // DIG1 aggregate flush while running (see Dig1Flusher).
    let mut dig1_flush = Dig1Flusher::new(Instant::now());

    // Clock (PLL) health (TODO 85), polled in every state below.
    let mut pll_poller = pll::PllPoller::default();

    // The EOS of a run this board could not finish (connection lost mid-run).
    let mut owed_eos = OwedEos::default();

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
                        Ok(dig_config) => {
                            // Same firmware mismatch check as the explicit
                            // ApplyConfig path (see check_firmware_match in
                            // reader/mod.rs). The Configure auto-load path
                            // calls apply_config directly, so without this
                            // gate the silent-miswire bug from 2026-05-07
                            // would still leak through here even if the
                            // operator-driven ApplyConfig path is protected.
                            match check_firmware_match(conn, &config.url, dig_config.firmware) {
                                Ok(()) => match conn.handle.apply_config(&dig_config) {
                                    Ok(count) => {
                                        info!(count, "Digitizer configuration applied");
                                        warn_if_couple_arbiter_active(
                                            &dig_config,
                                            config.time_step_ns,
                                        );
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Auto-configure from JSON failed — \
                                            awaiting Operator ApplyDigitizerConfig");
                                        conn.auto_config_failed = true;
                                    }
                                },
                                Err(msg) => {
                                    error!("{}", msg);
                                    conn.auto_config_failed = true;
                                }
                            }
                        }
                        Err(e) => {
                            // TODO 58 M1: the /cmd/reset above already wiped the
                            // digitizer to factory defaults, so "keeps its current
                            // settings" would be a lie — block Arm until the
                            // Operator supplies a valid config.
                            error!(error = %e, path = %config_path, "Failed to load config file — blocking Arm (digitizer is at factory defaults after reset)");
                            conn.auto_config_failed = true;
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

            // Backoff gate shared by the Arm/Start attempts below (TODO 58 M2):
            // the loop spins every ~10 ms, so failed attempts retry (and log)
            // at most once per backoff period instead of flooding.
            let hw_cmd_ready = conn.hw_cmd_retry_after.is_none_or(|t| Instant::now() >= t);

            // Arm needed?
            if target_rank >= state_rank(ComponentState::Armed) && !conn.hw_armed && hw_cmd_ready {
                if conn.auto_config_failed {
                    warn!(
                        "Cannot arm: auto-configure from JSON failed and no valid \
                        config received from Operator. Run Detect or fix the JSON config."
                    );
                    conn.hw_cmd_retry_after = Some(Instant::now() + Duration::from_secs(5));
                } else {
                    // TODO 58 M2: only claim Armed when the hardware command
                    // succeeded — a failed arm previously still reported Armed,
                    // showing the operator a data-less "Running" run later.
                    match send_arm_command(&conn.handle, config.firmware) {
                        Ok(()) => {
                            conn.hw_armed = true;
                            conn.hw_cmd_retry_after = None;
                            *hw_state.lock().unwrap() = ComponentState::Armed;
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to arm digitizer — staying un-armed, retrying in 5 s");
                            conn.hw_cmd_retry_after = Some(Instant::now() + Duration::from_secs(5));
                        }
                    }
                }
            }

            // Start needed?
            if target_rank >= state_rank(ComponentState::Running)
                && !conn.hw_running
                && hw_cmd_ready
            {
                // TODO 58 M2: same honesty rule as Arm — a failed start must not
                // report Running.
                if let Err(e) = send_start_command(&conn.handle, config.firmware) {
                    error!(error = %e, "Failed to start acquisition — staying stopped, retrying in 5 s");
                    conn.hw_cmd_retry_after = Some(Instant::now() + Duration::from_secs(5));
                    continue;
                }
                conn.hw_running = true;
                conn.hw_cmd_retry_after = None;
                *hw_state.lock().unwrap() = ComponentState::Running;
                owed_eos.run_started(metrics.current_run.load(Ordering::Relaxed));
                // Reset DIG2 poll state for new run
                dig2_poll.reset();
                // Signal decode_loop to reset decoder state (RolloverTracker etc.)
                // DIG1 has no Start signal in the data stream, so we must send it here.
                // Retry on Full: a lost Start means the RolloverTracker carries the
                // previous run's state → whole-run timestamp corruption (TODO 58 H2).
                let start_deadline = Instant::now() + Duration::from_secs(3);
                let mut start_signal = ReadLoopOutput::Start;
                loop {
                    match tx.try_send(start_signal) {
                        Ok(()) => break,
                        Err(mpsc::error::TrySendError::Full(returned)) => {
                            if Instant::now() > start_deadline {
                                error!(
                                    "Failed to send Start signal: channel full for 3s — \
                                     decoder state NOT reset, timestamps of this run may be corrupt"
                                );
                                break;
                            }
                            start_signal = returned;
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            warn!("Decode channel closed, Start signal not sent");
                            break;
                        }
                    }
                }
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
                eos::send_stop_signal(&tx);
                owed_eos.stop_sent();

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

        // A run whose connection was lost mid-run still owes its EOS: sent
        // once the Operator has stopped the run (eos.rs).
        eos::act_on(
            owed_eos.poll(
                target_state,
                metrics.current_run.load(Ordering::Relaxed),
                Instant::now(),
            ),
            &tx,
        );

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
                            // Hard-fail Apply if the digitizer's reported firmware
                            // doesn't match the config's declared firmware. Without
                            // this, mismatches (e.g. PHA2 config sent to AMax HW)
                            // surface only as ~70% per-param FELib rejects in WARN
                            // logs while the operator UI reports "Configured"
                            // success — exactly the silent-failure mode CLAUDE.md
                            // forbids. Discovered live 2026-05-07.
                            match check_firmware_match(conn, &config.url, dig_config.firmware) {
                                Ok(()) => {
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
                                Err(msg) => Err(msg),
                            }
                        }
                        None => Err("Not connected to digitizer".to_string()),
                    };
                    if result.is_ok() {
                        warn_if_couple_arbiter_active(&dig_config, config.time_step_ns);
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
                    // DIG1 firmwares (PSD1/PHA1) don't expose AMax board
                    // registers — return an empty list instead of erroring
                    // so the operator UI can render "no AMax registers" for
                    // mixed-FW setups without surfacing a fake error.
                    let _ = response_tx.send(Ok(Vec::new()));
                }
            }
        }

        // Clock (PLL) health in every state, connected or not (TODO 85): the
        // UI shows it before a run, not only once Start found a dead board.
        if let Some(reading) = pll_poller.poll(Instant::now(), connection.is_some(), || {
            connection
                .as_ref()
                .and_then(|conn| pll::read(conn, config.firmware))
        }) {
            metrics.pll_status.store(
                PllStatus::to_u8(reading.map(|r| r.status)),
                Ordering::Relaxed,
            );
            metrics
                .pll_fault_bits
                .store(reading.map_or(0, |r| r.fault_bits), Ordering::Relaxed);
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
                                "Read errors persisting, transitioning to Error \
                                 (this board's end-of-run marker is sent when the run is stopped)"
                            );
                            let _ = state_tx.send(ComponentState::Error);
                            // GetStatus reports hw_state: without this the
                            // Operator kept showing the dead board as Running
                            // (pf-qaq run 70).
                            *hw_state.lock().unwrap() = ComponentState::Error;
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

        // DIG1: close the partial aggregates so a slow couple on a busy board
        // is not held back until it has `eventaggr` events (see Dig1Flusher).
        if config.firmware.is_dig1() && dig1_flush.due(Instant::now()) {
            if let Some(ref conn) = connection {
                if conn.hw_running {
                    if let Err(e) = conn.handle.set_value(devtree::par::MANUAL_FLUSH, "1") {
                        if dig1_flush.failed(Instant::now()) {
                            warn!(
                                error = %e,
                                code = e.code,
                                failures = dig1_flush.failures,
                                "DIG1 aggregate flush failed: events of a slow channel couple \
                                 may arrive late (up to eventaggr events behind)"
                            );
                        }
                    }
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dig1_flush_is_due_every_interval() {
        let t0 = Instant::now();
        let mut f = Dig1Flusher::new(t0);
        assert!(!f.due(t0));
        assert!(!f.due(t0 + Duration::from_millis(99)));
        assert!(f.due(t0 + Dig1Flusher::INTERVAL));
        // counted from the flush just done
        assert!(!f.due(t0 + Dig1Flusher::INTERVAL + Duration::from_millis(50)));
        assert!(f.due(t0 + Dig1Flusher::INTERVAL * 2));
    }

    #[test]
    fn dig1_flush_failures_are_counted_and_logged_at_most_every_10_s() {
        let t0 = Instant::now();
        let mut f = Dig1Flusher::new(t0);
        assert!(f.failed(t0), "the first failure is logged");
        assert!(!f.failed(t0 + Duration::from_secs(1)));
        assert!(!f.failed(t0 + Duration::from_millis(9_900)));
        assert!(f.failed(t0 + Dig1Flusher::WARN_EVERY));
        assert_eq!(f.failures, 4);
    }

    /// The flush path must be a real, write-only, settable-in-run DIG1 parameter
    /// of both firmwares this loop drives (DevTree dumps from the DT5730B).
    #[test]
    fn manual_flush_is_a_run_time_parameter_of_psd1_and_pha1() {
        let name = devtree::par::MANUAL_FLUSH
            .strip_prefix("/par/")
            .expect("a board parameter");
        for dump in [
            "docs/devtree_examples/dt5730b_psd1_sn990.json",
            "docs/devtree_examples/dt5730b_pha1_sn990.json",
        ] {
            let text = std::fs::read_to_string(dump).expect("DevTree dump");
            let tree: serde_json::Value = serde_json::from_str(&text).expect("JSON");
            let par = &tree["par"][name];
            assert_eq!(par["accessmode"]["value"], "WRITE_ONLY", "{dump}");
            assert_eq!(par["setinrun"]["value"], "true", "{dump}");
            assert_eq!(par["maxvalue"]["value"], "1", "{dump}");
        }
    }
}

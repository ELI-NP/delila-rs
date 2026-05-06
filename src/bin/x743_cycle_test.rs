//! V1743 Start/Stop cycle reproducer.
//!
//! Isolated test to pinpoint which Stop/Start sequence survives repeated
//! cycles on V1743. Opens the digitizer once, applies a minimal hard-coded
//! config, then loops Start→(run)→Stop→Start... in one of four modes and
//! logs register 0x8178 (Board Fail Status) at key points.
//!
//! Config is hard-coded for the lab pulser on ch0 (negative, threshold DAC
//! 45874 ≈ -0.5 V, group 0 enabled). Trigger is self-trigger so the external
//! pulser drives it.
//!
//! Modes (Stop procedure → Start procedure):
//!   A  WaveDemo-style        : Stop+ClearData     → StartOnly
//!   B  Current DELILA Reader : Stop+ReadDrain     → ClearData+Start
//!   C  Everything            : Stop+Drain+Clear   → ClearData+Start
//!   D  Minimal               : Stop only          → Start only
//!
//! Usage:
//!   x743_cycle_test --mode A --cycles 20 --run-ms 500
//!
//! Example CSV log line:
//!   cycle,phase,mode,status_0x8178,elapsed_ms
//!   3,post_stop,A,0x00000000,12

use clap::Parser;
use delila_rs::config::DigitizerConfig;
use delila_rs::reader::caen_legacy::*;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "x743_cycle_test",
    about = "V1743 Start/Stop cycle reproducer — isolates CAEN crash pattern"
)]
struct Args {
    /// Stop/Start sequence to test
    #[arg(long, default_value = "A")]
    mode: String,

    /// Number of Start/Stop cycles to run
    #[arg(long, default_value_t = 20)]
    cycles: u32,

    /// Time between Start and Stop (ms)
    #[arg(long, default_value_t = 500)]
    run_ms: u64,

    /// Optional CSV output file (appended)
    #[arg(long)]
    csv: Option<String>,

    /// Re-apply full config (Reset + all registers) at the start of every cycle.
    /// Mirrors what DELILA Reader does when Tune Up Apply or run_start triggers
    /// apply_config_standard each time.
    #[arg(long)]
    reapply: bool,

    /// Load this JSON DigitizerConfig (x743 section) and use the production
    /// apply_config_standard() instead of the hardcoded minimal setup. Lets us
    /// reproduce the exact register sequence the production Reader uses.
    #[arg(long)]
    prod_config: Option<String>,

    /// Close and re-Open the CAEN handle at the start of every cycle.
    /// Mirrors stop_daq + start_daq (which kills Reader, next start_daq
    /// spawns a new Reader that does a fresh CAEN_DGTZ_OpenDigitizer).
    #[arg(long)]
    reopen: bool,

    /// Skip read_data during the run phase (let FIFO fill and overflow).
    /// Reproduces what the user observed: front-panel "board full" LED lit
    /// when the downstream drain is too slow / back-pressured.
    #[arg(long)]
    fill_fifo: bool,

    /// Disable the end-of-session AcqGuard (which calls SWStop + ClearData).
    /// Production Reader has no such guard — it exits with Stop only, no
    /// explicit ClearData. Use to match prod behaviour exactly.
    #[arg(long)]
    no_guard: bool,

    /// Mirror production: after every read_data that returns >0 bytes, call
    /// get_num_events + (get_event_info + decode_event) per event into a
    /// pre-allocated EventBuffer. Standalone normally only calls read_data;
    /// production always decodes. Tests Hyp H-Decode (P1).
    /// See plan: ~/.claude/plans/gemini-cli-peppy-turtle.md (T1).
    #[arg(long)]
    decode_events: bool,

    /// After every successful re-apply, free + re-malloc the readout buffer
    /// (and the event buffer if --decode-events). Production allocates these
    /// once at Open and never re-mallocs across Reset → may use stale internal
    /// lib pointers. Tests Hyp H-Buf (P6). See plan T2.
    #[arg(long)]
    realloc_buf: bool,

    /// Mirror production's double `CAEN_DGTZ_Reset` per cycle: in addition
    /// to the Reset inside apply_config_standard, call h.reset() *before*
    /// reapply (matches reader/mod.rs:2287 "Reset to Idle" block followed
    /// by Configure → apply_config_standard's own Reset). Tests Hyp P2.
    /// See plan T3.
    #[arg(long)]
    double_reset: bool,

    /// Spawn a tokio runtime with multiple workers + a ZMQ PUB socket
    /// publishing 1 MB random buffers at 50 Hz, plus a ZMQ REP socket
    /// echoing on a side port. None of these touch the CAEN handle, but
    /// they create scheduling pressure and ZMQ I/O activity that mirrors
    /// production. Tests Hyp P3 (tokio + ZMQ noise destabilizes lib's
    /// background thread). See plan T6.
    #[arg(long)]
    zmq_noise: bool,

    /// Allocate readout buffer + event buffer BEFORE the first apply_config
    /// (matches production reader/mod.rs:2148, which allocs immediately after
    /// Open with default board state, then later applies config which changes
    /// record_length and other DMA-relevant params). Standalone normally allocs
    /// AFTER apply_config, sized for the actual record_length. Tests Hyp H-Buf
    /// (P6) directly. See plan T7.
    #[arg(long)]
    alloc_before_apply: bool,

    /// Link type (optical/usb)
    #[arg(long, default_value = "optical")]
    link_type: String,

    /// Link/port number
    #[arg(long, default_value_t = 0)]
    link_num: u32,

    /// CONET node
    #[arg(long, default_value_t = 0)]
    conet_node: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// WaveDemo: Stop+ClearData → StartOnly
    A,
    /// Current DELILA: Stop+ReadDrain → ClearData+Start
    B,
    /// Both: Stop+Drain+Clear → ClearData+Start
    C,
    /// Minimal: Stop → Start
    D,
}

impl Mode {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_uppercase().as_str() {
            "A" => Ok(Self::A),
            "B" => Ok(Self::B),
            "C" => Ok(Self::C),
            "D" => Ok(Self::D),
            other => Err(format!("unknown mode '{}'", other)),
        }
    }
}

/// Log both stdout (via tracing) and optional CSV file.
struct Log {
    csv: Option<std::fs::File>,
    mode: Mode,
    start: Instant,
}

impl Log {
    fn new(path: Option<&str>, mode: Mode) -> Self {
        let csv = path.map(|p| {
            use std::io::Write;
            let existed = std::path::Path::new(p).exists();
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .expect("open csv");
            if !existed {
                writeln!(f, "timestamp_s,mode,cycle,phase,status_0x8178,note").unwrap();
            }
            f
        });
        Self {
            csv,
            mode,
            start: Instant::now(),
        }
    }

    fn record(&mut self, cycle: u32, phase: &str, status: Option<u32>, note: &str) {
        let t = self.start.elapsed().as_secs_f64();
        let status_s = status
            .map(|s| format!("0x{:08X}", s))
            .unwrap_or_else(|| "-".to_string());
        info!(cycle, phase, status = %status_s, note = %note, "mark");
        if let Some(ref mut f) = self.csv {
            use std::io::Write;
            let _ = writeln!(
                f,
                "{:.3},{:?},{},{},{},{}",
                t, self.mode, cycle, phase, status_s, note
            );
        }
    }
}

fn read_status(h: &X743Handle) -> u32 {
    h.read_register(0x8178).unwrap_or(0xBAD00001)
}

/// RAII guard: SWStopAcquisition + ClearData on drop so the hardware never
/// exits the test in a still-Running state, even if we `?` out mid-cycle.
struct AcqGuard<'a> {
    h: &'a X743Handle,
}

impl<'a> AcqGuard<'a> {
    fn new(h: &'a X743Handle) -> Self {
        Self { h }
    }
}

impl<'a> Drop for AcqGuard<'a> {
    fn drop(&mut self) {
        match self.h.sw_stop_acquisition() {
            Ok(()) => info!("AcqGuard: SWStopAcquisition"),
            Err(e) => warn!("AcqGuard: SWStopAcquisition failed: {}", e),
        }
        match self.h.clear_data() {
            Ok(()) => info!("AcqGuard: ClearData"),
            Err(e) => warn!("AcqGuard: ClearData failed: {}", e),
        }
    }
}

fn apply_minimal_config(h: &X743Handle) -> Result<(), DigitizerError> {
    info!("Applying minimal config");
    h.reset()?;

    // Group 0 only (channels 0, 1)
    h.set_group_enable_mask(0b0000_0001)?;

    // Post-trigger size for group 0
    h.set_sam_post_trigger_size(0, 40)?;

    // Sampling
    h.set_sam_sampling_frequency(SamFrequency::Ghz3_2)?;

    // Disable test pulse generator
    for ch in 0..MAX_CHANNELS as u32 {
        h.disable_sam_pulse_gen(ch)?;
    }

    // Ch0: enabled, self-trigger, negative polarity, DC offset 50%, threshold -0.5 V
    h.set_channel_dc_offset(0, 32768)?;
    h.set_channel_trigger_threshold(0, 45874)?;
    h.set_trigger_polarity(0, TriggerPolarity::FallingEdge)?;
    h.set_channel_self_trigger(TriggerMode::AcqOnly, 0b0000_0001)?;

    // Ch1: also needs dc offset / polarity for cleanliness, but no self-trigger
    h.set_channel_dc_offset(1, 32768)?;
    h.set_trigger_polarity(1, TriggerPolarity::FallingEdge)?;

    // Trigger source: self-trigger only (SW/Ext disabled)
    h.set_sw_trigger_mode(TriggerMode::Disabled)?;
    h.set_ext_trigger_input_mode(TriggerMode::Disabled)?;

    // SAM correction
    h.set_sam_correction_level(SamCorrectionLevel::All)?;

    // Max events per BLT — kept small so read loop drains quickly and our
    // readout buffer is unlikely to overflow during one transfer.
    h.set_max_num_events_blt(100)?;

    // Record length (samples)
    h.set_record_length(256)?;

    // I/O level
    h.set_io_level(IOLevel::NIM)?;

    // SW-controlled acquisition
    h.set_acquisition_mode(AcqMode::SWControlled)?;

    info!("Minimal config applied");
    Ok(())
}

fn drain_data(
    h: &X743Handle,
    buf: &mut ReadoutBuffer,
    event_buf: Option<&mut EventBuffer>,
    label: &str,
) -> Result<u64, DigitizerError> {
    let mut total_bytes = 0u64;
    let mut total_events = 0u64;
    let mut iters = 0u32;
    let mut eb = event_buf;
    loop {
        match h.read_data(buf) {
            Ok(0) => break,
            Ok(n) => {
                total_bytes += n as u64;
                if let Some(ref mut ebuf) = eb {
                    if let Ok(num) = h.get_num_events(buf, n) {
                        for i in 0..num {
                            if let Ok((_info, ptr)) = h.get_event_info(buf, n, i) {
                                if h.decode_event(ptr, ebuf).is_ok() {
                                    total_events += 1;
                                }
                            }
                        }
                    }
                }
                iters += 1;
                if iters > 200 {
                    warn!("{}: drain loop exceeded 200 iters, breaking", label);
                    break;
                }
            }
            Err(e) => {
                warn!("{}: ReadData error: {}", label, e);
                break;
            }
        }
    }
    if total_events > 0 {
        info!(
            "{}: drained {} events ({} bytes)",
            label, total_events, total_bytes
        );
    }
    Ok(total_bytes)
}

/// T6 helper: spawn tokio runtime with ZMQ activity to simulate the production
/// reader's tokio runtime + ZMQ data socket. Returns the runtime so the caller
/// can keep it alive (drop = shutdown). All async tasks run on the runtime;
/// the cycle test continues on the calling thread (std::thread).
fn spawn_zmq_noise() -> Result<tokio::runtime::Runtime, Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    // PUB socket: publish ~1 MB buffers @ 50 Hz, mirrors production data PUB
    rt.spawn(async {
        use delila_rs::common::pub_no_hwm;
        use futures::SinkExt;
        use tmq::{publish, Context};
        let ctx = Context::new();
        let mut socket = match publish(&ctx).bind("tcp://*:54330") {
            Ok(s) => {
                // Best-effort: this is a noise-traffic emitter inside a hardware
                // reproducer; if HWM cannot be set we keep going.
                let _ = pub_no_hwm(&s);
                s
            }
            Err(e) => {
                warn!("noise PUB bind failed: {}", e);
                return;
            }
        };
        let data = vec![0u8; 1024 * 1024];
        loop {
            let payload = data.clone();
            let msg: tmq::Multipart = vec![tmq::Message::from(payload.as_slice())].into();
            if let Err(e) = socket.send(msg).await {
                warn!("noise PUB send: {}", e);
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
    });

    // CPU spinner: simulate scheduler pressure on tokio worker threads
    for i in 0..2 {
        rt.spawn(async move {
            loop {
                let mut sum: u64 = 0;
                for k in 0..1_000_000u64 {
                    sum = sum.wrapping_add(k.wrapping_mul(i + 1));
                }
                std::hint::black_box(sum);
                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
            }
        });
    }

    info!("ZMQ noise spawned: PUB tcp://*:54330 (50 Hz, 1MB) + 2 CPU spinners");
    Ok(rt)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let mode = Mode::parse(&args.mode)?;

    let link_type = match args.link_type.as_str() {
        "optical" => ConnectionType::OpticalLink,
        "usb" => ConnectionType::USB,
        other => return Err(format!("bad link_type '{}'", other).into()),
    };

    info!(
        mode = ?mode,
        cycles = args.cycles,
        run_ms = args.run_ms,
        reopen = args.reopen,
        "V1743 Start/Stop cycle test"
    );

    // T6: optionally spawn tokio + ZMQ noise to mirror production reader's
    // runtime activity. Kept alive for entire test (dropped at end of main).
    let _noise_rt = if args.zmq_noise {
        Some(spawn_zmq_noise()?)
    } else {
        None
    };

    // Load production DigitizerConfig if requested; otherwise we'll use the
    // hardcoded minimal path.
    let prod_cfg: Option<DigitizerConfig> = match args.prod_config.as_deref() {
        Some(p) => {
            info!("Loading prod DigitizerConfig from {}", p);
            let s = std::fs::read_to_string(p)?;
            Some(serde_json::from_str(&s)?)
        }
        None => None,
    };

    let mut log = Log::new(args.csv.as_deref(), mode);
    let run_dur = Duration::from_millis(args.run_ms);

    // Outer loop: with --reopen, each session is exactly 1 cycle (Open→Apply→
    // 1 Start/Stop→Close). Without --reopen, one session holds the handle for
    // all cycles.
    let mut global_cycle: u32 = 0;
    while global_cycle < args.cycles {
        let session_cycles = if args.reopen {
            1
        } else {
            args.cycles - global_cycle
        };

        let h = X743Handle::open(link_type, args.link_num, args.conet_node, 0)?;

        // T7: with --alloc-before-apply, allocate buffers immediately after
        // Open (with default board state), exactly like production's
        // reader/mod.rs:2148. With this, the CAEN library sizes the buffer
        // for default record_length, then apply_config_standard changes
        // record_length — buffer may be undersized for the new DMA needs.
        let pre_apply_buf = if args.alloc_before_apply {
            let b = h.malloc_readout_buffer()?;
            let eb = if args.decode_events {
                Some(h.allocate_event()?)
            } else {
                None
            };
            info!(
                "Pre-apply: ReadoutBuffer allocated_size={} bytes (BEFORE apply_config)",
                b.allocated_size()
            );
            Some((b, eb))
        } else {
            None
        };

        if let Some(ref cfg) = prod_cfg {
            info!("Applying prod config via apply_config_standard");
            h.apply_config_standard(cfg)?;
        } else {
            apply_minimal_config(&h)?;
        }

        let (mut buf, mut event_buf): (ReadoutBuffer, Option<EventBuffer>) = match pre_apply_buf {
            Some((b, eb)) => {
                info!("Post-apply: ReadoutBuffer allocated_size still={} bytes (alloc_before_apply, NOT re-mallocd after apply)", b.allocated_size());
                (b, eb)
            }
            None => {
                let b = h.malloc_readout_buffer()?;
                let eb = if args.decode_events {
                    Some(h.allocate_event()?)
                } else {
                    None
                };
                info!(
                    "Post-apply: ReadoutBuffer allocated_size={} bytes (AFTER apply_config)",
                    b.allocated_size()
                );
                (b, eb)
            }
        };

        // Pre-cycle defensive stop+clear in case previous session left state.
        if let Err(e) = h.sw_stop_acquisition() {
            warn!("pre-cycle SWStopAcquisition: {}", e);
        }
        h.clear_data()?;

        if global_cycle == 0 {
            log.record(0, "after_apply", Some(read_status(&h)), "");
        }

        // Guard ensures this session leaves hw Stopped+Cleared even on ?-exit.
        // Can be disabled with --no-guard to exactly mirror production (which
        // has no such post-Stop ClearData).
        let _guard = if args.no_guard {
            None
        } else {
            Some(AcqGuard::new(&h))
        };

        for _ in 0..session_cycles {
            global_cycle += 1;
            let cycle = global_cycle;
            // ---- (optional) DOUBLE RESET ----
            // T3: production's state machine calls h.reset() in the "Reset to Idle"
            // block (mod.rs:2287) when the operator's Phase 0 sends Reset, and then
            // apply_config_standard does its own h.reset() (handle.rs:590) at the
            // start of the Configure phase. The two Resets fire ~7 ms to ~1.7 s
            // apart. Standalone normally has only one (apply's). This flag adds
            // the outer Reset to mirror production exactly.
            if args.double_reset {
                h.reset()?;
                log.record(cycle, "after_outer_reset", Some(read_status(&h)), "");
            }

            // ---- (optional) RE-APPLY CONFIG ----
            // Simulates the Reader's behaviour where every Configure command or
            // every Tune Up Apply triggers apply_config_standard afresh.
            if args.reapply {
                if let Some(ref cfg) = prod_cfg {
                    h.apply_config_standard(cfg)?;
                } else {
                    apply_minimal_config(&h)?;
                }
                log.record(cycle, "after_reapply", Some(read_status(&h)), "");

                // T2: realloc readout buffer + event buffer to clear any stale
                // lib-internal pointers that Reset may have invalidated.
                if args.realloc_buf {
                    drop(buf);
                    buf = h.malloc_readout_buffer()?;
                    if event_buf.is_some() {
                        // Drop the old EventBuffer first, then reallocate.
                        // `take()` makes the order explicit so clippy doesn't
                        // flag the intermediate `None` write as dead.
                        let _ = event_buf.take();
                        event_buf = Some(h.allocate_event()?);
                    }
                    log.record(cycle, "after_realloc_buf", Some(read_status(&h)), "");
                }
            }

            // ---- START ----
            match mode {
                Mode::A | Mode::D => {}
                Mode::B | Mode::C => h.clear_data()?,
            }
            let s_before_start = read_status(&h);
            h.sw_start_acquisition()?;
            let s_after_start = read_status(&h);
            log.record(
                cycle,
                "after_start",
                Some(s_after_start),
                &format!("before_start=0x{:08X}", s_before_start),
            );

            // ---- RUN ----
            let run_start = Instant::now();
            let mut total_bytes = 0u64;
            let mut total_events = 0u64;
            let mut read_err: Option<String> = None;
            if args.fill_fifo {
                // Don't touch the buffer — let board FIFO fill / overflow.
                std::thread::sleep(run_dur);
            } else {
                while run_start.elapsed() < run_dur {
                    match h.read_data(&mut buf) {
                        Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                        Ok(n) => {
                            total_bytes += n as u64;
                            // Mirror production: per-event get_event_info + decode_event
                            if let Some(ref mut eb) = event_buf {
                                match h.get_num_events(&buf, n) {
                                    Ok(num) => {
                                        for i in 0..num {
                                            match h.get_event_info(&buf, n, i) {
                                                Ok((_info, ptr)) => {
                                                    if let Err(e) = h.decode_event(ptr, eb) {
                                                        warn!(
                                                            "DecodeEvent cycle {} idx {}: {}",
                                                            cycle, i, e
                                                        );
                                                    } else {
                                                        total_events += 1;
                                                    }
                                                }
                                                Err(e) => {
                                                    warn!(
                                                        "GetEventInfo cycle {} idx {}: {}",
                                                        cycle, i, e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => warn!("GetNumEvents cycle {}: {}", cycle, e),
                                }
                            }
                        }
                        Err(e) => {
                            // Non-fatal: board FIFO overflow (OutOfMemory) or transient.
                            // Log and break this cycle's read loop — we still want to
                            // proceed to Stop so the hardware exits cleanly.
                            read_err = Some(e.to_string());
                            warn!("ReadData during cycle {} run: {}", cycle, e);
                            break;
                        }
                    }
                }
            }
            log.record(
                cycle,
                "run_done",
                Some(read_status(&h)),
                &format!(
                    "bytes_read={} events={}{}",
                    total_bytes,
                    total_events,
                    read_err
                        .as_deref()
                        .map(|s| format!(" err={}", s))
                        .unwrap_or_default()
                ),
            );

            // ---- STOP ----
            h.sw_stop_acquisition()?;
            let s_after_stop = read_status(&h);
            log.record(cycle, "after_stop", Some(s_after_stop), "");

            match mode {
                Mode::A => {
                    h.clear_data()?;
                }
                Mode::B => {
                    drain_data(&h, &mut buf, event_buf.as_mut(), "post_stop_drain")?;
                }
                Mode::C => {
                    drain_data(&h, &mut buf, event_buf.as_mut(), "post_stop_drain")?;
                    h.clear_data()?;
                }
                Mode::D => {}
            }
            log.record(cycle, "post_procedure", Some(read_status(&h)), "");
        }
        // _guard drops here (Stop + ClearData), then h drops (CloseDigitizer).
    }

    info!("All {} cycles completed successfully", args.cycles);
    Ok(())
}

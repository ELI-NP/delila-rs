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
use tracing::{error, info, warn};

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
        let status_s = status.map(|s| format!("0x{:08X}", s)).unwrap_or_else(|| "-".to_string());
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
    label: &str,
) -> Result<u64, DigitizerError> {
    let mut total_bytes = 0u64;
    let mut iters = 0u32;
    loop {
        match h.read_data(buf) {
            Ok(0) => break,
            Ok(n) => {
                total_bytes += n as u64;
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
    Ok(total_bytes)
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

        if let Some(ref cfg) = prod_cfg {
            info!("Applying prod config via apply_config_standard");
            h.apply_config_standard(cfg)?;
        } else {
            apply_minimal_config(&h)?;
        }

        // Allocate readout buffer AFTER configuring so CAEN sizes it for the
        // actual record_length / max_num_events_blt we just set.
        let mut buf = h.malloc_readout_buffer()?;

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
        let mut read_err: Option<String> = None;
        if args.fill_fifo {
            // Don't touch the buffer — let board FIFO fill / overflow.
            std::thread::sleep(run_dur);
        } else {
            while run_start.elapsed() < run_dur {
                match h.read_data(&mut buf) {
                    Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                    Ok(n) => total_bytes += n as u64,
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
                "bytes_read={}{}",
                total_bytes,
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
                drain_data(&h, &mut buf, "post_stop_drain")?;
            }
            Mode::C => {
                drain_data(&h, &mut buf, "post_stop_drain")?;
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

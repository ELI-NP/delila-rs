//! `pha_fw_scan` — FW-side DPP-PHA trapezoid scan over the Operator REST API
//! (TODO 59 §8).
//!
//! For every rise × flat-top point: program the selected boards, take a short
//! list-mode run (`/api/run/start` … `/api/stop`), read that run's `.delila`
//! files and fit the ⁶⁰Co lines of every channel. The first point is always
//! the configuration as found ("as configured"), measured under the same
//! conditions as the grid.
//!
//! The boards' original configs are saved to `--out-dir` before anything is
//! changed and re-applied at the end — also on an error, Ctrl-C or SIGTERM.
//! The result is a per-channel table, a CSV, and `dig<ID>_proposed.json`
//! (original + a per-channel trapezoid override at each channel's best point)
//! for review. Nothing proposed is applied.
//!
//! Run it ON the DAQ host — it reads the Recorder's files directly:
//!
//! ```text
//! pha_fw_scan --data-dir ~/DELILA_data --digitizers 0,1,2 \
//!     --rise 2000:8000:1000 --flat 1200 --seconds 300 [--dry-run]
//! ```
//!
//! Waveforms should be OFF (list mode): the FW energy is all that is used, and
//! waveforms with a long record trigger the DIG1 couple arbiter (§5.7).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Parser;
use rayon::prelude::*;
use tokio::sync::watch;

use delila_rs::config::DigitizerConfig;
use delila_rs::offline::fw_scan::{
    self, best_point, keeps_peak, measure_channel, ChannelMeasurement,
};
use delila_rs::offline::scan::parse_values;
use delila_rs::operator::{ApiResponse, ConfigureRequest, SystemState, SystemStatus};
use delila_rs::recorder::DataFileReader;

#[derive(Parser, Debug)]
#[command(about = "FW-side trapezoid scan via the Operator REST API (TODO 59)")]
struct Args {
    /// Operator base URL.
    #[arg(long, default_value = "http://localhost:9090")]
    operator: String,
    /// The Recorder's output directory (where run<NNNN>_*.delila appear).
    #[arg(long)]
    data_dir: PathBuf,
    /// Digitizer (source) IDs to program, comma-separated.
    #[arg(long, value_delimiter = ',', required = true)]
    digitizers: Vec<u32>,
    /// Trapezoid rise times, ns: lo:hi:step or a,b,c.
    #[arg(long)]
    rise: String,
    /// Flat-top times, ns: lo:hi:step or a,b,c.
    #[arg(long)]
    flat: String,
    /// Run length per point, s.
    #[arg(long, default_value_t = 300)]
    seconds: u64,
    /// ADC sampling period, ns (V1725: 4). The FW step is 4 samples.
    #[arg(long, default_value_t = 4.0)]
    sample_ns: f64,
    /// First run number (default: the Operator's next run number).
    #[arg(long)]
    first_run: Option<u32>,
    /// Output directory (backups, CSV, summary, proposed configs).
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Print the plan and exit without touching the DAQ.
    #[arg(long)]
    dry_run: bool,
}

/// One grid point: the configuration as found, or a board-wide trapezoid.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Point {
    AsConfigured,
    Trap { rise: u32, flat: u32 },
}

impl Point {
    fn label(&self) -> String {
        match self {
            Point::AsConfigured => "as-configured".into(),
            Point::Trap { rise, flat } => format!("{rise}/{flat}"),
        }
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Raised when Ctrl-C / SIGTERM arrives mid-scan.
#[derive(Debug)]
struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cancelled by signal")
    }
}
impl std::error::Error for Cancelled {}

// ─────────────────────────────── REST client ───────────────────────────────

struct Operator {
    base: String,
    http: reqwest::Client,
}

impl Operator {
    fn new(base: &str) -> Result<Self, BoxError> {
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                // /api/run/start runs reset + configure + arm + start.
                .timeout(Duration::from_secs(120))
                .build()?,
        })
    }

    async fn status(&self) -> Result<SystemStatus, BoxError> {
        let url = format!("{}/api/status", self.base);
        Ok(self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn digitizer(&self, id: u32) -> Result<DigitizerConfig, BoxError> {
        let url = format!("{}/api/digitizers/{id}", self.base);
        Ok(self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// POST and require `success: true`, whatever the HTTP status.
    async fn post<T: serde::Serialize>(&self, path: &str, body: &T) -> Result<String, BoxError> {
        let url = format!("{}{path}", self.base);
        let resp = self.http.post(url).json(body).send().await?;
        let code = resp.status();
        let api: ApiResponse = resp.json().await?;
        if api.success {
            Ok(api.message)
        } else {
            Err(format!("{path} → HTTP {code}: {}", api.message).into())
        }
    }

    async fn apply(&self, config: &DigitizerConfig) -> Result<String, BoxError> {
        self.post(
            &format!("/api/digitizers/{}/apply", config.digitizer_id),
            config,
        )
        .await
    }

    async fn run_start(&self, run_number: u32, comment: String) -> Result<String, BoxError> {
        let req = ConfigureRequest {
            run_number,
            comment,
            exp_name: String::new(), // the Operator uses its configured name
        };
        self.post("/api/run/start", &req).await
    }

    async fn stop(&self) -> Result<String, BoxError> {
        self.post("/api/stop", &serde_json::json!({})).await
    }

    /// Poll until the system reaches `want` (or `timeout` passes).
    async fn wait_for(&self, want: SystemState, timeout: Duration) -> Result<(), BoxError> {
        let t0 = Instant::now();
        loop {
            let s = self.status().await?;
            if s.system_state == want {
                return Ok(());
            }
            if t0.elapsed() > timeout {
                return Err(format!(
                    "system is {:?} after {:?} (waiting for {want:?}){}",
                    s.system_state,
                    timeout,
                    describe_components(&s)
                )
                .into());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// Components not in the system state, with their errors — for messages.
fn describe_components(s: &SystemStatus) -> String {
    let lines: Vec<String> = s
        .components
        .iter()
        .map(|c| {
            format!(
                "\n  {} = {:?}{}{}",
                c.name,
                c.state,
                if c.online { "" } else { " (offline)" },
                c.error
                    .as_deref()
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default()
            )
        })
        .collect();
    lines.concat()
}

// ──────────────────────────────── data files ───────────────────────────────

/// FW energies per (module, channel) from all files of one run.
fn read_run(data_dir: &Path, run: u32) -> Result<BTreeMap<(u8, u8), Vec<u16>>, BoxError> {
    let prefix = format!("run{run:04}_");
    let mut files: Vec<PathBuf> = std::fs::read_dir(data_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".delila"))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no {prefix}*.delila in {}", data_dir.display()).into());
    }
    let mut out: BTreeMap<(u8, u8), Vec<u16>> = BTreeMap::new();
    for path in &files {
        let mut reader = DataFileReader::new(BufReader::new(File::open(path)?))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        for block in reader.data_blocks() {
            let block = block.map_err(|e| format!("{}: {e}", path.display()))?;
            for e in &block.events {
                out.entry((e.module, e.channel)).or_default().push(e.energy);
            }
        }
    }
    Ok(out)
}

// ───────────────────────────────── the scan ────────────────────────────────

/// Sleep, unless a signal arrives first.
async fn sleep_or_cancel(d: Duration, cancel: &mut watch::Receiver<bool>) -> Result<(), BoxError> {
    if *cancel.borrow() {
        return Err(Box::new(Cancelled));
    }
    tokio::select! {
        _ = tokio::time::sleep(d) => Ok(()),
        _ = cancel.changed() => Err(Box::new(Cancelled)),
    }
}

/// Program, run, stop and measure one point.
async fn measure_point(
    op: &Operator,
    args: &Args,
    originals: &[DigitizerConfig],
    point: Point,
    run: u32,
    cancel: &mut watch::Receiver<bool>,
) -> Result<BTreeMap<(u8, u8), ChannelMeasurement>, BoxError> {
    for original in originals {
        let mut cfg = original.clone();
        if let Point::Trap { rise, flat } = point {
            fw_scan::set_board_trap(&mut cfg, rise, flat);
        }
        op.apply(&cfg)
            .await
            .map_err(|e| format!("apply digitizer {}: {e}", cfg.digitizer_id))?;
    }
    let comment = format!("pha_fw_scan {} (rise/flat ns)", point.label());
    op.run_start(run, comment).await?;
    println!("  run {run} started, {} s …", args.seconds);

    let t0 = Instant::now();
    let length = Duration::from_secs(args.seconds);
    // Check the state after every wait, the last one included: a run that died
    // in its final seconds is as broken as one that died early.
    let result = async {
        loop {
            let remaining = length.saturating_sub(t0.elapsed());
            sleep_or_cancel(Duration::from_secs(5).min(remaining), cancel).await?;
            let s = op.status().await?;
            if s.system_state != SystemState::Running {
                return Err::<(), BoxError>(
                    format!(
                        "run {run} left Running ({:?}){}",
                        s.system_state,
                        describe_components(&s)
                    )
                    .into(),
                );
            }
            if t0.elapsed() >= length {
                return Ok(());
            }
        }
    }
    .await;
    // Stop in every case (a cancelled or failed run must not keep running).
    let stopped = op.stop().await;
    result?;
    stopped?;
    op.wait_for(SystemState::Configured, Duration::from_secs(60))
        .await?;

    // The Recorder finalizes its last file on Stop; give it a few tries.
    let mut last_err: Option<BoxError> = None;
    for _ in 0..5 {
        match read_run(&args.data_dir, run) {
            Ok(spectra) => {
                return Ok(spectra
                    .into_par_iter()
                    .map(|(key, energies)| (key, measure_channel(&energies)))
                    .collect());
            }
            Err(e) => last_err = Some(e),
        }
        sleep_or_cancel(Duration::from_secs(2), cancel).await?;
    }
    Err(last_err.unwrap_or_else(|| "no data".into()))
}

fn fmt_opt(v: Option<f64>, prec: usize) -> String {
    v.map_or_else(|| "-".into(), |v| format!("{v:.prec$}"))
}

fn print_point(point: Point, run: u32, results: &BTreeMap<(u8, u8), ChannelMeasurement>) {
    println!("  {} run {run}:", point.label());
    for ((m, c), r) in results {
        if r.high.is_none() && r.events < 1000 {
            continue; // empty / noise-only channel
        }
        println!(
            "    mod {m} ch {c:>2}: FWHM@1332 {} keV (N={}), @1173 {} keV, events {}",
            fmt_opt(r.fwhm_kev_high(), 3),
            r.high.map_or(0, |f| f.counts.round() as i64),
            fmt_opt(r.fwhm_kev_low(), 3),
            r.events
        );
    }
}

fn csv_row(
    w: &mut impl Write,
    idx: usize,
    point: Point,
    run: u32,
    results: &BTreeMap<(u8, u8), ChannelMeasurement>,
) -> std::io::Result<()> {
    let (rise, flat) = match point {
        Point::Trap { rise, flat } => (rise.to_string(), flat.to_string()),
        Point::AsConfigured => (String::new(), String::new()),
    };
    for ((m, c), r) in results {
        writeln!(
            w,
            "{idx},{},{rise},{flat},{run},{m},{c},{},{},{},{},{},{},{},{}",
            point.label(),
            r.events,
            fmt_opt(r.fwhm_kev_high(), 4),
            fmt_opt(r.high.map(|f| f.centroid), 2),
            fmt_opt(r.high.map(|f| f.counts), 0),
            fmt_opt(r.fwhm_kev_low(), 4),
            fmt_opt(r.low.map(|f| f.centroid), 2),
            fmt_opt(r.low.map(|f| f.counts), 0),
            fmt_opt(r.peak_fraction(), 6),
        )?;
    }
    w.flush()
}

/// Trapezoid picks: module → channel → (rise, flat).
type Picks = BTreeMap<u8, BTreeMap<u8, (u32, u32)>>;

/// Per-channel ranking, printed and written to `summary.txt`; returns the
/// trapezoid picks per module (channels whose best point is a grid point).
fn summarize(
    points: &[Point],
    runs: &[u32],
    measured: &[BTreeMap<(u8, u8), ChannelMeasurement>],
    out: &mut impl Write,
) -> std::io::Result<Picks> {
    let mut keys: Vec<(u8, u8)> = measured.iter().flat_map(|m| m.keys().copied()).collect();
    keys.sort();
    keys.dedup();
    let mut picks = Picks::new();
    writeln!(
        out,
        "FWHM at 1332 keV (keV); * = best, ! = lost > 5 % of the peak"
    )?;
    write!(out, "{:>13}", "point")?;
    for (m, c) in &keys {
        write!(out, " {:>9}", format!("{m}/{c}"))?;
    }
    writeln!(out)?;
    let columns: Vec<Vec<ChannelMeasurement>> = keys
        .iter()
        .map(|k| {
            measured
                .iter()
                .map(|m| {
                    m.get(k).copied().unwrap_or(ChannelMeasurement {
                        events: 0,
                        high: None,
                        low: None,
                    })
                })
                .collect()
        })
        .collect();
    let best: Vec<Option<usize>> = columns.iter().map(|col| best_point(col)).collect();
    for (i, point) in points.iter().enumerate().take(measured.len()) {
        write!(out, "{:>13}", point.label())?;
        for (k, col) in columns.iter().enumerate() {
            let cell = match col[i].fwhm_kev_high() {
                None => "-".to_string(),
                Some(w) => {
                    let mark = if best[k] == Some(i) {
                        "*"
                    } else if !keeps_peak(&col[i], col) {
                        "!"
                    } else {
                        " "
                    };
                    format!("{w:.3}{mark}")
                }
            };
            write!(out, " {cell:>9}")?;
        }
        writeln!(out, "   run {}", runs[i])?;
    }
    writeln!(out)?;
    for (k, &(m, c)) in keys.iter().enumerate() {
        let col = &columns[k];
        let Some(b) = best[k] else {
            writeln!(out, "mod {m} ch {c:>2}: no ⁶⁰Co pair found at any point")?;
            continue;
        };
        let w = col[b].fwhm_kev_high().unwrap_or(f64::NAN);
        let n = col[b].high.map_or(0.0, |f| f.counts);
        let reference = col[0].fwhm_kev_high();
        writeln!(
            out,
            "mod {m} ch {c:>2}: best {} → {w:.3} keV (±{:.1} % stat){}",
            points[b].label(),
            150.0 / n.max(1.0).sqrt(),
            reference.map_or_else(String::new, |r| format!(
                ", as configured {r:.3} keV ({:+.1} %)",
                100.0 * (w / r - 1.0)
            ))
        )?;
        if let Point::Trap { rise, flat } = points[b] {
            picks.entry(m).or_default().insert(c, (rise, flat));
        }
    }
    Ok(picks)
}

async fn restore(op: &Operator, originals: &[DigitizerConfig], out_dir: &Path) -> bool {
    let mut ok = true;
    for cfg in originals {
        match op.apply(cfg).await {
            Ok(_) => println!("restored digitizer {}", cfg.digitizer_id),
            Err(e) => {
                ok = false;
                eprintln!(
                    "ERROR: could not restore digitizer {}: {e}\n  restore by hand:\n  curl -X POST -H 'Content-Type: application/json' \
                     --data @{}/dig{}_original.json {}/api/digitizers/{}/apply",
                    cfg.digitizer_id,
                    out_dir.display(),
                    cfg.digitizer_id,
                    op.base,
                    cfg.digitizer_id
                );
            }
        }
    }
    ok
}

async fn run(args: Args) -> Result<(), BoxError> {
    let step = fw_scan::trap_step_ns(args.sample_ns);
    let rises = parse_values(&args.rise).map_err(|e| format!("--rise: {e}"))?;
    let flats = parse_values(&args.flat).map_err(|e| format!("--flat: {e}"))?;
    let mut points = vec![Point::AsConfigured];
    points.extend(
        fw_scan::grid(&rises, &flats, step)
            .into_iter()
            .map(|(rise, flat)| Point::Trap { rise, flat }),
    );

    let op = Operator::new(&args.operator)?;
    let status = op.status().await?;
    if status.tuneup_mode {
        return Err("Tune Up is active — stop it first".into());
    }
    if !matches!(
        status.system_state,
        SystemState::Idle | SystemState::Configured
    ) {
        return Err(format!(
            "system is {:?}; need Idle or Configured{}",
            status.system_state,
            describe_components(&status)
        )
        .into());
    }
    let first_run = match (args.first_run, status.next_run_number) {
        (Some(r), _) => r,
        (None, Some(r)) if r >= 0 => r as u32,
        _ => {
            return Err("no next run number from the Operator (MongoDB?) — pass --first-run".into())
        }
    };
    let mut originals = Vec::new();
    for &id in &args.digitizers {
        originals.push(op.digitizer(id).await?);
    }

    println!(
        "pha_fw_scan: {} points × {} s, FW step {step} ns",
        points.len(),
        args.seconds
    );
    for o in &originals {
        let d = &o.channel_defaults;
        println!(
            "  digitizer {} ({}): now rise {:?} / flat {:?} ns, {} per-channel overrides",
            o.digitizer_id,
            o.name,
            d.trap_rise_time_ns,
            d.trap_flat_top_ns,
            o.channel_overrides.len()
        );
    }
    for (i, p) in points.iter().enumerate() {
        println!("  point {i}: {} → run {}", p.label(), first_run + i as u32);
    }
    let eta = points.len() as u64 * (args.seconds + 40);
    println!("  estimated time ≈ {} min", eta.div_ceil(60));
    if args.dry_run {
        println!("dry run — nothing changed");
        return Ok(());
    }

    let out_dir = args.out_dir.clone().unwrap_or_else(|| {
        PathBuf::from(format!(
            "fw_scan_{}",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ))
    });
    std::fs::create_dir_all(&out_dir)?;
    for o in &originals {
        let path = out_dir.join(format!("dig{}_original.json", o.digitizer_id));
        std::fs::write(&path, serde_json::to_string_pretty(o)?)?;
    }
    println!("backups in {}", out_dir.display());

    // Ctrl-C / SIGTERM → cancel (stop the run, restore, report what we have).
    let (cancel_tx, mut cancel) = watch::channel(false);
    tokio::spawn(async move {
        let term = async {
            #[cfg(unix)]
            {
                if let Ok(mut s) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                {
                    s.recv().await;
                    return;
                }
            }
            std::future::pending::<()>().await
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term => {}
        }
        eprintln!("signal received — stopping the run and restoring the boards …");
        let _ = cancel_tx.send(true);
    });

    let mut csv = BufWriter::new(File::create(out_dir.join("fw_scan.csv"))?);
    writeln!(
        csv,
        "point,label,rise_ns,flat_ns,run,module,channel,events,fwhm_kev_1332,centroid_1332,counts_1332,fwhm_kev_1173,centroid_1173,counts_1173,peak_fraction"
    )?;
    let mut runs = Vec::new();
    let mut measured = Vec::new();
    let mut outcome: Result<(), BoxError> = Ok(());
    for (i, &point) in points.iter().enumerate() {
        let run = first_run + i as u32;
        println!("[{}/{}] {}", i + 1, points.len(), point.label());
        match measure_point(&op, &args, &originals, point, run, &mut cancel).await {
            Ok(results) => {
                print_point(point, run, &results);
                csv_row(&mut csv, i, point, run, &results)?;
                runs.push(run);
                measured.push(results);
            }
            Err(e) => {
                outcome = Err(e);
                break;
            }
        }
    }

    // A failure between Start and Stop can leave the run going; the boards can
    // only be re-programmed from Idle / Configured.
    if let Ok(s) = op.status().await {
        if matches!(s.system_state, SystemState::Running | SystemState::Armed) {
            if let Err(e) = op.stop().await {
                eprintln!("ERROR: stop before restore: {e}");
            }
            if let Err(e) = op
                .wait_for(SystemState::Configured, Duration::from_secs(60))
                .await
            {
                eprintln!("ERROR: {e}");
            }
        }
    }
    let restored = restore(&op, &originals, &out_dir).await;

    if !measured.is_empty() {
        let mut text = Vec::new();
        let picks = summarize(&points, &runs, &measured, &mut text)?;
        let text = String::from_utf8_lossy(&text);
        println!("\n{text}");
        std::fs::write(out_dir.join("summary.txt"), text.as_bytes())?;
        // Events carry `module` = the source's module_id, which defaults to the
        // source id = digitizer id.
        for o in &originals {
            let Ok(module) = u8::try_from(o.digitizer_id) else {
                continue;
            };
            if !measured
                .iter()
                .any(|m| m.keys().any(|&(mm, _)| mm == module))
            {
                eprintln!(
                    "WARNING: no events with module {module} — digitizer {} has a module_id \
                     override? No proposal written for it.",
                    o.digitizer_id
                );
            }
            if let Some(p) = picks.get(&module) {
                let path = out_dir.join(format!("dig{}_proposed.json", o.digitizer_id));
                let proposed = fw_scan::with_channel_traps(o, p);
                std::fs::write(&path, serde_json::to_string_pretty(&proposed)?)?;
                println!("proposed (NOT applied): {}", path.display());
            }
        }
    }
    if !restored {
        return Err("boards NOT restored — see above".into());
    }
    outcome
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ERROR: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    //! End-to-end against a mock Operator: it records every apply, and on
    //! `/api/run/start` writes a ⁶⁰Co run whose resolution depends on the
    //! trapezoid rise that was applied (best at 6592 ns).
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Path as AxPath, State};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use delila_rs::common::{EventData, EventDataBatch};
    use delila_rs::config::FirmwareType;
    use delila_rs::offline::fw_scan::{CO60_HIGH_KEV, CO60_LOW_KEV};
    use delila_rs::recorder::{ChecksumCalculator, FileFooter, FileHeader};
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use rand_distr::{Distribution, Normal};

    const BEST_RISE: f64 = 6592.0;

    #[derive(Default)]
    struct Mock {
        running: bool,
        configs: HashMap<u32, DigitizerConfig>,
        applies: Vec<DigitizerConfig>,
        data_dir: PathBuf,
        next_run: i32,
        /// Report `Error` while this run is going.
        fail_run: Option<u32>,
        current_run: u32,
    }
    type Shared = Arc<Mutex<Mock>>;

    fn write_delila(path: &Path, batch: &EventDataBatch) {
        let mut buf = Vec::new();
        FileHeader::new(1, "mock".to_string(), 0)
            .write_to(&mut buf)
            .unwrap();
        let mut checksum = ChecksumCalculator::new();
        let mut footer = FileFooter::new();
        let data = batch.to_msgpack().unwrap();
        let len = (data.len() as u32).to_le_bytes();
        buf.extend_from_slice(&len);
        buf.extend_from_slice(&data);
        checksum.update(&len);
        checksum.update(&data);
        footer.total_events += batch.events.len() as u64;
        footer.data_checksum = checksum.finalize();
        footer.data_bytes = checksum.bytes_processed();
        footer.finalize();
        footer.write_to(&mut buf).unwrap();
        std::fs::write(path, buf).unwrap();
    }

    /// FWHM (keV) the mock detector shows for a rise time.
    fn mock_fwhm(rise: f64) -> f64 {
        3.0 * (1.0 + ((rise - BEST_RISE) / 4000.0).powi(2))
    }

    /// Channels 0..3 of every board: both ⁶⁰Co lines at 6 codes/keV plus noise.
    fn write_run(m: &Mock, run: u32) {
        let mut rng = StdRng::seed_from_u64(run as u64);
        let mut batch = EventDataBatch::new(0, 0);
        for (&id, cfg) in &m.configs {
            for ch in 0u8..3 {
                let rise = cfg
                    .channel_overrides
                    .get(&ch)
                    .and_then(|o| o.trap_rise_time_ns)
                    .or(cfg.channel_defaults.trap_rise_time_ns)
                    .unwrap_or(0) as f64;
                let sigma = mock_fwhm(rise) / 2.354_820_045 * 6.0;
                for (kev, n) in [(CO60_HIGH_KEV, 3000), (CO60_LOW_KEV, 3600)] {
                    let g = Normal::new(kev * 6.0, sigma).unwrap();
                    for _ in 0..n {
                        let e = g.sample(&mut rng).round() as u16;
                        batch.push(EventData::new(id as u8, ch, e, 0, 0.0, 0));
                    }
                }
                for _ in 0..20_000 {
                    let e = rng.gen_range(1..20u16);
                    batch.push(EventData::new(id as u8, ch, e, 0, 0.0, 0));
                }
            }
        }
        write_delila(
            &m.data_dir.join(format!("run{run:04}_0000_mock.delila")),
            &batch,
        );
    }

    async fn status(State(s): State<Shared>) -> Json<SystemStatus> {
        let m = s.lock().unwrap();
        let state = match (m.running, m.fail_run) {
            (true, Some(r)) if r == m.current_run => SystemState::Error,
            (true, _) => SystemState::Running,
            (false, _) => SystemState::Configured,
        };
        Json(SystemStatus {
            components: vec![],
            system_state: state,
            run_info: None,
            experiment_name: "mock".into(),
            next_run_number: Some(m.next_run),
            last_run_info: None,
            tuneup_mode: false,
            tuneup_digitizer_id: None,
            monitor_http_port: None,
        })
    }

    async fn get_dig(State(s): State<Shared>, AxPath(id): AxPath<u32>) -> Json<DigitizerConfig> {
        Json(s.lock().unwrap().configs[&id].clone())
    }

    async fn apply(
        State(s): State<Shared>,
        AxPath(id): AxPath<u32>,
        Json(cfg): Json<DigitizerConfig>,
    ) -> Json<ApiResponse> {
        let mut m = s.lock().unwrap();
        if m.running {
            return Json(ApiResponse::error("not in Idle/Configured"));
        }
        m.configs.insert(id, cfg.clone());
        m.applies.push(cfg);
        Json(ApiResponse::success("applied"))
    }

    async fn run_start(
        State(s): State<Shared>,
        Json(req): Json<ConfigureRequest>,
    ) -> Json<ApiResponse> {
        let mut m = s.lock().unwrap();
        m.running = true;
        m.current_run = req.run_number;
        m.next_run = req.run_number as i32 + 1;
        write_run(&m, req.run_number);
        Json(ApiResponse::success("started"))
    }

    async fn stop(State(s): State<Shared>) -> Json<ApiResponse> {
        s.lock().unwrap().running = false;
        Json(ApiResponse::success("stopped"))
    }

    fn board(id: u32) -> DigitizerConfig {
        let mut cfg = DigitizerConfig::new(id, format!("mock {id}"), FirmwareType::PHA1);
        cfg.channel_defaults.trap_rise_time_ns = Some(3008);
        cfg.channel_defaults.trap_flat_top_ns = Some(1008);
        cfg
    }

    async fn serve(mock: Mock) -> (String, Shared) {
        let shared: Shared = Arc::new(Mutex::new(mock));
        let app = Router::new()
            .route("/api/status", get(status))
            .route("/api/digitizers/:id", get(get_dig))
            .route("/api/digitizers/:id/apply", post(apply))
            .route("/api/run/start", post(run_start))
            .route("/api/stop", post(stop))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), shared)
    }

    fn args(operator: String, dir: &Path, seconds: u64) -> Args {
        Args {
            operator,
            data_dir: dir.join("data"),
            digitizers: vec![0, 1],
            rise: "3296,6592,9888".into(),
            flat: "1200".into(),
            seconds,
            sample_ns: 4.0,
            first_run: None,
            out_dir: Some(dir.join("out")),
            dry_run: false,
        }
    }

    fn mock(dir: &Path) -> Mock {
        std::fs::create_dir_all(dir.join("data")).unwrap();
        Mock {
            configs: HashMap::from([(0, board(0)), (1, board(1))]),
            data_dir: dir.join("data"),
            next_run: 40,
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scans_ranks_proposes_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        let (url, shared) = serve(mock(dir.path())).await;
        run(args(url, dir.path(), 0)).await.expect("scan");

        let m = shared.lock().unwrap();
        // 4 points × 2 boards + 2 restores; the restores are the originals.
        assert_eq!(m.applies.len(), 10);
        for id in [0, 1] {
            let c = &m.configs[&id];
            assert_eq!(c.channel_defaults.trap_rise_time_ns, Some(3008));
            assert_eq!(c.channel_defaults.trap_flat_top_ns, Some(1008));
        }
        assert_eq!(m.next_run, 44, "runs 40..43");

        let out = dir.path().join("out");
        let csv = std::fs::read_to_string(out.join("fw_scan.csv")).unwrap();
        assert_eq!(csv.lines().count(), 1 + 4 * 2 * 3, "{csv}");
        let summary = std::fs::read_to_string(out.join("summary.txt")).unwrap();
        assert!(summary.contains("mod 1 ch  2: best 6592/1200"), "{summary}");
        let proposed: DigitizerConfig =
            serde_json::from_str(&std::fs::read_to_string(out.join("dig0_proposed.json")).unwrap())
                .unwrap();
        assert_eq!(proposed.channel_defaults.trap_rise_time_ns, Some(3008));
        for ch in 0..3 {
            let o = &proposed.channel_overrides[&ch];
            assert_eq!(
                (o.trap_rise_time_ns, o.trap_flat_top_ns),
                (Some(6592), Some(1200))
            );
        }
        assert!(out.join("dig1_original.json").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_run_still_restores_the_boards() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = mock(dir.path());
        m.fail_run = Some(42); // the third point
        let (url, shared) = serve(m).await;
        let err = run(args(url, dir.path(), 1)).await.expect_err("must fail");
        assert!(err.to_string().contains("run 42 left Running"), "{err}");

        let m = shared.lock().unwrap();
        assert!(!m.running, "the failed run was stopped");
        for id in [0, 1] {
            assert_eq!(
                m.configs[&id].channel_defaults.trap_rise_time_ns,
                Some(3008)
            );
        }
        // Two good points are still summarized.
        let summary = std::fs::read_to_string(dir.path().join("out").join("summary.txt")).unwrap();
        assert!(summary.contains("as-configured"), "{summary}");
    }
}

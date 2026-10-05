//! `pha_trap_tune` — offline SW DPP-PHA trapezoid replay / validation.
//!
//! TODO 59 Phase 1 driver. Reads `.delila` waveform files, replays the software
//! trapezoid ([`delila_rs::offline::trap`]) with the FW's parameters, and reports
//! how well the SW energy reproduces the FW energy **per event** — the trust
//! anchor before any offline optimization (§4.2). It also compares the computed
//! peaking-window position against the FW `D1 = Peaking` digital probe.
//!
//! ```text
//! pha_trap_tune <file.delila>... [--ch C]
//!     [--rise-ns N] [--flat-ns N] [--pz-ns N]
//!     [--peak-pct P] [--peak-nsmean N] [--baseline-nsmean N]
//!     [--trigger-sample S]        # override D0-derived trigger anchor
//!     [--dump-trace OUT.csv]      # dump one event's input + trapezoid for overlay
//! ```
//!
//! Defaults match the es2 V1725 PHA config (rise 5000 / flat 1000 / pz 50000 ns,
//! peaking 80 %, PEAK_NSMEAN 1, BLINE_NSMEAN 256).
//!
//! # Scan mode (TODO 59 Phase 2)
//!
//! ```text
//! pha_trap_tune <file.delila>... --scan [--ch C] [--module M] [--probe 1|2]
//!     [--scan-rise-ns 1000:8000:500] [--scan-flat-ns 500:2000:250]   # lo:hi:step or a,b,c
//!     [--fw-window LO:HI]         # FW-energy window selecting ONE line (default: tallest peak ±1 %)
//!     [--line-kev E]              # report FWHM in keV for that line
//!     [--pz-auto | --pz-ns N]     # measure the decay constant from the pulses, or fix it
//!     [--peak-pct P] [--peak-nsmean N] [--peak-shift S]
//!     [--scan-csv OUT.csv]
//! ```
//!
//! Replays every (rise, flat-top) on the SAME events and ranks them by the
//! photopeak FWHM. Grid points the record cannot support (pre-trigger shorter
//! than `rise + (1 − peak%)·flat`) are listed as such, never evaluated.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;

use delila_rs::common::Waveform;
use delila_rs::offline::peak::fit_peak;
use delila_rs::offline::scan::{self, PointOutcome, ScanEvent};
use delila_rs::offline::trap::{self, TrapParams, WindowError};
use delila_rs::recorder::DataFileReader;
use rayon::prelude::*;

/// One event's replay input reduced to scalars we need.
struct EventInput {
    input: Vec<f64>,
    /// Time reference for the energy sampling position (see [`reference_sample`]).
    trigger: usize,
    /// D0 marker minus the reference, when the half height was found.
    marker_lag: Option<isize>,
    fw_energy: f64,
    /// D1 = Peaking window center (samples), if the probe carried any 1-bits.
    fw_peak_center: Option<usize>,
}

struct Opts {
    files: Vec<PathBuf>,
    channel: u8,
    rise_ns: f64,
    flat_ns: f64,
    pz_ns: f64,
    peak_pct: f64,
    peak_nsmean: usize,
    baseline_nsmean: usize,
    trigger_override: Option<usize>,
    peak_shift: isize,
    dump_trace: Option<PathBuf>,
    /// Validation mode: per-event `fw_energy,sw_energy,reference,marker_lag,pedestal` CSV.
    events_csv: Option<PathBuf>,
    // --- scan mode ---
    scan: bool,
    module: Option<u8>,
    probe: u8,
    scan_rise_ns: Vec<f64>,
    scan_flat_ns: Vec<f64>,
    fw_window: Option<(f64, f64)>,
    line_kev: Option<f64>,
    pz_auto: bool,
    scan_csv: Option<PathBuf>,
}

impl Default for Opts {
    fn default() -> Self {
        // es2_v1725_pha.json operating point.
        Self {
            files: Vec::new(),
            channel: 0,
            rise_ns: 5000.0,
            flat_ns: 1000.0,
            pz_ns: 50000.0,
            peak_pct: 80.0,
            peak_nsmean: 1,
            baseline_nsmean: 256,
            trigger_override: None,
            peak_shift: 0,
            dump_trace: None,
            events_csv: None,
            scan: false,
            module: None,
            probe: 1,
            scan_rise_ns: range_list("1000:8000:500").unwrap_or_default(),
            scan_flat_ns: range_list("500:2000:250").unwrap_or_default(),
            fw_window: None,
            line_kev: None,
            pz_auto: false,
            scan_csv: None,
        }
    }
}

/// Parse `lo:hi:step` (inclusive) or a comma-separated list into values.
fn range_list(text: &str) -> Result<Vec<f64>, String> {
    let num = |t: &str| t.trim().parse::<f64>().map_err(|e| format!("'{t}': {e}"));
    let values = if text.contains(':') {
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() != 3 {
            return Err(format!("'{text}': expected lo:hi:step"));
        }
        let (lo, hi, step) = (num(parts[0])?, num(parts[1])?, num(parts[2])?);
        if !(step > 0.0 && hi >= lo) {
            return Err(format!("'{text}': need step > 0 and hi ≥ lo"));
        }
        let n = ((hi - lo) / step + 1e-9).floor() as usize;
        (0..=n).map(|i| lo + i as f64 * step).collect()
    } else {
        text.split(',')
            .map(num)
            .collect::<Result<Vec<f64>, String>>()?
    };
    if values.is_empty() || values.iter().any(|v| v.is_nan() || *v <= 0.0) {
        return Err(format!("'{text}': values must be positive"));
    }
    Ok(values)
}

/// Parse `LO:HI` into an ordered pair.
fn window(text: &str) -> Result<(f64, f64), String> {
    let (lo, hi) = text
        .split_once(':')
        .ok_or_else(|| format!("'{text}': expected LO:HI"))?;
    let lo: f64 = lo.trim().parse().map_err(|e| format!("'{text}': {e}"))?;
    let hi: f64 = hi.trim().parse().map_err(|e| format!("'{text}': {e}"))?;
    if hi <= lo {
        return Err(format!("'{text}': need HI > LO"));
    }
    Ok((lo, hi))
}

fn print_usage(argv0: &str) {
    eprintln!(
        "Usage:\n  {0} <file.delila>... [--ch C]\n      \
         [--rise-ns N] [--flat-ns N] [--pz-ns N]\n      \
         [--peak-pct P] [--peak-nsmean N] [--baseline-nsmean N]\n      \
         [--trigger-sample S] [--dump-trace OUT.csv] [--events-csv OUT.csv]\n\n\
         Scan mode:\n  {0} <file.delila>... --scan [--ch C] [--module M] [--probe 1|2]\n      \
         [--scan-rise-ns lo:hi:step|a,b,c] [--scan-flat-ns lo:hi:step|a,b,c]\n      \
         [--fw-window LO:HI] [--line-kev E] [--pz-auto] [--scan-csv OUT.csv]\n\n\
         Defaults match es2 V1725 PHA (rise 5000 / flat 1000 / pz 50000 ns, peaking 80%).",
        argv0
    );
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 1;
    // Small helper: read the value that follows a flag.
    let val = |i: usize| -> Result<String, String> {
        args.get(i + 1)
            .cloned()
            .ok_or_else(|| format!("{} requires a value", args[i]))
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--ch" => {
                o.channel = val(i)?.parse().map_err(|e| format!("--ch: {e}"))?;
                i += 2;
            }
            "--rise-ns" => {
                o.rise_ns = val(i)?.parse().map_err(|e| format!("--rise-ns: {e}"))?;
                i += 2;
            }
            "--flat-ns" => {
                o.flat_ns = val(i)?.parse().map_err(|e| format!("--flat-ns: {e}"))?;
                i += 2;
            }
            "--pz-ns" => {
                o.pz_ns = val(i)?.parse().map_err(|e| format!("--pz-ns: {e}"))?;
                i += 2;
            }
            "--peak-pct" => {
                o.peak_pct = val(i)?.parse().map_err(|e| format!("--peak-pct: {e}"))?;
                i += 2;
            }
            "--peak-nsmean" => {
                o.peak_nsmean = val(i)?.parse().map_err(|e| format!("--peak-nsmean: {e}"))?;
                i += 2;
            }
            "--baseline-nsmean" => {
                o.baseline_nsmean = val(i)?
                    .parse()
                    .map_err(|e| format!("--baseline-nsmean: {e}"))?;
                i += 2;
            }
            "--trigger-sample" => {
                o.trigger_override = Some(
                    val(i)?
                        .parse()
                        .map_err(|e| format!("--trigger-sample: {e}"))?,
                );
                i += 2;
            }
            "--peak-shift" => {
                o.peak_shift = val(i)?.parse().map_err(|e| format!("--peak-shift: {e}"))?;
                i += 2;
            }
            "--dump-trace" => {
                o.dump_trace = Some(PathBuf::from(val(i)?));
                i += 2;
            }
            "--events-csv" => {
                o.events_csv = Some(PathBuf::from(val(i)?));
                i += 2;
            }
            "--scan" => {
                o.scan = true;
                i += 1;
            }
            "--pz-auto" => {
                o.pz_auto = true;
                i += 1;
            }
            "--module" => {
                o.module = Some(val(i)?.parse().map_err(|e| format!("--module: {e}"))?);
                i += 2;
            }
            "--probe" => {
                o.probe = val(i)?.parse().map_err(|e| format!("--probe: {e}"))?;
                if !(1..=2).contains(&o.probe) {
                    return Err("--probe must be 1 or 2".into());
                }
                i += 2;
            }
            "--scan-rise-ns" => {
                o.scan_rise_ns =
                    range_list(&val(i)?).map_err(|e| format!("--scan-rise-ns: {e}"))?;
                i += 2;
            }
            "--scan-flat-ns" => {
                o.scan_flat_ns =
                    range_list(&val(i)?).map_err(|e| format!("--scan-flat-ns: {e}"))?;
                i += 2;
            }
            "--fw-window" => {
                o.fw_window = Some(window(&val(i)?).map_err(|e| format!("--fw-window: {e}"))?);
                i += 2;
            }
            "--line-kev" => {
                o.line_kev = Some(val(i)?.parse().map_err(|e| format!("--line-kev: {e}"))?);
                i += 2;
            }
            "--scan-csv" => {
                o.scan_csv = Some(PathBuf::from(val(i)?));
                i += 2;
            }
            _ if a.starts_with("--") => return Err(format!("unknown flag: {a}")),
            _ => {
                o.files.push(PathBuf::from(a));
                i += 1;
            }
        }
    }
    if o.files.is_empty() {
        return Err("no input files".into());
    }
    Ok(o)
}

/// First rising edge (0→1) in a digital probe, i.e. the trigger sample for
/// `D0 = Trigger`. `None` if the probe is empty or never asserts.
fn first_rising_edge(probe: &[u8]) -> Option<usize> {
    probe.iter().position(|&b| b != 0)
}

/// Time reference for the energy sampling position: the half height of the
/// edge ([`trap::pulse_midpoint`]) — the D0 marker is recorded late with
/// respect to the input probe — falling back to the marker when no clear step
/// is found. Returns `(reference, marker − reference)`; the lag is `None` on
/// fallback. `--trigger-sample` bypasses this (see the callers).
fn reference_sample<T: Copy + Into<f64>>(samples: &[T], marker: usize) -> (usize, Option<isize>) {
    match trap::pulse_midpoint(samples, marker) {
        Some(mid) => (mid, Some(marker as isize - mid as isize)),
        None => (marker, None),
    }
}

/// One-line summary of how the time reference was obtained.
fn reference_summary(lags: &[isize], fallbacks: usize, ns_per_sample: f64) -> String {
    if lags.is_empty() {
        return format!("D0 trigger marker for all {fallbacks} events (no clear edge found)");
    }
    let mut sorted = lags.to_vec();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    format!(
        "edge half height for {} events (D0 marker is a median {median} samples = {:.0} ns later); \
         D0 marker for {fallbacks} without a clear edge",
        lags.len(),
        median as f64 * ns_per_sample
    )
}

/// Center of the asserted region of a digital probe (mean of the 1-bit indices),
/// used to locate the `D1 = Peaking` window. `None` if never asserted.
fn asserted_center(probe: &[u8]) -> Option<usize> {
    let idx: Vec<usize> = probe
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b != 0).then_some(i))
        .collect();
    if idx.is_empty() {
        None
    } else {
        Some(idx.iter().sum::<usize>() / idx.len())
    }
}

/// Reduce a waveform to the scalars the replay needs. Returns `None` if the
/// waveform has no analog samples.
fn event_input(
    wf: &Waveform,
    fw_energy: f64,
    trigger_override: Option<usize>,
) -> Option<EventInput> {
    if wf.analog_probe1.is_empty() {
        return None;
    }
    let input: Vec<f64> = wf.analog_probe1.iter().map(|&s| s as f64).collect();
    let (trigger, marker_lag) = match trigger_override {
        Some(t) => (t, None),
        None => reference_sample(&input, first_rising_edge(&wf.digital_probe1).unwrap_or(0)),
    };
    Some(EventInput {
        input,
        trigger,
        marker_lag,
        fw_energy,
        fw_peak_center: asserted_center(&wf.digital_probe2),
    })
}

/// Ordinary-least-squares fit `y = a·x + b`. Returns `(a, b)`.
fn linear_fit(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let mut cov = 0.0;
    let mut var = 0.0;
    for (&x, &y) in xs.iter().zip(ys) {
        cov += (x - mx) * (y - my);
        var += (x - mx) * (x - mx);
    }
    if var == 0.0 {
        (0.0, my)
    } else {
        let a = cov / var;
        (a, my - a * mx)
    }
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn std(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let m = mean(xs);
    let var = xs.iter().map(|&x| (x - m) * (x - m)).sum::<f64>() / (xs.len() as f64 - 1.0);
    var.sqrt()
}

/// Pearson correlation coefficient between two equal-length series. Returns `0.0`
/// if either series is constant (no variance to correlate).
fn pearson(xs: &[f64], ys: &[f64]) -> f64 {
    let (mx, my) = (mean(xs), mean(ys));
    let mut cov = 0.0;
    let mut vx = 0.0;
    let mut vy = 0.0;
    for (&x, &y) in xs.iter().zip(ys) {
        cov += (x - mx) * (y - my);
        vx += (x - mx) * (x - mx);
        vy += (y - my) * (y - my);
    }
    if vx == 0.0 || vy == 0.0 {
        0.0
    } else {
        cov / (vx.sqrt() * vy.sqrt())
    }
}

/// Compact ASCII histogram of integer-valued energies, printed to stdout.
fn print_energy_histogram(label: &str, values: &[f64]) {
    if values.is_empty() {
        return;
    }
    let lo = values.iter().cloned().fold(f64::INFINITY, f64::min).floor() as i64;
    let hi = values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil() as i64;
    let span = (hi - lo).max(1) as usize + 1;
    let mut bins = vec![0usize; span];
    for &v in values {
        let idx = ((v.round() as i64) - lo).clamp(0, span as i64 - 1) as usize;
        bins[idx] += 1;
    }
    let peak = bins.iter().cloned().max().unwrap_or(1).max(1);
    println!("--- {label} (N={}) ---", values.len());
    for (k, &c) in bins.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let bar = (c * 50 / peak).max(1);
        println!("  {:>7} | {:>7} {}", lo + k as i64, c, "#".repeat(bar));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opts = match parse_opts(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage(&args[0]);
            std::process::exit(2);
        }
    };

    let result = if opts.scan {
        run_scan(&opts)
    } else {
        run(&opts)
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(opts: &Opts) -> Result<(), Box<dyn std::error::Error>> {
    // Collect (SW energy, FW energy, peak-window comparison) across all files.
    let mut sw_energy = Vec::new();
    let mut fw_energy = Vec::new();
    let mut peak_deltas = Vec::new(); // computed peak_index - FW D1 center
    let mut ns_per_sample = 0.0f64;
    let mut params: Option<TrapParams> = None;
    let mut first_trace_dumped = false;
    let mut pretrigger_shortfall: Option<WindowError> = None;
    let (mut ref_lags, mut ref_fallbacks) = (Vec::<isize>::new(), 0usize);
    let mut event_rows: Vec<String> = Vec::new();

    for path in &opts.files {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut data_reader = DataFileReader::new(reader)?;

        for block in data_reader.data_blocks() {
            let batch = block?;
            // Reduce this batch to the target channel's replay inputs.
            let inputs: Vec<EventInput> = batch
                .events
                .iter()
                .filter(|e| e.channel == opts.channel && e.waveform.is_some())
                .filter_map(|e| {
                    let wf = e.waveform.as_ref().unwrap();
                    if ns_per_sample == 0.0 && wf.ns_per_sample > 0.0 {
                        ns_per_sample = wf.ns_per_sample;
                    }
                    event_input(wf, e.energy as f64, opts.trigger_override)
                })
                .collect();

            if inputs.is_empty() {
                continue;
            }

            // Build params once ns_per_sample is known.
            let p = *params.get_or_insert_with(|| {
                let ns = if ns_per_sample > 0.0 {
                    ns_per_sample
                } else {
                    4.0
                };
                let mut tp = TrapParams::from_ns(
                    opts.rise_ns,
                    opts.flat_ns,
                    opts.pz_ns,
                    opts.peak_pct,
                    opts.peak_nsmean,
                    opts.baseline_nsmean,
                    ns,
                );
                tp.peak_shift = opts.peak_shift;
                tp
            });

            // Optional: dump the first event's stage traces for probe overlay.
            if let Some(out) = &opts.dump_trace {
                if !first_trace_dumped {
                    dump_trace(out, &inputs[0], &p)?;
                    first_trace_dumped = true;
                    eprintln!("wrote stage-trace CSV to {}", out.display());
                }
            }

            if pretrigger_shortfall.is_none() {
                pretrigger_shortfall = inputs
                    .iter()
                    .find_map(|ev| p.check_window(ev.input.len(), ev.trigger).err());
            }
            for ev in &inputs {
                match ev.marker_lag {
                    Some(lag) => ref_lags.push(lag),
                    None => ref_fallbacks += 1,
                }
            }

            // Replay in parallel across the batch.
            let results: Vec<(f64, f64, Option<i64>, f64)> = inputs
                .par_iter()
                .map(|ev| {
                    let r = trap::analyze(&ev.input, ev.trigger, &p);
                    let dpeak = ev.fw_peak_center.map(|c| r.peak_index as i64 - c as i64);
                    (r.energy, ev.fw_energy, dpeak, r.pedestal)
                })
                .collect();

            for (ev, (sw, fw, dpeak, ped)) in inputs.iter().zip(results) {
                if opts.events_csv.is_some() {
                    event_rows.push(format!(
                        "{fw},{sw:.3},{},{},{ped:.2}",
                        ev.trigger,
                        ev.marker_lag.map(|l| l.to_string()).unwrap_or_default()
                    ));
                }
                sw_energy.push(sw);
                fw_energy.push(fw);
                if let Some(d) = dpeak {
                    peak_deltas.push(d as f64);
                }
            }
        }
    }

    if sw_energy.is_empty() {
        return Err(format!("no waveform events found for channel {}", opts.channel).into());
    }

    let p = params.unwrap();
    if let Some(short) = pretrigger_shortfall {
        eprintln!(
            "WARNING: {short} — the pedestal stands in for the missing baseline samples, so the SW \
             energy is noisier than the FW's. Record with pre-trigger ≥ {:.0} ns for these parameters.",
            p.pre_trigger_needed() as f64 * ns_per_sample
        );
    }
    if let Some(out) = &opts.events_csv {
        let mut text = String::from("fw_energy,sw_energy,reference,marker_lag,pedestal\n");
        for row in &event_rows {
            text.push_str(row);
            text.push('\n');
        }
        std::fs::write(out, text)?;
        eprintln!(
            "wrote {} per-event rows to {}",
            event_rows.len(),
            out.display()
        );
    }
    let reference = match opts.trigger_override {
        Some(t) => format!("fixed sample {t} (--trigger-sample)"),
        None => reference_summary(&ref_lags, ref_fallbacks, ns_per_sample),
    };
    report(
        opts,
        &p,
        ns_per_sample,
        &reference,
        &sw_energy,
        &fw_energy,
        &peak_deltas,
    );
    Ok(())
}

fn report(
    opts: &Opts,
    p: &TrapParams,
    ns_per_sample: f64,
    reference: &str,
    sw: &[f64],
    fw: &[f64],
    peak_deltas: &[f64],
) {
    const FWHM: f64 = 2.354_820_045; // Gaussian σ → FWHM

    println!("=== pha_trap_tune — Phase 1 validation ===");
    println!("channel        : {}", opts.channel);
    println!("ns_per_sample  : {ns_per_sample}");
    println!(
        "trap params    : rise={} flat={} samples, M={:.2}, peak={}% nsmean={} baseline_nsmean={}",
        p.rise, p.flat, p.pz_multiplier, opts.peak_pct, p.peak_nsmean, p.baseline_nsmean
    );
    println!("events         : {}", sw.len());
    println!("time reference : {reference}");
    println!();

    let fw_mean = mean(fw);
    let fw_std = std(fw);
    let sw_std = std(sw);
    let fw_range = fw.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - fw.iter().cloned().fold(f64::INFINITY, f64::min);
    let r = pearson(sw, fw);

    // Relative spread (σ/|mean|) is the scale-free quantity that lets us compare
    // the trap-unit SW energy against the LSB-unit FW energy on equal footing.
    let sw_mean = mean(sw);
    let fw_rel = if fw_mean != 0.0 {
        fw_std / fw_mean.abs()
    } else {
        0.0
    };
    let sw_rel = if sw_mean != 0.0 {
        sw_std / sw_mean.abs()
    } else {
        0.0
    };

    println!(
        "FW energy      : mean={fw_mean:.3}  σ={fw_std:.4} LSB  (FWHM={:.4})  σ/mean={:.0} ppm  range={fw_range:.0} LSB",
        fw_std * FWHM,
        fw_rel * 1e6
    );
    println!(
        "SW energy      : mean={sw_mean:.2}  σ/mean={:.0} ppm  [input ADC counts]",
        sw_rel * 1e6
    );
    println!(
        "rel-spread SW/FW: {:.2}x   (>1 = SW noisier than FW at these params)",
        if fw_rel > 0.0 { sw_rel / fw_rel } else { 0.0 }
    );
    println!("corr(SW,FW)    : r={r:.4}  (r²={:.4})", r * r);
    println!();

    // The per-event residual criterion (§4.2) requires an SW→FW *gain* calibration,
    // which needs energy DIVERSITY: SW must track FW across a real range of
    // amplitudes. A single-amplitude source (pulser) — or a single isolated
    // photopeak — has no such range, and a Gaussian's order statistics span
    // ~8–10σ regardless, so range/σ is not a usable discriminator. Use r²: only a
    // high r² means the linear fit actually explains the FW variance.
    let calibratable = r * r > 0.9;

    if calibratable {
        let (a, b) = linear_fit(sw, fw);
        let residuals: Vec<f64> = sw.iter().zip(fw).map(|(&s, &f)| f - (a * s + b)).collect();
        let res_std = std(&residuals);
        println!("linear calib   : FW ≈ {a:.6}·SW + {b:.3}");
        println!(
            "per-event resid: σ={res_std:.4} LSB   <-- Phase 1 criterion: ≪ peak FWHM, ideally ±1 LSB"
        );
        let verdict = if res_std <= 1.0 {
            "PASS (±1 LSB)"
        } else if res_std < fw_std {
            "MARGINAL (residual < FW spread but > 1 LSB — inspect stages)"
        } else {
            "FAIL (residual ≥ FW spread — SW model diverges, use --dump-trace)"
        };
        println!("verdict        : {verdict}");
    } else {
        println!("gain/linearity : NOT VALIDATED — single-amplitude data (FW range {fw_range:.0} LSB ≈ noise).");
        println!("                 A slope needs an energy spread; use a γ source (or dual-trace probe2).");
        println!("                 Pulser data validates the stage-4 anchor (below) + that the SW");
        println!(
            "                 trap produces a stable energy (σ_SW={sw_std:.3}). Low r is EXPECTED"
        );
        println!("                 here: the ±1 LSB FW spread is FW-internal (fixed-point) noise");
        println!("                 that a smooth f64 SW filter does not co-vary with — not a gain error.");
    }
    println!();

    // Peaking-window comparison (stage 4 anchor vs FW D1=Peaking probe).
    if !peak_deltas.is_empty() {
        println!(
            "peak-window Δ  : computed − FW(D1) = {:.2} ± {:.2} samples  (N={})",
            mean(peak_deltas),
            std(peak_deltas),
            peak_deltas.len()
        );
        println!(
            "                 (a constant offset = FW pipeline latency; feed it back into --peak-pct or a fixed shift)"
        );
        println!();
    } else {
        println!("peak-window Δ  : (no D1=Peaking probe in data — skip stage-4 anchor check)\n");
    }

    print_energy_histogram("FW energy", fw);
}

/// Dump one event's input + trapezoid trace to CSV for a probe-overlay plot.
fn dump_trace(
    path: &PathBuf,
    ev: &EventInput,
    p: &TrapParams,
) -> Result<(), Box<dyn std::error::Error>> {
    let r = trap::analyze_with_trace(&ev.input, ev.trigger, p);
    let trace = r.trap.unwrap_or_default();
    let mut w = BufWriter::new(File::create(path)?);
    writeln!(
        w,
        "# trigger={} pedestal={:.3} peak_index={} energy_adc={:.3}",
        ev.trigger, r.pedestal, r.peak_index, r.energy
    )?;
    writeln!(w, "sample_idx,input_adc,trapezoid")?;
    for (i, (&inp, &tr)) in ev.input.iter().zip(&trace).enumerate() {
        writeln!(w, "{i},{inp},{tr:.4}")?;
    }
    Ok(())
}

// ───────────────────────────── scan mode ─────────────────────────────

/// Visit every waveform event of the selected module/channel in all input files.
fn for_each_event(
    opts: &Opts,
    mut visit: impl FnMut(&delila_rs::common::EventData, &Waveform),
) -> Result<(), Box<dyn std::error::Error>> {
    for path in &opts.files {
        let mut reader = DataFileReader::new(BufReader::new(File::open(path)?))?;
        for block in reader.data_blocks() {
            for e in &block?.events {
                if e.channel != opts.channel || opts.module.is_some_and(|m| m != e.module) {
                    continue;
                }
                if let Some(wf) = &e.waveform {
                    visit(e, wf);
                }
            }
        }
    }
    Ok(())
}

/// Lowest FW code considered a γ line. Noise triggers crowd the bottom codes
/// (SN01 run 22 ch0: ~93 k events at E ≈ 1..5 after the threshold was lowered).
const AUTO_MIN_CODE: usize = 64;

/// The `n` tallest FW codes in `AUTO_MIN_CODE..0x7FFF` (the top codes are
/// overflow / saturation markers), each at least 2 % away from a taller one —
/// i.e. distinct lines, tallest first. Which line is which is the caller's
/// call: two ⁶⁰Co lines can come out in either order.
fn fw_lines(energies: &[u16], n: usize) -> Vec<usize> {
    let mut hist = vec![0u32; 1 << 16];
    for &e in energies {
        hist[e as usize] += 1;
    }
    let mut codes: Vec<usize> = (AUTO_MIN_CODE..0x7FFF).filter(|&e| hist[e] > 0).collect();
    codes.sort_by_key(|&e| std::cmp::Reverse(hist[e]));
    let mut lines: Vec<usize> = Vec::new();
    for e in codes {
        if lines.len() == n {
            break;
        }
        if lines
            .iter()
            .all(|&l| (e as f64 - l as f64).abs() > 0.02 * l as f64)
        {
            lines.push(e);
        }
    }
    lines
}

/// FW-energy window around the tallest line of the spectrum: mode ± 1 %
/// (at least ± 8 LSB). `None` when there is no line.
fn auto_fw_window(energies: &[u16]) -> Option<(f64, f64)> {
    let mode = *fw_lines(energies, 1).first()? as f64;
    let half = (mode * 0.01).max(8.0);
    Some((mode - half, mode + half))
}

fn run_scan(opts: &Opts) -> Result<(), Box<dyn std::error::Error>> {
    // Pass 1 (only when the window is not given): the FW spectrum picks the line.
    let (lo, hi) = match opts.fw_window {
        Some(w) => w,
        None => {
            let mut energies = Vec::new();
            for_each_event(opts, |e, _| energies.push(e.energy))?;
            let w = auto_fw_window(&energies)
                .ok_or_else(|| format!("no waveform events found for channel {}", opts.channel))?;
            let lines: Vec<String> = fw_lines(&energies, 3)
                .iter()
                .map(|l| l.to_string())
                .collect();
            println!(
                "FW window      : auto-selected tallest FW line, {:.0}..{:.0} LSB (lines, tallest first: {}; pass --fw-window to choose)",
                w.0,
                w.1,
                lines.join(", ")
            );
            if let Some(kev) = opts.line_kev {
                eprintln!(
                    "WARNING: --line-kev {kev} assumes the auto-selected line ({:.0} LSB) IS the {kev} keV line. \
                     The tallest line is not necessarily that one (⁶⁰Co 1173 keV is often taller than 1332 keV) — \
                     pick it with --fw-window LO:HI.",
                    0.5 * (w.0 + w.1)
                );
            }
            w
        }
    };

    // Pass 2: keep the waveforms of the events in the window.
    let mut events: Vec<ScanEvent> = Vec::new();
    let mut fw_energy: Vec<f64> = Vec::new();
    let (mut seen, mut no_trigger, mut ns_per_sample) = (0usize, 0usize, 0.0f64);
    let (mut ref_lags, mut ref_fallbacks) = (Vec::<isize>::new(), 0usize);
    for_each_event(opts, |e, wf| {
        seen += 1;
        let energy = e.energy as f64;
        if energy < lo || energy > hi {
            return;
        }
        let samples = if opts.probe == 2 {
            &wf.analog_probe2
        } else {
            &wf.analog_probe1
        };
        if samples.is_empty() {
            return;
        }
        let trigger = match opts.trigger_override {
            Some(t) => t,
            None => {
                let Some(marker) = first_rising_edge(&wf.digital_probe1) else {
                    no_trigger += 1;
                    return;
                };
                let (reference, lag) = reference_sample(samples, marker);
                match lag {
                    Some(lag) => ref_lags.push(lag),
                    None => ref_fallbacks += 1,
                }
                reference
            }
        };
        if ns_per_sample == 0.0 && wf.ns_per_sample > 0.0 {
            ns_per_sample = wf.ns_per_sample;
        }
        events.push(ScanEvent {
            samples: samples.clone(),
            trigger,
        });
        fw_energy.push(energy);
    })?;
    if ns_per_sample == 0.0 {
        ns_per_sample = 4.0;
        eprintln!("WARNING: waveforms carry no sample period — assuming 4 ns/sample");
    }
    if no_trigger > 0 {
        eprintln!(
            "WARNING: {no_trigger} events in the window have no D0=Trigger bit and were skipped \
             (pass --trigger-sample to fix the trigger position)"
        );
    }
    if events.is_empty() {
        return Err(format!(
            "no events of channel {} with FW energy in {lo:.0}..{hi:.0} ({seen} waveform events seen)",
            opts.channel
        )
        .into());
    }

    let ns = ns_per_sample;
    let mut triggers: Vec<usize> = events.iter().map(|e| e.trigger).collect();
    triggers.sort_unstable();
    let trigger = triggers[triggers.len() / 2];
    let len = events[0].samples.len();

    println!("=== pha_trap_tune — Phase 2 scan ===");
    println!(
        "channel        : {}{}",
        opts.channel,
        opts.module
            .map(|m| format!(" (module {m})"))
            .unwrap_or_default()
    );
    println!(
        "events         : {} in the FW window {lo:.0}..{hi:.0} LSB (of {seen} waveform events)",
        events.len()
    );
    println!(
        "record         : {len} samples @ {ns} ns = {:.1} µs, reference at sample {trigger} → pre-trigger {:.0} ns, post {:.0} ns",
        len as f64 * ns / 1000.0,
        trigger as f64 * ns,
        (len - trigger.min(len)) as f64 * ns
    );
    println!(
        "time reference : {}",
        match opts.trigger_override {
            Some(t) => format!("fixed sample {t} (--trigger-sample)"),
            None => reference_summary(&ref_lags, ref_fallbacks, ns),
        }
    );

    // Pole-zero: measured from the pulses, or as given.
    let pz_ns = if opts.pz_auto {
        let tau = scan::measure_decay_tau(&events).ok_or(
            "--pz-auto: could not measure the decay constant (tail too short or not decaying)",
        )?;
        println!(
            "pole-zero      : measured decay τ = {:.0} ns (FW setting given: {:.0} ns)",
            tau * ns,
            opts.pz_ns
        );
        tau * ns
    } else {
        println!(
            "pole-zero      : τ = {:.0} ns (fixed; --pz-auto measures it from the pulses)",
            opts.pz_ns
        );
        opts.pz_ns
    };

    let mut base = TrapParams::from_ns(
        opts.rise_ns,
        opts.flat_ns,
        pz_ns,
        opts.peak_pct,
        opts.peak_nsmean,
        opts.baseline_nsmean,
        ns,
    );
    base.peak_shift = opts.peak_shift;
    println!(
        "peaking        : {} % of the flat top, {} sample mean, shift {}",
        opts.peak_pct, base.peak_nsmean, base.peak_shift
    );

    let to_samples = |list: &[f64]| {
        let mut v: Vec<usize> = list
            .iter()
            .map(|x| (x / ns).round().max(1.0) as usize)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (rises, flats) = (
        to_samples(&opts.scan_rise_ns),
        to_samples(&opts.scan_flat_ns),
    );
    let report = scan::scan(&events, &scan::grid(&base, &rises, &flats));
    println!(
        "pile-up cut    : {} of {} events excluded (pre-trigger baseline on the tail of an earlier pulse)",
        report.events_tilt_rejected, report.events_total
    );

    // Reference: the FW's own resolution on the same selection.
    let kev = |rel: f64| opts.line_kev.map(|e| rel * e);
    let show = |rel: f64| match kev(rel) {
        Some(k) => format!("{k:.3} keV"),
        None => format!("{:.3} ‰", rel * 1e3),
    };
    match fit_peak(&fw_energy) {
        Some(f) => println!(
            "FW as recorded : FWHM {} ({:.2} LSB at {:.1}, LSB-quantized)",
            show(f.rel_fwhm()),
            f.fwhm,
            f.centroid
        ),
        None => {
            println!("FW as recorded : (FW energies in the window do not form a fittable peak)")
        }
    }
    println!();

    // Tables: rows = rise, columns = flat top.
    let best = report.best().map(|p| (p.params.rise, p.params.flat));
    let header = || {
        print!("{:>9}", "");
        for &f in &flats {
            print!("{:>10.0}", f as f64 * ns);
        }
        println!();
    };
    println!(
        "FWHM [{}]  rows: rise (ns), columns: flat top (ns)   * = best, ! = loses events to a tail (not eligible)",
        if opts.line_kev.is_some() { "keV" } else { "‰ of peak position" }
    );
    header();
    let mut skipped: Vec<String> = Vec::new();
    for &r in &rises {
        print!("{:>9.0}", r as f64 * ns);
        for &f in &flats {
            let point = report
                .points
                .iter()
                .find(|p| p.params.rise == r && p.params.flat == f);
            let cell = match point.map(|p| (p, p.outcome)) {
                Some((p, PointOutcome::Fit(fit))) => {
                    let v = kev(fit.rel_fwhm()).unwrap_or(fit.rel_fwhm() * 1e3);
                    let mark = if best == Some((r, f)) {
                        "*"
                    } else if !report.keeps_peak(p) {
                        "!"
                    } else {
                        " "
                    };
                    format!("{v:.3}{mark}")
                }
                Some((_, PointOutcome::Window(err))) => {
                    let (what, need) = match err {
                        WindowError::PreTriggerTooShort { need, .. } => ("pre-trigger", need),
                        WindowError::PostTriggerTooShort { need, .. } => ("post-trigger", need),
                    };
                    skipped.push(format!(
                        "rise {:.0} / flat {:.0} ns: needs {what} ≥ {:.0} ns",
                        r as f64 * ns,
                        f as f64 * ns,
                        need as f64 * ns
                    ));
                    "window ".to_string()
                }
                Some((_, PointOutcome::NoPeak)) | None => "no-peak ".to_string(),
            };
            print!("{cell:>10}");
        }
        println!();
    }
    println!();
    let window = report
        .content_half_width
        .map(|w| {
            format!(
                "±{} (= {}× the narrowest FWHM)",
                show(w),
                scan::CONTENT_WINDOW_FWHMS
            )
        })
        .unwrap_or_else(|| "(no fitted point)".to_string());
    println!(
        "events within {window} of each centroid [% of the replayed events]  (a drop = ballistic deficit / tailing)"
    );
    header();
    for &r in &rises {
        print!("{:>9.0}", r as f64 * ns);
        for &f in &flats {
            let frac = report
                .points
                .iter()
                .find(|p| p.params.rise == r && p.params.flat == f)
                .and_then(|p| p.peak_fraction());
            match frac {
                Some(x) => print!("{:>9.1} ", x * 100.0),
                None => print!("{:>9} ", "-"),
            }
        }
        println!();
    }
    println!();
    match report.best() {
        Some(p) => {
            let rel = p.rel_fwhm().unwrap_or(f64::NAN);
            println!(
                "best           : rise {:.0} ns, flat top {:.0} ns → FWHM {}",
                p.params.rise as f64 * ns,
                p.params.flat as f64 * ns,
                show(rel)
            );
            let edge = |v: usize, list: &[usize]| {
                list.len() > 1 && (Some(&v) == list.first() || Some(&v) == list.last())
            };
            if edge(p.params.rise, &rises) || edge(p.params.flat, &flats) {
                println!("                 NOTE: the best point is on the edge of the grid — extend the scan range.");
            }
        }
        None => println!("best           : (no grid point produced a fittable peak)"),
    }
    if !skipped.is_empty() {
        println!(
            "\n{} grid point(s) NOT evaluated — the record does not contain the samples they need:",
            skipped.len()
        );
        for line in skipped.iter().take(6) {
            println!("  {line}");
        }
        if skipped.len() > 6 {
            println!("  … and {} more", skipped.len() - 6);
        }
    }

    if let Some(path) = &opts.scan_csv {
        let mut w = BufWriter::new(File::create(path)?);
        writeln!(
            w,
            "rise_ns,flat_ns,status,centroid_adc,fwhm_adc,rel_fwhm,fwhm_kev,peak_fraction,eligible"
        )?;
        for p in &report.points {
            let (rise, flat) = (p.params.rise as f64 * ns, p.params.flat as f64 * ns);
            match p.outcome {
                PointOutcome::Fit(f) => writeln!(
                    w,
                    "{rise},{flat},ok,{:.4},{:.5},{:.6e},{},{:.4},{}",
                    f.centroid,
                    f.fwhm,
                    f.rel_fwhm(),
                    kev(f.rel_fwhm())
                        .map(|k| format!("{k:.4}"))
                        .unwrap_or_default(),
                    p.peak_fraction().unwrap_or(0.0),
                    report.keeps_peak(p)
                )?,
                PointOutcome::Window(e) => {
                    // Comma-free status so the file stays a plain CSV.
                    let status = match e {
                        WindowError::PreTriggerTooShort { need, have } => {
                            format!("pre-trigger-too-short need={need} have={have} samples")
                        }
                        WindowError::PostTriggerTooShort { need, have } => {
                            format!("post-trigger-too-short need={need} have={have} samples")
                        }
                    };
                    writeln!(w, "{rise},{flat},{status},,,,,,")?
                }
                PointOutcome::NoPeak => writeln!(w, "{rise},{flat},no-peak,,,,,,")?,
            }
        }
        eprintln!("wrote scan CSV to {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use delila_rs::common::{EventData, EventDataBatch};
    use delila_rs::recorder::{ChecksumCalculator, FileFooter, FileHeader};
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use rand_distr::{Distribution, Normal};

    #[test]
    fn range_list_accepts_ranges_and_lists() {
        assert_eq!(
            range_list("1000:2000:500").unwrap(),
            vec![1000.0, 1500.0, 2000.0]
        );
        assert_eq!(
            range_list("1000:2200:500").unwrap(),
            vec![1000.0, 1500.0, 2000.0]
        );
        assert_eq!(
            range_list("250, 1000,4000").unwrap(),
            vec![250.0, 1000.0, 4000.0]
        );
        assert!(range_list("1000:500:100").is_err());
        assert!(range_list("1000:2000").is_err());
        assert!(range_list("0,100").is_err());
        assert!(range_list("abc").is_err());
    }

    #[test]
    fn scan_flags_are_parsed() {
        let args: Vec<String> = "pha_trap_tune run.delila --scan --ch 3 --module 2 --probe 2 \
            --scan-rise-ns 1000,2000 --scan-flat-ns 500:1000:250 --fw-window 3960:4040 \
            --line-kev 1332.5 --pz-auto --scan-csv out.csv"
            .split_whitespace()
            .map(String::from)
            .collect();
        let o = parse_opts(&args).unwrap();
        assert!(o.scan && o.pz_auto);
        assert_eq!((o.channel, o.module, o.probe), (3, Some(2), 2));
        assert_eq!(o.scan_rise_ns, vec![1000.0, 2000.0]);
        assert_eq!(o.scan_flat_ns, vec![500.0, 750.0, 1000.0]);
        assert_eq!(o.fw_window, Some((3960.0, 4040.0)));
        assert_eq!(o.line_kev, Some(1332.5));
        assert!(parse_opts(&["x".into(), "f".into(), "--fw-window".into(), "5:1".into()]).is_err());
    }

    #[test]
    fn auto_window_brackets_the_tallest_fw_peak() {
        let mut energies = vec![0u16; 500]; // energy 0 = no energy, must be ignored
        energies.extend(std::iter::repeat_n(4000u16, 300));
        energies.extend(std::iter::repeat_n(3520u16, 100));
        assert_eq!(auto_fw_window(&energies), Some((3960.0, 4040.0)));
        assert_eq!(auto_fw_window(&[0, 0, 0]), None);
    }

    #[test]
    fn auto_window_ignores_noise_triggers_at_the_lowest_codes() {
        // SN01 run 22 ch0: a lowered threshold put ~93 k noise triggers at
        // FW E ≈ 1..5, far more than any γ line.
        let mut energies: Vec<u16> = (0..90_000).map(|i| 1 + (i % 5) as u16).collect();
        energies.extend(std::iter::repeat_n(7870u16, 400));
        energies.extend(std::iter::repeat_n(6929u16, 450));
        let (lo, hi) = auto_fw_window(&energies).expect("a line");
        assert!(
            (lo - 6859.71).abs() < 1e-6 && (hi - 6998.29).abs() < 1e-6,
            "{lo}..{hi}"
        );
        // Both ⁶⁰Co lines are listed, tallest first — the caller must say which is which.
        assert_eq!(fw_lines(&energies, 3), vec![6929, 7870]);
    }

    // ---- end-to-end: synthetic run file → run_scan → CSV ----

    const NS: f64 = 4.0;
    const LEN: usize = 4200;
    const PRE: usize = 2044; // the DPP-PHA maximum: 511 × 4 samples (UM5678 §1.5)
    const TAU: f64 = 12_500.0; // 50 µs

    /// Write a complete `.delila` file (same layout as tests/file_format_test.rs).
    fn write_delila(path: &std::path::Path, batches: &[EventDataBatch]) {
        let mut buf = Vec::new();
        FileHeader::new(1, "scan_test".to_string(), 0)
            .write_to(&mut buf)
            .unwrap();
        let mut checksum = ChecksumCalculator::new();
        let mut footer = FileFooter::new();
        for batch in batches {
            let data = batch.to_msgpack().unwrap();
            let len = (data.len() as u32).to_le_bytes();
            buf.extend_from_slice(&len);
            buf.extend_from_slice(&data);
            checksum.update(&len);
            checksum.update(&data);
            footer.total_events += batch.events.len() as u64;
        }
        footer.data_checksum = checksum.finalize();
        footer.data_bytes = checksum.bytes_processed();
        footer.finalize();
        footer.write_to(&mut buf).unwrap();
        std::fs::write(path, buf).unwrap();
    }

    /// One PHA-like event on channel 3: exponential pulse of amplitude `amp`
    /// collected over `collect` samples, white noise 2 ADC counts, D0 = Trigger
    /// at the pulse start, FW energy = 2 × amplitude ± 1.5 LSB.
    fn event(rng: &mut StdRng, amp: f64, collect: usize) -> EventData {
        let noise = Normal::new(0.0, 2.0).unwrap();
        let decay = (-1.0 / TAU).exp();
        let mut v = 0.0f64;
        let samples: Vec<i16> = (0..LEN)
            .map(|n| {
                v *= decay;
                if n >= PRE && n < PRE + collect {
                    v += amp / collect as f64;
                }
                (-8104.0 + v + noise.sample(rng)).round() as i16
            })
            .collect();
        let mut trigger_bit = vec![0u8; LEN];
        trigger_bit[PRE..PRE + 8].fill(1);
        let waveform = Waveform {
            analog_probe1: samples,
            digital_probe1: trigger_bit,
            ns_per_sample: NS,
            ..Default::default()
        };
        let fw = (2.0 * amp + Normal::new(0.0, 1.5).unwrap().sample(rng)).round() as u16;
        EventData::with_waveform(0, 3, fw, 0, 0.0, 0, waveform)
    }

    /// 900 events of a "1332 keV" line (amplitude 2000) and 250 of a second line.
    fn synthetic_run(path: &std::path::Path) {
        let mut rng = StdRng::seed_from_u64(59);
        let mut batch = EventDataBatch::new(0, 0);
        for i in 0..1150 {
            let amp = if i % 23 < 18 { 2000.0 } else { 1760.0 };
            let collect = rng.gen_range(10..=80); // 40–320 ns charge collection
            batch.push(event(&mut rng, amp, collect));
        }
        write_delila(path, &[batch]);
    }

    fn scan_opts(file: &std::path::Path, csv: &std::path::Path, rise: &str, flat: &str) -> Opts {
        Opts {
            files: vec![file.to_path_buf()],
            channel: 3,
            scan: true,
            pz_auto: true,
            line_kev: Some(1332.5),
            scan_rise_ns: range_list(rise).unwrap(),
            scan_flat_ns: range_list(flat).unwrap(),
            scan_csv: Some(csv.to_path_buf()),
            ..Opts::default()
        }
    }

    struct Row {
        rise: f64,
        flat: f64,
        status: String,
        fwhm_kev: Option<f64>,
        peak_fraction: Option<f64>,
        eligible: bool,
    }

    fn read_csv(path: &std::path::Path) -> Vec<Row> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| {
                let f: Vec<&str> = line.split(',').collect();
                Row {
                    rise: f[0].parse().unwrap(),
                    flat: f[1].parse().unwrap(),
                    status: f[2].to_string(),
                    fwhm_kev: f[6].parse().ok(),
                    peak_fraction: f[7].parse().ok(),
                    eligible: f[8] == "true",
                }
            })
            .collect()
    }

    #[test]
    fn scan_ranks_the_grid_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let (file, csv) = (dir.path().join("run.delila"), dir.path().join("scan.csv"));
        synthetic_run(&file);

        run_scan(&scan_opts(&file, &csv, "1000,2000,4000,7000", "200,1000")).unwrap();
        let rows = read_csv(&csv);
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|r| r.status == "ok"));

        // A 200 ns flat top is shorter than the 40–320 ns charge collection: those
        // points lose events to a ballistic-deficit tail and are not eligible,
        // however narrow their core.
        for r in &rows {
            let frac = r.peak_fraction.unwrap();
            if r.flat == 200.0 {
                assert!(frac < 0.85 && !r.eligible, "flat 200: fraction {frac}");
            } else {
                assert!(frac > 0.95 && r.eligible, "flat 1000: fraction {frac}");
            }
        }
        // Among the eligible points, white noise → the longest rise wins.
        let fwhm = |r: &Row| r.fwhm_kev.unwrap();
        let best = rows
            .iter()
            .filter(|r| r.eligible)
            .min_by(|a, b| fwhm(a).total_cmp(&fwhm(b)))
            .unwrap();
        assert_eq!((best.rise, best.flat), (7000.0, 1000.0));
        // Absolute scale: FWHM = 2.355·σ·√(2/k)/A·E = 0.106 keV for σ = 2, k = 1750.
        let want = 2.354_82 * 2.0 * (2.0f64 / 1750.0).sqrt() / 2000.0 * 1332.5;
        assert!(
            (fwhm(best) / want - 1.0).abs() < 0.2,
            "FWHM {} keV, expected ≈ {want}",
            fwhm(best)
        );
        // Rise 1000 → 4000 ns must improve by ≈ √4.
        let at = |rise: f64| {
            fwhm(
                rows.iter()
                    .find(|r| r.rise == rise && r.flat == 1000.0)
                    .unwrap(),
            )
        };
        assert!(
            (1.6..2.4).contains(&(at(1000.0) / at(4000.0))),
            "{} vs {}",
            at(1000.0),
            at(4000.0)
        );
    }

    #[test]
    fn scan_reports_points_the_record_cannot_support() {
        let dir = tempfile::tempdir().unwrap();
        let (file, csv) = (dir.path().join("run.delila"), dir.path().join("scan.csv"));
        synthetic_run(&file);

        // Rise 9000 ns needs 2250 + 50 − 1 samples of pre-trigger; the record has 2044.
        run_scan(&scan_opts(&file, &csv, "4000,9000", "1000")).unwrap();
        let rows = read_csv(&csv);
        assert_eq!(rows[0].status, "ok");
        assert_eq!(
            rows[1].status,
            "pre-trigger-too-short need=2299 have=2044 samples"
        );
        assert!(rows[1].fwhm_kev.is_none() && !rows[1].eligible);
    }
}

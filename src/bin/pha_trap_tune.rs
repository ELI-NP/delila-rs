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

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;

use delila_rs::common::Waveform;
use delila_rs::offline::trap::{self, TrapParams};
use delila_rs::recorder::DataFileReader;
use rayon::prelude::*;

/// One event's replay input reduced to scalars we need.
struct EventInput {
    input: Vec<f64>,
    trigger: usize,
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
        }
    }
}

fn print_usage(argv0: &str) {
    eprintln!(
        "Usage:\n  {0} <file.delila>... [--ch C]\n      \
         [--rise-ns N] [--flat-ns N] [--pz-ns N]\n      \
         [--peak-pct P] [--peak-nsmean N] [--baseline-nsmean N]\n      \
         [--trigger-sample S] [--dump-trace OUT.csv]\n\n\
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
    let trigger = trigger_override
        .or_else(|| first_rising_edge(&wf.digital_probe1))
        .unwrap_or(0);
    let input: Vec<f64> = wf.analog_probe1.iter().map(|&s| s as f64).collect();
    Some(EventInput {
        input,
        trigger,
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

    if let Err(e) = run(&opts) {
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

            // Replay in parallel across the batch.
            let results: Vec<(f64, f64, Option<i64>)> = inputs
                .par_iter()
                .map(|ev| {
                    let r = trap::analyze(&ev.input, ev.trigger, &p);
                    let dpeak = ev.fw_peak_center.map(|c| r.peak_index as i64 - c as i64);
                    (r.energy, ev.fw_energy, dpeak)
                })
                .collect();

            for (sw, fw, dpeak) in results {
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
    report(
        opts,
        &p,
        ns_per_sample,
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
        "SW energy      : mean={sw_mean:.4e}  σ/mean={:.0} ppm  [trap units]",
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
        println!(
            "                 here: the ±1 LSB FW spread is FW-internal (fixed-point / BLR) noise"
        );
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
        "# trigger={} baseline={:.3} peak_index={} energy={:.3}",
        ev.trigger, r.baseline, r.peak_index, r.energy
    )?;
    writeln!(w, "sample_idx,input_adc,trapezoid")?;
    for (i, (&inp, &tr)) in ev.input.iter().zip(&trace).enumerate() {
        writeln!(w, "{i},{inp},{tr:.4}")?;
    }
    Ok(())
}

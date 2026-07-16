//! Software DPP-PHA trapezoidal filter (offline replay core).
//!
//! This is the from-scratch SW trapezoid for TODO 59 (ELIADE PHA energy-resolution
//! auto-tune). It is **not** in the reader hot path — it runs offline over stored
//! `.delila` waveforms so any parameter set can be re-applied to the *same* events
//! in ~ms/eval. See `TODO/59_eliade_trap_autotune.md`.
//!
//! # Stage decomposition (§4.4)
//!
//! The filter is deliberately split into separable, inspectable stages so that
//! when Phase-1 validation against the FW diverges, we can localize *which* stage
//! is wrong instead of debugging a monolith:
//!
//! | Stage | Function                | Affects FWHM? |
//! |-------|-------------------------|:---:|
//! | 1 Input      | (raw `analog_probe1`) | — |
//! | 2 Baseline   | [`trap_baseline`]     | yes |
//! | 3 Trapezoid  | [`trapezoid_trace`]   | yes (shaping) |
//! | 4 Energy     | [`extract_energy`]    | yes (noise averaging) |
//! | 5 Gain       | (linear, ignored — resolution is relative) | — |
//!
//! # Math anchor (Jordanov-Knoll; UM4380)
//!
//! ```text
//! l = k + m                                  # k = rise, m = flat-top (SAMPLES)
//! d[n] = v[n] - v[n-k] - v[n-l] + v[n-k-l]
//! p[n] = p[n-1] + d[n]
//! r[n] = p[n] + M·d[n]                        # M = pole-zero multiplier
//! s[n] = s[n-1] + r[n]
//! energy = mean(s over the peaking window) - baseline
//! ```
//!
//! The difference filter rejects DC, so the raw ADC pedestal (e.g. −8104 in the
//! es2 pulser data) needs no pre-subtraction. The pole-zero term `M·d[n]`
//! compensates the preamp exponential decay so a matched `M` yields a flat top.

/// DPP-PHA trapezoid parameters, in **samples** (the caller converts from ns via
/// [`TrapParams::from_ns`]). Gain normalization (stage 5) is intentionally absent —
/// resolution work re-calibrates the peak each eval, so the absolute scale of the
/// trapezoid output does not matter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrapParams {
    /// `k` — trapezoid rise time, in samples.
    pub rise: usize,
    /// `m` — trapezoid flat-top width, in samples.
    pub flat: usize,
    /// `M` — pole-zero multiplier (`r[n] = p[n] + M·d[n]`). For a preamp decay
    /// constant `τ` (in samples) the matched value is `1/(exp(1/τ) − 1) ≈ τ`.
    pub pz_multiplier: f64,
    /// Peaking position within the flat top, as a percentage `[0, 100]`. The
    /// energy is sampled at `trigger + rise + peak_pct% · flat`.
    pub peak_pct: f64,
    /// Number of trapezoid samples averaged at the peaking position (≥ 1).
    /// Mirrors the FW `N Samples Peak` (PEAK_NSMEAN).
    pub peak_nsmean: usize,
    /// Number of pre-trigger trapezoid samples averaged for the baseline.
    /// Mirrors the FW `N Samples Baseline` (BLINE_NSMEAN).
    pub baseline_nsmean: usize,
    /// Extra samples added to the computed peaking index to absorb the FW's
    /// internal filter/pipeline latency (measured against the `D1 = Peaking`
    /// probe in Phase 1). `0` = naive `trigger + rise + peak_pct%·flat`.
    pub peak_shift: isize,
}

impl TrapParams {
    /// Build from nanosecond-domain FW parameters and the waveform sample period.
    ///
    /// `pz_ns` is the preamp decay constant τ (e.g. `trap_pole_zero_ns`). The
    /// pole-zero multiplier is derived as `M = 1/(exp(1/τ_samples) − 1)`, which is
    /// the discrete value giving an exactly flat top for a `exp(-n/τ)` input.
    pub fn from_ns(
        rise_ns: f64,
        flat_ns: f64,
        pz_ns: f64,
        peak_pct: f64,
        peak_nsmean: usize,
        baseline_nsmean: usize,
        ns_per_sample: f64,
    ) -> Self {
        let rise = (rise_ns / ns_per_sample).round().max(1.0) as usize;
        let flat = (flat_ns / ns_per_sample).round().max(1.0) as usize;
        let tau_samples = pz_ns / ns_per_sample;
        Self {
            rise,
            flat,
            pz_multiplier: pole_zero_multiplier(tau_samples),
            peak_pct,
            peak_nsmean: peak_nsmean.max(1),
            baseline_nsmean: baseline_nsmean.max(1),
            peak_shift: 0,
        }
    }

    /// `l = k + m`, the outer tap of the difference filter (samples).
    #[inline]
    pub fn span(&self) -> usize {
        self.rise + self.flat
    }
}

/// Matched pole-zero multiplier for a preamp decay constant `τ` (in samples):
/// `M = 1/(exp(1/τ) − 1)`. For large τ this is ≈ `τ − 0.5`. A non-finite or
/// non-positive τ disables the correction (`M = 0`), which is the pure
/// trapezoid (no droop compensation).
#[inline]
pub fn pole_zero_multiplier(tau_samples: f64) -> f64 {
    if tau_samples.is_finite() && tau_samples > 0.0 {
        1.0 / ((1.0 / tau_samples).exp() - 1.0)
    } else {
        0.0
    }
}

/// Stage 3 — the full Jordanov trapezoid recursion, returning `s[n]` for every
/// input sample.
///
/// Pre-history (`v[i]` for `i < 0`) is clamped to `input[0]`, treating the
/// samples before the record as a constant pedestal. Because the difference
/// filter rejects DC, this avoids an artificial edge at `n = 0` while keeping the
/// pole-zero accumulator well-defined.
pub fn trapezoid_trace(input: &[f64], p: &TrapParams) -> Vec<f64> {
    let n = input.len();
    let mut s = vec![0.0f64; n];
    if n == 0 {
        return s;
    }
    let k = p.rise as isize;
    let l = p.span() as isize;
    let kl = k + l;
    let first = input[0];
    let v = |i: isize| -> f64 {
        if i < 0 {
            first
        } else {
            input[i as usize]
        }
    };
    let mut p_acc = 0.0f64;
    let mut s_acc = 0.0f64;
    for (i, out) in s.iter_mut().enumerate() {
        let ii = i as isize;
        let d = v(ii) - v(ii - k) - v(ii - l) + v(ii - kl);
        p_acc += d;
        let r = p_acc + p.pz_multiplier * d;
        s_acc += r;
        *out = s_acc;
    }
    s
}

/// Stage 2 — baseline of the trapezoid, taken as the mean over a pre-trigger
/// window. The window ends `guard` samples before `trigger` (to stay clear of the
/// rising edge) and spans `baseline_nsmean` samples. Returns `0.0` when there is
/// no room before the trigger.
pub fn trap_baseline(trap: &[f64], trigger: usize, p: &TrapParams, guard: usize) -> f64 {
    let hi = trigger.saturating_sub(guard);
    let lo = hi.saturating_sub(p.baseline_nsmean);
    mean(&trap[lo..hi.min(trap.len())])
}

/// Stage 4 — extract the energy from the trapezoid.
///
/// Samples `s[n]` at the peaking position `trigger + rise + peak_pct%·flat`,
/// averages `peak_nsmean` samples centered there, and subtracts `baseline`.
/// Returns `(energy, peak_center_index)`.
pub fn extract_energy(trap: &[f64], trigger: usize, p: &TrapParams, baseline: f64) -> (f64, usize) {
    let peak_offset = p.rise as f64 + (p.peak_pct / 100.0) * p.flat as f64;
    let center = (trigger as isize + peak_offset.round() as isize + p.peak_shift).max(0) as usize;
    let half = p.peak_nsmean / 2;
    let lo = center.saturating_sub(half).min(trap.len());
    let hi = (lo + p.peak_nsmean).min(trap.len());
    (mean(&trap[lo..hi]) - baseline, center)
}

/// Result of a single-event trapezoid analysis.
#[derive(Debug, Clone)]
pub struct TrapResult {
    /// Extracted energy in trapezoid units (pre-gain; compare distributions/FWHM,
    /// or linearly calibrate to FW energy for the per-event residual check).
    pub energy: f64,
    /// Trapezoid baseline (stage 2) that was subtracted.
    pub baseline: f64,
    /// Sample index where the energy was evaluated (stage 4).
    pub peak_index: usize,
    /// Full `s[n]` trapezoid trace (stage 3), for per-stage inspection / overlay
    /// against the FW `analog_probe2`. `None` unless [`analyze_with_trace`] is used.
    pub trap: Option<Vec<f64>>,
}

/// Guard between the baseline window and the trigger, in samples. Keeps the
/// baseline mean clear of the trapezoid's rising edge.
const BASELINE_GUARD: usize = 8;

/// Full single-event analysis (stages 2–4), discarding the trapezoid trace.
/// Use this on the hot grid-scan path where the trace is not needed.
pub fn analyze(input: &[f64], trigger: usize, p: &TrapParams) -> TrapResult {
    let trap = trapezoid_trace(input, p);
    let baseline = trap_baseline(&trap, trigger, p, BASELINE_GUARD);
    let (energy, peak_index) = extract_energy(&trap, trigger, p, baseline);
    TrapResult {
        energy,
        baseline,
        peak_index,
        trap: None,
    }
}

/// Full single-event analysis (stages 2–4), keeping the trapezoid trace for
/// inspection (Phase-1 probe overlay / debugging).
pub fn analyze_with_trace(input: &[f64], trigger: usize, p: &TrapParams) -> TrapResult {
    let trap = trapezoid_trace(input, p);
    let baseline = trap_baseline(&trap, trigger, p, BASELINE_GUARD);
    let (energy, peak_index) = extract_energy(&trap, trigger, p, baseline);
    TrapResult {
        energy,
        baseline,
        peak_index,
        trap: Some(trap),
    }
}

/// Mean of a slice; `0.0` for an empty slice (avoids NaN on edge windows).
#[inline]
fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic preamp pulse: flat pedestal `base`, then a step at `t0` that
    /// decays exponentially with constant `tau` (samples). Positive `amp` = a
    /// positive-going pulse (es2 PHA polarity).
    fn synth_pulse(len: usize, t0: usize, tau: f64, amp: f64, base: f64) -> Vec<f64> {
        (0..len)
            .map(|n| {
                if n < t0 {
                    base
                } else {
                    base + amp * (-((n - t0) as f64) / tau).exp()
                }
            })
            .collect()
    }

    #[test]
    fn dc_input_yields_zero_trapezoid() {
        // A flat pedestal must produce ~0 everywhere: the difference filter
        // rejects DC regardless of the pedestal level.
        let input = vec![-8104.0; 2000];
        let p = TrapParams::from_ns(5000.0, 1000.0, 50000.0, 80.0, 1, 256, 4.0);
        let trap = trapezoid_trace(&input, &p);
        assert!(
            trap.iter().all(|&s| s.abs() < 1e-6),
            "DC input should give zero trapezoid, max = {}",
            trap.iter().cloned().fold(0.0f64, |a, b| a.max(b.abs()))
        );
    }

    #[test]
    fn matched_pole_zero_gives_flat_top() {
        // With M matched to the input decay, the flat-top region of the trapezoid
        // should be flat (small relative slope across the plateau).
        let tau = 3000.0;
        let ns = 4.0;
        let t0 = 300;
        let input = synth_pulse(6000, t0, tau, 2000.0, -8104.0);
        // rise 1000 samples, flat 250 samples, pz τ matched.
        let p = TrapParams::from_ns(
            1000.0 * ns, // rise_ns so rise=1000 samples
            250.0 * ns,  // flat_ns so flat=250 samples
            tau * ns,    // pz_ns so τ_samples = tau
            80.0,
            1,
            200,
            ns,
        );
        let trap = trapezoid_trace(&input, &p);
        // Flat-top window: [t0+rise, t0+rise+flat).
        let a = t0 + p.rise;
        let b = a + p.flat;
        let plateau = &trap[a..b];
        let top = mean(plateau);
        let max_dev = plateau
            .iter()
            .map(|&s| (s - top).abs())
            .fold(0.0f64, f64::max);
        // Deviation across the flat top should be a small fraction of its height.
        assert!(
            top > 0.0 && max_dev / top < 0.02,
            "flat top not flat: top={top}, max_dev={max_dev}, ratio={}",
            max_dev / top
        );
    }

    #[test]
    fn energy_scales_linearly_with_amplitude() {
        let tau = 3000.0;
        let ns = 4.0;
        let t0 = 300;
        let p = TrapParams::from_ns(1000.0 * ns, 250.0 * ns, tau * ns, 80.0, 1, 200, ns);
        let e1 = analyze(&synth_pulse(6000, t0, tau, 1000.0, -8104.0), t0, &p).energy;
        let e2 = analyze(&synth_pulse(6000, t0, tau, 2000.0, -8104.0), t0, &p).energy;
        // Double amplitude → double energy (to within a fraction of a percent).
        assert!(
            e1 > 0.0 && ((e2 / e1) - 2.0).abs() < 0.01,
            "energy not linear: e1={e1}, e2={e2}, ratio={}",
            e2 / e1
        );
    }

    #[test]
    fn pedestal_offset_does_not_change_energy() {
        // DC rejection at the energy level: shifting the whole waveform by a
        // constant must not change the extracted energy.
        let tau = 3000.0;
        let ns = 4.0;
        let t0 = 300;
        let p = TrapParams::from_ns(1000.0 * ns, 250.0 * ns, tau * ns, 80.0, 1, 200, ns);
        let e_lo = analyze(&synth_pulse(6000, t0, tau, 2000.0, -8104.0), t0, &p).energy;
        let e_hi = analyze(&synth_pulse(6000, t0, tau, 2000.0, 500.0), t0, &p).energy;
        assert!(
            (e_lo - e_hi).abs() / e_lo < 1e-3,
            "pedestal changed energy: {e_lo} vs {e_hi}"
        );
    }

    #[test]
    fn matched_pole_zero_flatter_than_mismatched() {
        // Sanity for stage-2/3 diagnostics: a matched M gives a flatter top than a
        // badly mismatched M (droop). Measures plateau slope.
        let tau = 3000.0;
        let ns = 4.0;
        let t0 = 300;
        let input = synth_pulse(6000, t0, tau, 2000.0, -8104.0);
        let slope = |pz_tau: f64| {
            let p = TrapParams::from_ns(1000.0 * ns, 250.0 * ns, pz_tau * ns, 80.0, 1, 200, ns);
            let trap = trapezoid_trace(&input, &p);
            let a = t0 + p.rise;
            let b = a + p.flat;
            (trap[b - 1] - trap[a]).abs()
        };
        let matched = slope(tau);
        let mismatched = slope(tau * 0.3); // way off
        assert!(
            matched < mismatched,
            "matched slope {matched} should be < mismatched {mismatched}"
        );
    }

    #[test]
    fn from_ns_converts_and_matches_es2_config() {
        // es2_v1725_pha.json: rise 5000, flat 1000, pz 50000 ns @ 4 ns/sample.
        let p = TrapParams::from_ns(5000.0, 1000.0, 50000.0, 80.0, 1, 256, 4.0);
        assert_eq!(p.rise, 1250);
        assert_eq!(p.flat, 250);
        assert_eq!(p.peak_shift, 0);
        // τ = 12500 samples → M ≈ 12499.5.
        assert!(
            (p.pz_multiplier - 12499.5).abs() < 1.0,
            "M = {}",
            p.pz_multiplier
        );
    }

    #[test]
    fn peak_shift_moves_the_energy_sample() {
        // A non-zero peak_shift must move the evaluated peak index by exactly that
        // many samples (this is how the FW pipeline latency gets absorbed).
        let tau = 3000.0;
        let ns = 4.0;
        let t0 = 300;
        let mut p = TrapParams::from_ns(1000.0 * ns, 250.0 * ns, tau * ns, 80.0, 1, 200, ns);
        let trap = trapezoid_trace(&synth_pulse(6000, t0, tau, 2000.0, -8104.0), &p);
        let (_, i0) = extract_energy(&trap, t0, &p, 0.0);
        p.peak_shift = 19;
        let (_, i1) = extract_energy(&trap, t0, &p, 0.0);
        assert_eq!(i1, i0 + 19);
    }
}

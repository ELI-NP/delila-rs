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
//! | 2 Pedestal   | [`pedestal`]          | yes (it stands in for the pre-record history) |
//! | 3 Trapezoid  | [`trapezoid_trace`]   | yes (shaping) |
//! | 4 Energy     | [`extract_energy`]    | yes (noise averaging) |
//! | 5 Gain       | [`TrapParams::gain`]  | — (normalizes to input ADC units) |
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
//! The pole-zero term `M·d[n]` compensates the preamp exponential decay so a
//! matched `M` yields a flat top.
//!
//! # A record is not a stream: the pre-record history (2026-09-30)
//!
//! The FW runs this recursion on a continuous stream; offline we only have a
//! finite record, so something must stand in for `v[i]`, `i < 0`. The first
//! implementation clamped it to `input[0]`. That is exact for noise-free input
//! but with real noise it replicates ONE noisy sample `k` times into the
//! baseline-side window, i.e. it injects that sample's noise at full weight
//! (σ/A instead of σ·√(2/k)/A — 425 ppm vs 20 ppm per ADC count of noise for
//! the es2 pulser geometry). This, not a missing "dynamic BLR", is what made the
//! July-2026 SW spread 4.5× the FW's. The pre-record history is now the
//! **pedestal** = mean of the pre-trigger samples ([`pedestal`]).
//!
//! The pedestal only *stands in* for missing data. A parameter set is faithfully
//! replayed only if both `k`-sample windows of the trapezoid lie inside the
//! record, which needs `pre-trigger ≥ rise + (1 − peak%)·flat`
//! ([`TrapParams::check_window`]). The grid scan uses [`Prepared`], which
//! evaluates the same filter in closed form from prefix sums (O(1) per parameter
//! set) and *refuses* parameter sets the record cannot support instead of
//! silently degrading them.

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
    /// FW `N Samples Baseline` (BLINE_NSMEAN). **Not used by the offline replay**
    /// (the FW averages a settled trapezoid over a long stream history, which a
    /// finite record does not contain — see the module docs); kept so a
    /// `TrapParams` mirrors the complete FW operating point.
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

    /// Flat-top height per unit input amplitude: a step of amplitude `A` with a
    /// matched pole-zero gives `s = A · k · (M + 1)` on the flat top. Dividing by
    /// this expresses the energy in **input ADC counts**, which keeps the peak
    /// position (nearly) fixed across a rise/flat scan.
    #[inline]
    pub fn gain(&self) -> f64 {
        self.rise as f64 * (self.pz_multiplier + 1.0)
    }

    /// Energy sampling position relative to the trigger sample:
    /// `rise + peak_pct%·flat + peak_shift`.
    #[inline]
    pub fn peak_offset(&self) -> isize {
        (self.rise as f64 + (self.peak_pct / 100.0) * self.flat as f64).round() as isize
            + self.peak_shift
    }

    /// Samples required **before** the trigger so that the baseline-side window
    /// of the trapezoid consists of recorded data (not the pedestal stand-in):
    /// `rise + (1 − peak%)·flat − peak_shift` plus half the peak averaging.
    pub fn pre_trigger_needed(&self) -> usize {
        let need =
            (self.rise + self.span() + self.peak_nsmean / 2) as isize - 1 - self.peak_offset();
        need.max(PEDESTAL_GUARD as isize + 1) as usize
    }

    /// Samples required from the trigger to the end of the record so that the
    /// whole peak-averaging window is recorded.
    pub fn post_trigger_needed(&self) -> usize {
        let need = self.peak_offset() - (self.peak_nsmean / 2) as isize + self.peak_nsmean as isize;
        need.max(1) as usize
    }

    /// Can this parameter set be replayed faithfully on a record of `len` samples
    /// whose trigger sits at sample `trigger`? `Err` names what is missing —
    /// callers must surface it (no silently degraded grid points).
    pub fn check_window(&self, len: usize, trigger: usize) -> Result<(), WindowError> {
        let need = self.pre_trigger_needed();
        if trigger < need {
            return Err(WindowError::PreTriggerTooShort {
                need,
                have: trigger,
            });
        }
        let need = self.post_trigger_needed();
        let have = len.saturating_sub(trigger);
        if have < need {
            return Err(WindowError::PostTriggerTooShort { need, have });
        }
        Ok(())
    }
}

/// Why a parameter set cannot be replayed on a given record (all in samples).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    /// The baseline-side window would reach before the first recorded sample.
    PreTriggerTooShort { need: usize, have: usize },
    /// The peak-averaging window would reach past the last recorded sample.
    PostTriggerTooShort { need: usize, have: usize },
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreTriggerTooShort { need, have } => {
                write!(
                    f,
                    "pre-trigger too short: need {need} samples, record has {have}"
                )
            }
            Self::PostTriggerTooShort { need, have } => {
                write!(
                    f,
                    "record too short after the trigger: need {need} samples, have {have}"
                )
            }
        }
    }
}

impl std::error::Error for WindowError {}

/// Samples immediately before the trigger that are excluded from the pedestal:
/// the pulse starts before the discriminator fires (input rise time + RC-CR2
/// delay), so the last ~100 ns before the trigger are not baseline.
pub const PEDESTAL_GUARD: usize = 32;

/// Stage 2 — the raw-input pedestal: mean of the samples before
/// `trigger − PEDESTAL_GUARD`. It stands in for the pre-record history. Falls
/// back to `input[0]` when the record has no usable pre-trigger (degenerate; the
/// window check rejects such records).
pub fn pedestal(input: &[f64], trigger: usize) -> f64 {
    let end = trigger.saturating_sub(PEDESTAL_GUARD).min(input.len());
    if end == 0 {
        input.first().copied().unwrap_or(0.0)
    } else {
        mean(&input[..end])
    }
}

/// How far before the FW trigger marker the pulse may start (samples). On the
/// ELIADE V1725 the D0 marker lands ~40 samples after the half height of an
/// HPGe edge (2026-10-01, run 22), so 128 samples covers rise + latency.
pub const MIDPOINT_LOOKBACK: usize = 128;

/// Samples after the marker over which the post-step level is averaged
/// (skipping the first few, which may still be on the edge). 64 samples span
/// about one period of the 2–4 MHz pickup seen on the same detector.
const MIDPOINT_LEVEL: std::ops::Range<usize> = 8..72;

/// Smallest step, in units of the pedestal noise, for which the half-height
/// crossing is located; smaller pulses fall back to the marker.
const MIDPOINT_MIN_SNR: f64 = 4.0;

/// Half-height crossing of the pulse edge, used as the time reference for the
/// energy sampling position instead of the FW D0 trigger marker.
///
/// Why: the FW samples the trapezoid `rise + peak%·flat` after *its* trigger,
/// which fires at the RC-CR2 zero crossing ≈ the half height of the edge. The
/// D0 marker in the recorded waveform is delayed with respect to the input
/// probe (FW pipeline latency), so sampling relative to the marker lands late
/// — past the end of a short flat top. The half height is recovered from the
/// record itself: pedestal = mean before `marker − MIDPOINT_LOOKBACK`, level =
/// mean of `marker + MIDPOINT_LEVEL`, then the last sample before the marker
/// whose 4-sample average is still below 50 % of the step, plus one.
///
/// `None` when the record is too short around the marker, the step is below
/// `MIDPOINT_MIN_SNR` pedestal-noise units, or the edge started more than
/// `MIDPOINT_LOOKBACK` samples before the marker. Either polarity works.
pub fn pulse_midpoint<T: Copy + Into<f64>>(samples: &[T], marker: usize) -> Option<usize> {
    let ped_end = marker.checked_sub(MIDPOINT_LOOKBACK)?;
    if ped_end < 16 || marker + MIDPOINT_LEVEL.end > samples.len() {
        return None;
    }
    let x = |i: usize| -> f64 { samples[i].into() };
    let ped = (0..ped_end).map(x).sum::<f64>() / ped_end as f64;
    let ped_rms = ((0..ped_end).map(|i| (x(i) - ped).powi(2)).sum::<f64>() / ped_end as f64).sqrt();
    let level = MIDPOINT_LEVEL.map(|i| x(marker + i)).sum::<f64>() / MIDPOINT_LEVEL.len() as f64;
    let step = level - ped;
    if step.abs() < MIDPOINT_MIN_SNR * ped_rms.max(1.0) {
        return None;
    }
    let half = 0.5 * step.abs();
    // Centered 4-sample average (i−2..=i+1) keeps the crossing unbiased.
    let above =
        |i: usize| (((i - 2)..=(i + 1)).map(x).sum::<f64>() / 4.0 - ped) * step.signum() >= half;
    // Start just before the level window: the edge must be past half height
    // there, otherwise the level itself is not the post-step plateau.
    let start = marker + MIDPOINT_LEVEL.start - 2;
    if !above(start) {
        return None;
    }
    (ped_end.max(2)..start)
        .rev()
        .find(|&i| !above(i))
        .map(|i| i + 1)
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
/// Pre-history (`v[i]` for `i < 0`) is taken to be `pedestal` (see [`pedestal`]
/// and the module docs — never a single input sample, which would inject that
/// sample's noise with weight `k`).
pub fn trapezoid_trace(input: &[f64], pedestal: f64, p: &TrapParams) -> Vec<f64> {
    let n = input.len();
    let mut s = vec![0.0f64; n];
    if n == 0 {
        return s;
    }
    let k = p.rise as isize;
    let l = p.span() as isize;
    let kl = k + l;
    let v = |i: isize| -> f64 {
        if i < 0 {
            pedestal
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

/// Stage 4 — extract the energy from the trapezoid.
///
/// Samples `s[n]` at the peaking position `trigger + rise + peak_pct%·flat`,
/// averages `peak_nsmean` samples centered there, and subtracts `baseline`.
/// Returns `(energy, peak_center_index)`.
pub fn extract_energy(trap: &[f64], trigger: usize, p: &TrapParams, baseline: f64) -> (f64, usize) {
    let center = (trigger as isize + p.peak_offset()).max(0) as usize;
    let half = p.peak_nsmean / 2;
    let lo = center.saturating_sub(half).min(trap.len());
    let hi = (lo + p.peak_nsmean).min(trap.len());
    (mean(&trap[lo..hi]) - baseline, center)
}

/// Result of a single-event trapezoid analysis.
#[derive(Debug, Clone)]
pub struct TrapResult {
    /// Extracted energy in input ADC counts (flat-top height / [`TrapParams::gain`]).
    pub energy: f64,
    /// Raw-input pedestal (stage 2) used as the pre-record history.
    pub pedestal: f64,
    /// Sample index where the energy was evaluated (stage 4).
    pub peak_index: usize,
    /// Full `s[n]` trapezoid trace (stage 3, un-normalized), for per-stage inspection / overlay
    /// against the FW `analog_probe2`. `None` unless [`analyze_with_trace`] is used.
    pub trap: Option<Vec<f64>>,
}

/// Full single-event analysis (stages 2–4) via the recursion, discarding the
/// trapezoid trace. Evaluates whatever the record allows — when the pre-trigger
/// is shorter than [`TrapParams::pre_trigger_needed`] the pedestal fills in and
/// the result is noisier than the FW's. Use [`Prepared::energy`] when that must
/// be an error (grid scans).
pub fn analyze(input: &[f64], trigger: usize, p: &TrapParams) -> TrapResult {
    let mut r = analyze_with_trace(input, trigger, p);
    r.trap = None;
    r
}

/// As [`analyze`], keeping the trapezoid trace for inspection (Phase-1 probe
/// overlay / debugging).
pub fn analyze_with_trace(input: &[f64], trigger: usize, p: &TrapParams) -> TrapResult {
    let ped = pedestal(input, trigger);
    let trap = trapezoid_trace(input, ped, p);
    let (raw, peak_index) = extract_energy(&trap, trigger, p, 0.0);
    TrapResult {
        energy: raw / p.gain(),
        pedestal: ped,
        peak_index,
        trap: Some(trap),
    }
}

/// One event prepared for O(1) evaluation of any parameter set.
///
/// The trapezoid is the difference of two `k`-sample window sums of the
/// pole-zero-deconvolved signal `Q[i] = M·v'[i] + Σ_{j≤i} v'[j]` (`v'` =
/// pedestal-subtracted input):
///
/// ```text
/// s[n] = Σ_{i=n−k+1..n} Q[i]  −  Σ_{i=n−l−k+1..n−l} Q[i]
/// ```
///
/// which is algebraically identical to the recursion in [`trapezoid_trace`].
/// Two prefix-sum arrays turn each window sum into a subtraction, so a grid scan
/// costs O(samples) once per event plus O(1) per parameter set — and `M` enters
/// linearly, so the pole-zero can be scanned for free.
///
/// Because both windows have the same length, the unknown pre-record part of
/// the running sum cancels: when the windows lie inside the record the result
/// uses recorded samples only (the pedestal enters with the small weight
/// `l/(M+1)`). [`Prepared::energy`] therefore rejects parameter sets whose
/// windows do not fit ([`TrapParams::check_window`]).
#[derive(Debug, Clone)]
pub struct Prepared {
    /// `c1[i] = Σ_{j<i} v'[j]`
    c1: Vec<f64>,
    /// `c2[i] = Σ_{j<i} c1[j+1]` (prefix sum of the running sum)
    c2: Vec<f64>,
    trigger: usize,
    pedestal: f64,
    tilt: f64,
}

impl Prepared {
    /// Prepare one record (`samples` = raw ADC trace, `trigger` = trigger sample).
    pub fn new(samples: &[i16], trigger: usize) -> Self {
        let end = trigger.saturating_sub(PEDESTAL_GUARD).min(samples.len());
        let sum = |xs: &[i16]| xs.iter().map(|&x| x as f64).sum::<f64>();
        let pedestal = if end == 0 {
            samples.first().map(|&x| x as f64).unwrap_or(0.0)
        } else {
            sum(&samples[..end]) / end as f64
        };
        let tilt = if end >= 2 {
            let h = end / 2;
            sum(&samples[h..end]) / (end - h) as f64 - sum(&samples[..h]) / h as f64
        } else {
            0.0
        };
        let mut c1 = Vec::with_capacity(samples.len() + 1);
        let mut c2 = Vec::with_capacity(samples.len() + 1);
        c1.push(0.0);
        c2.push(0.0);
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for &x in samples {
            a += x as f64 - pedestal;
            c1.push(a);
            b += a;
            c2.push(b);
        }
        Self {
            c1,
            c2,
            trigger,
            pedestal,
            tilt,
        }
    }

    /// Number of samples in the record.
    pub fn len(&self) -> usize {
        self.c1.len() - 1
    }

    /// `true` for an empty record.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Trigger sample index.
    pub fn trigger(&self) -> usize {
        self.trigger
    }

    /// Pedestal (mean of the pre-trigger samples), in ADC counts.
    pub fn pedestal(&self) -> f64 {
        self.pedestal
    }

    /// Pedestal tilt: mean of the second half of the pre-trigger window minus
    /// the mean of the first half (ADC counts). A value far from the population
    /// marks a record sitting on the tail of an earlier pulse (pile-up), whose
    /// pedestal is not the true baseline.
    pub fn pedestal_tilt(&self) -> f64 {
        self.tilt
    }

    /// Pedestal-subtracted sample `v'[i]`.
    pub fn sample(&self, i: usize) -> f64 {
        self.c1[i + 1] - self.c1[i]
    }

    /// Un-normalized trapezoid `s[n]`; the caller guarantees `n + 1 ≥ k + l`.
    #[inline]
    fn trap_at(&self, n: usize, k: usize, l: usize, m: f64) -> f64 {
        let window = |c: &[f64], hi: usize| c[hi + 1] - c[hi + 1 - k];
        m * (window(&self.c1, n) - window(&self.c1, n - l))
            + (window(&self.c2, n) - window(&self.c2, n - l))
    }

    /// Energy in input ADC counts for parameter set `p`, or the reason the
    /// record cannot support it.
    pub fn energy(&self, p: &TrapParams) -> Result<f64, WindowError> {
        p.check_window(self.len(), self.trigger)?;
        let center = (self.trigger as isize + p.peak_offset()) as usize;
        let lo = center - p.peak_nsmean / 2;
        let (k, l) = (p.rise, p.span());
        let sum: f64 = (lo..lo + p.peak_nsmean)
            .map(|n| self.trap_at(n, k, l, p.pz_multiplier))
            .sum();
        Ok(sum / p.peak_nsmean as f64 / p.gain())
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
        let trap = trapezoid_trace(&input, -8104.0, &p);
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
        let trap = trapezoid_trace(&input, -8104.0, &p);
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
            let trap = trapezoid_trace(&input, -8104.0, &p);
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
        let trap = trapezoid_trace(&synth_pulse(6000, t0, tau, 2000.0, -8104.0), -8104.0, &p);
        let (_, i0) = extract_energy(&trap, t0, &p, 0.0);
        p.peak_shift = 19;
        let (_, i1) = extract_energy(&trap, t0, &p, 0.0);
        assert_eq!(i1, i0 + 19);
    }

    // ---- pre-record history, window checks, closed form (2026-09-30) ----

    use crate::offline::testutil::{noisy_event, rng};

    /// es2 pulser geometry: 20 µs record, rise 5 µs / flat 1 µs, τ = 50 µs.
    fn es2_params() -> TrapParams {
        TrapParams::from_ns(5000.0, 1000.0, 50000.0, 80.0, 1, 256, 4.0)
    }

    fn rel_spread_ppm(xs: &[f64]) -> f64 {
        let m = mean(xs);
        let var = xs.iter().map(|&x| (x - m) * (x - m)).sum::<f64>() / (xs.len() - 1) as f64;
        var.sqrt() / m.abs() * 1e6
    }

    #[test]
    fn noisy_first_sample_does_not_leak_into_energy() {
        // Regression for the July-2026 "SW spread = 4.5 × FW" finding. With 1 ADC
        // count of white noise and the es2 geometry (pre-trigger 252 samples «
        // rise 1250), clamping the pre-record history to input[0] gave ~425 ppm;
        // the pedestal stand-in gives ~38 ppm (= σ·√(1/k + 1/P)/A).
        let p = es2_params();
        let mut g = rng(1);
        let energies: Vec<f64> = (0..400)
            .map(|_| {
                let ev = noisy_event(&mut g, 5000, 252, 12500.0, 1970.0, -8104.0, 0, 1.0);
                let input: Vec<f64> = ev.iter().map(|&x| x as f64).collect();
                analyze(&input, 252, &p).energy
            })
            .collect();
        let ppm = rel_spread_ppm(&energies);
        assert!(
            ppm < 80.0,
            "relative spread {ppm:.0} ppm (input[0] clamp gave ~425)"
        );
    }

    #[test]
    fn energy_is_the_pulse_amplitude_in_adc_counts() {
        let p = es2_params();
        let input = synth_pulse(5000, 1600, 12500.0, 2000.0, -8104.0);
        let e = analyze(&input, 1600, &p).energy;
        assert!(
            (e / 2000.0 - 1.0).abs() < 5e-3,
            "energy {e} for amplitude 2000"
        );
    }

    #[test]
    fn closed_form_matches_recursion() {
        // Prepared::energy must equal the recursion sample-for-sample whenever
        // the windows fit in the record — for any rise/flat/peaking/nsmean/shift.
        let mut g = rng(7);
        let ev = noisy_event(&mut g, 5000, 1600, 12500.0, 1970.0, -8104.0, 40, 2.0);
        let input: Vec<f64> = ev.iter().map(|&x| x as f64).collect();
        let prep = Prepared::new(&ev, 1600);
        for (rise, flat, pct, nsmean, shift) in [
            (1250, 250, 80.0, 1, 0),
            (500, 125, 50.0, 4, 19),
            (1400, 300, 90.0, 16, -5),
            (250, 64, 80.0, 64, 0),
        ] {
            let mut p = TrapParams::from_ns(
                rise as f64 * 4.0,
                flat as f64 * 4.0,
                50000.0,
                pct,
                nsmean,
                256,
                4.0,
            );
            p.peak_shift = shift;
            let direct = prep.energy(&p).expect("window fits");
            let recursion = analyze(&input, 1600, &p).energy;
            assert!(
                (direct - recursion).abs() < 1e-6 * recursion.abs(),
                "rise {rise} flat {flat}: closed form {direct} vs recursion {recursion}"
            );
        }
    }

    #[test]
    fn check_window_names_what_is_missing() {
        let p = es2_params(); // rise 1250, flat 250, peak 80 % → needs 1250 + 50 − 1 before the trigger
        assert_eq!(p.pre_trigger_needed(), 1299);
        assert_eq!(
            p.check_window(5000, 252),
            Err(WindowError::PreTriggerTooShort {
                need: 1299,
                have: 252
            })
        );
        assert_eq!(p.check_window(5000, 1299), Ok(()));
        // peak sample = trigger + 1450 → needs 1451 samples from the trigger on.
        assert_eq!(p.post_trigger_needed(), 1451);
        assert_eq!(
            p.check_window(2700, 1300),
            Err(WindowError::PostTriggerTooShort {
                need: 1451,
                have: 1400
            })
        );
        // Prepared::energy surfaces the same error instead of a degraded value.
        let prep = Prepared::new(&vec![0i16; 5000], 252);
        assert!(matches!(
            prep.energy(&p),
            Err(WindowError::PreTriggerTooShort { .. })
        ));
    }

    #[test]
    fn sufficient_pre_trigger_reaches_the_white_noise_limit() {
        // With the baseline-side window fully recorded the spread must approach
        // the symmetric-trapezoid limit σ·√(2/k)/A = 20 ppm (1 ADC count noise).
        let p = es2_params();
        let mut g = rng(3);
        let energies: Vec<f64> = (0..400)
            .map(|_| {
                let ev = noisy_event(&mut g, 5000, 1600, 12500.0, 1970.0, -8104.0, 0, 1.0);
                Prepared::new(&ev, 1600).energy(&p).expect("window fits")
            })
            .collect();
        let ppm = rel_spread_ppm(&energies);
        assert!(
            (15.0..30.0).contains(&ppm),
            "relative spread {ppm:.1} ppm, expected ≈ 20"
        );
    }

    #[test]
    fn pedestal_tilt_flags_a_record_on_a_pileup_tail() {
        let mut g = rng(5);
        let clean = noisy_event(&mut g, 5000, 1600, 12500.0, 1970.0, -8104.0, 0, 1.0);
        // Same pulse riding on the decaying tail of an earlier one.
        let tail: Vec<i16> = clean
            .iter()
            .enumerate()
            .map(|(i, &x)| x + (300.0 * (-(i as f64) / 12500.0).exp()).round() as i16)
            .collect();
        let t_clean = Prepared::new(&clean, 1600).pedestal_tilt();
        let t_tail = Prepared::new(&tail, 1600).pedestal_tilt();
        assert!(t_clean.abs() < 0.5, "clean tilt {t_clean}");
        assert!(t_tail < -10.0, "tail tilt {t_tail}");
    }

    #[test]
    fn pulse_midpoint_finds_the_half_height_before_a_late_marker() {
        // Charge collected over 60 samples from t0 = 1000 → half height at ≈1030.
        // The FW trigger marker is recorded 40 samples after the rise ends.
        let mut g = rng(11);
        let ev = noisy_event(&mut g, 5000, 1000, 12500.0, 3000.0, 3100.0, 60, 20.0);
        let mid = pulse_midpoint(&ev, 1100).expect("clear step");
        assert!((1027..=1033).contains(&mid), "midpoint {mid}");
    }

    #[test]
    fn pulse_midpoint_handles_negative_pulses() {
        let mut g = rng(12);
        let ev = noisy_event(&mut g, 5000, 1000, 12500.0, -3000.0, 12000.0, 60, 20.0);
        let mid = pulse_midpoint(&ev, 1100).expect("clear step");
        assert!((1027..=1033).contains(&mid), "midpoint {mid}");
    }

    #[test]
    fn pulse_midpoint_is_none_without_a_step() {
        let mut g = rng(13);
        let ev = noisy_event(&mut g, 5000, 1000, 12500.0, 0.0, 3100.0, 60, 20.0);
        assert_eq!(pulse_midpoint(&ev, 1100), None);
    }

    #[test]
    fn midpoint_reference_keeps_the_sample_on_a_short_flat_top() {
        // Rise 250 / flat 100 samples and a 20-sample charge collection leave
        // the flat region [t0+270, t0+350]; peaking 80 % from the half height
        // (t0+10) samples t0+340. A marker 100 samples after t0 moves the
        // sampling point to t0+430, 80 samples down the falling edge.
        let p = TrapParams::from_ns(1000.0, 400.0, 50000.0, 80.0, 1, 256, 4.0);
        let mut g = rng(14);
        let ev = noisy_event(&mut g, 5000, 1000, 12500.0, 3000.0, 3100.0, 20, 2.0);
        let from_marker = Prepared::new(&ev, 1100).energy(&p).expect("fits");
        let mid = pulse_midpoint(&ev, 1100).expect("clear step");
        let from_mid = Prepared::new(&ev, mid).energy(&p).expect("fits");
        assert!(
            (from_mid / 3000.0 - 1.0).abs() < 0.01,
            "midpoint reference gives {from_mid:.1}"
        );
        assert!(
            from_marker < 0.9 * 3000.0,
            "marker reference should fall off the flat top, got {from_marker:.1}"
        );
    }
}

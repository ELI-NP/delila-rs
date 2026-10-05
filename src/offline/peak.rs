//! Photopeak width estimator for the grid scan (TODO 59 §5.4).
//!
//! The scan re-evaluates the *same* events at every parameter set, so the peak
//! moves (gain, ballistic deficit) and changes width between evaluations. The
//! estimator therefore **finds the peak afresh each time** instead of using a
//! fixed window:
//!
//! 1. `locate` — the shortest interval holding 10 % of the events gives a
//!    rough, *local* centre and σ (works even when the selection is much wider
//!    than the line, and on integer-quantized FW energies).
//! 2. Soft-window moments, unbinned: weight every event with a Gaussian window
//!    `exp(−(x−μ)²/2s²)`, `s` = current σ. For a Gaussian line the windowed
//!    mean/variance invert in closed form to the true ones
//!    (`1/σ² = 1/σ_w² − 1/s²`), so a few iterations converge and the result is
//!    exact for a Gaussian. A linear background, measured in side bands at
//!    4.5–6σ, is subtracted from the moments analytically.
//!
//! The soft window weights the core of the peak (weight ½ at the half-maximum
//! points, 1 % at 3σ), so the result tracks the FWHM a spectroscopist quotes and
//! is insensitive to low-energy tailing. Being unbinned and smooth in (μ, σ) it
//! has no binning jitter: the ranking of neighbouring grid points, which share
//! their events, is far more precise than the ~1.5/√N error of one width.
//!
//! Precondition: `values` are pre-selected around ONE line (the caller picks the
//! events, e.g. by the FW energy). A sample with no line does not return `None`:
//! it yields a width comparable to the spread of the sample, which ranks it last.

/// Gaussian σ → FWHM.
pub const SIGMA_TO_FWHM: f64 = 2.354_820_045;

/// Fewer events than this cannot support a width estimate.
pub const MIN_EVENTS: usize = 200;

/// Side bands for the background: from this many σ …
const SIDEBAND_INNER_SIGMA: f64 = 4.5;
/// … to this many σ on either side of the peak.
const SIDEBAND_OUTER_SIGMA: f64 = 6.0;
/// The peak must carry at least this fraction of the windowed counts.
const MIN_SIGNAL_FRACTION: f64 = 0.1;
/// Iteration stops when σ and the centre move by less than this fraction of σ.
const CONVERGENCE: f64 = 1e-3;
/// …and a result is still accepted if the last step was below this.
const ACCEPTABLE_LAST_STEP: f64 = 1e-2;
const MAX_ITERATIONS: usize = 60;

/// Result of a photopeak fit, in the units of the input values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeakFit {
    /// Peak position.
    pub centroid: f64,
    /// Full width at half maximum of the Gaussian core.
    pub fwhm: f64,
    /// Background-subtracted number of events in the peak.
    pub counts: f64,
}

impl PeakFit {
    /// `fwhm / centroid` — the scale-free resolution.
    pub fn rel_fwhm(&self) -> f64 {
        self.fwhm / self.centroid.abs()
    }
}

/// Median of a sorted slice (must be non-empty).
fn median_sorted(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        0.5 * (xs[n / 2 - 1] + xs[n / 2])
    }
}

/// Robust centre and scale: `(median, 1.4826 · MAD)`. `None` for an empty input.
pub fn median_and_sigma(values: &[f64]) -> Option<(f64, f64)> {
    let mut xs: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(|a, b| a.total_cmp(b));
    let med = median_sorted(&xs);
    let mut dev: Vec<f64> = xs.iter().map(|x| (x - med).abs()).collect();
    dev.sort_by(|a, b| a.total_cmp(b));
    Some((med, 1.4826 * median_sorted(&dev)))
}

/// Number of values in `[lo, hi)` of a sorted slice.
fn count_in(sorted: &[f64], lo: f64, hi: f64) -> usize {
    sorted.partition_point(|&x| x < hi) - sorted.partition_point(|&x| x < lo)
}

/// Rough local `(centre, σ)` of the tallest peak in a sorted sample, from the
/// **shortest interval containing a fraction `q` of the events** (the mode lives
/// where the data are densest). For a Gaussian that interval is `2·z(q)·σ` long.
/// No binning, so it works unchanged on integer-quantized energies: when ties
/// make the interval zero-length the fraction is raised until it has a width.
fn locate(sorted: &[f64]) -> Option<(f64, f64)> {
    // (q, z) with z = Φ⁻¹((1 + q)/2): half-length of the central-q interval in σ.
    const FRACTIONS: [(f64, f64); 5] = [
        (0.1, 0.1257),
        (0.2, 0.2533),
        (0.4, 0.5244),
        (0.6, 0.8416),
        (0.8, 1.2816),
    ];
    let n = sorted.len();
    for (q, z) in FRACTIONS {
        let m = ((q * n as f64).ceil() as usize).clamp(2, n);
        let (start, length) = (0..=n - m)
            .map(|i| (i, sorted[i + m - 1] - sorted[i]))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        if length > 0.0 && length.is_finite() {
            return Some((
                0.5 * (sorted[start] + sorted[start + m - 1]),
                length / (2.0 * z),
            ));
        }
    }
    None // (nearly) all values identical
}

/// Fit the single photopeak in `values`. See the module docs for the method and
/// its precondition. `None` when the sample is too small, has no spread, or the
/// iteration does not settle.
pub fn fit_peak(values: &[f64]) -> Option<PeakFit> {
    let mut xs: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if xs.len() < MIN_EVENTS {
        return None;
    }
    xs.sort_by(|a, b| a.total_cmp(b));
    let (mut mu, mut sigma) = locate(&xs)?;
    let root_2pi = (2.0 * std::f64::consts::PI).sqrt();

    let mut last = None;
    for _ in 0..MAX_ITERATIONS {
        let s = sigma; // soft-window σ
                       // Windowed moments about the current centre (weights vanish beyond 6s).
        let a = xs.partition_point(|&x| x < mu - SIDEBAND_OUTER_SIGMA * s);
        let b = xs.partition_point(|&x| x < mu + SIDEBAND_OUTER_SIGMA * s);
        let (mut s0, mut s1, mut s2) = (0.0f64, 0.0f64, 0.0f64);
        for &x in &xs[a..b] {
            let u = x - mu;
            let w = (-0.5 * (u / s) * (u / s)).exp();
            s0 += w;
            s1 += w * u;
            s2 += w * u * u;
        }
        let windowed = s0;

        // Linear background from the side bands, removed analytically.
        let (inner, outer) = (SIDEBAND_INNER_SIGMA * sigma, SIDEBAND_OUTER_SIGMA * sigma);
        let left = count_in(&xs, mu - outer, mu - inner) as f64 / (outer - inner);
        let right = count_in(&xs, mu + inner, mu + outer) as f64 / (outer - inner);
        let density = 0.5 * (left + right);
        let slope = (right - left) / (inner + outer);
        s0 -= density * s * root_2pi;
        s1 -= slope * s.powi(3) * root_2pi;
        s2 -= density * s.powi(3) * root_2pi;
        if s0 < MIN_SIGNAL_FRACTION * windowed || s0 < 20.0 {
            return None; // nothing stands out of the background here
        }

        let mean_w = s1 / s0;
        let var_w = s2 / s0 - mean_w * mean_w;
        // Undo the window: 1/σ² = 1/σ_w² − 1/s².
        let inv = 1.0 / var_w - 1.0 / (s * s);
        if !(var_w > 0.0 && inv > 0.0 && inv.is_finite()) {
            return None;
        }
        let var = 1.0 / inv;
        let new_mu = mu + mean_w * var / var_w;
        // Limit the step so a rough start cannot overshoot.
        let new_sigma = var.sqrt().clamp(sigma / 3.0, sigma * 3.0);

        let step = (new_sigma / sigma - 1.0)
            .abs()
            .max((new_mu - mu).abs() / sigma);
        let fit = PeakFit {
            centroid: new_mu,
            fwhm: SIGMA_TO_FWHM * new_sigma,
            counts: s0 * (1.0 + var / (s * s)).sqrt(),
        };
        mu = new_mu;
        sigma = new_sigma;
        last = Some((fit, step));
        if step < CONVERGENCE {
            break;
        }
    }
    last.filter(|(_, step)| *step < ACCEPTABLE_LAST_STEP)
        .map(|(fit, _)| fit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::testutil::rng;
    use rand::Rng;
    use rand_distr::{Distribution, Exp, Normal};

    fn gaussian(seed: u64, n: usize, mu: f64, sigma: f64) -> Vec<f64> {
        let mut g = rng(seed);
        let d = Normal::new(mu, sigma).unwrap();
        (0..n).map(|_| d.sample(&mut g)).collect()
    }

    fn assert_close(got: f64, want: f64, tol: f64, what: &str) {
        assert!(
            (got / want - 1.0).abs() < tol,
            "{what}: got {got}, want {want} (±{:.0} %)",
            tol * 100.0
        );
    }

    #[test]
    fn recovers_width_and_position_of_a_clean_peak() {
        let fit = fit_peak(&gaussian(1, 50_000, 1970.0, 0.84)).expect("peak");
        assert_close(fit.fwhm, SIGMA_TO_FWHM * 0.84, 0.02, "fwhm");
        assert!(
            (fit.centroid - 1970.0).abs() < 0.02,
            "centroid {}",
            fit.centroid
        );
        assert_close(fit.counts, 50_000.0, 0.02, "counts");
    }

    #[test]
    fn works_with_a_few_thousand_events() {
        let fit = fit_peak(&gaussian(2, 3_000, 4045.0, 1.2)).expect("peak");
        assert_close(fit.fwhm, SIGMA_TO_FWHM * 1.2, 0.08, "fwhm");
    }

    #[test]
    fn is_scale_and_offset_invariant() {
        let xs = gaussian(3, 20_000, 10.0, 0.01);
        let a = fit_peak(&xs).expect("peak");
        let ys: Vec<f64> = xs.iter().map(|x| 1000.0 * x + 5.0).collect();
        let b = fit_peak(&ys).expect("peak");
        assert_close(b.fwhm, 1000.0 * a.fwhm, 1e-6, "fwhm scaling");
        assert_close(
            b.centroid,
            1000.0 * a.centroid + 5.0,
            1e-9,
            "centroid scaling",
        );
    }

    #[test]
    fn subtracts_a_flat_background() {
        // 25 % of the events are continuum spread over ±20σ around the line.
        let mut xs = gaussian(4, 40_000, 1332.5, 0.9);
        let mut g = rng(40);
        xs.extend((0..13_000).map(|_| 1332.5 + g.gen_range(-18.0..18.0)));
        let fit = fit_peak(&xs).expect("peak");
        assert_close(fit.fwhm, SIGMA_TO_FWHM * 0.9, 0.04, "fwhm");
        assert_close(fit.counts, 40_000.0, 0.05, "counts");
    }

    #[test]
    fn low_energy_tail_barely_moves_the_core_width() {
        // 15 % of the events lose an exponentially distributed amount (mean 2σ).
        let mut g = rng(5);
        let loss = Exp::new(1.0 / (2.0 * 0.9)).unwrap();
        let xs: Vec<f64> = gaussian(50, 50_000, 1332.5, 0.9)
            .into_iter()
            .enumerate()
            .map(|(i, x)| {
                if i % 20 < 3 {
                    x - loss.sample(&mut g)
                } else {
                    x
                }
            })
            .collect();
        let fit = fit_peak(&xs).expect("peak");
        let pure = SIGMA_TO_FWHM * 0.9;
        assert!(
            fit.fwhm > 0.98 * pure && fit.fwhm < 1.10 * pure,
            "fwhm {} vs pure {pure}",
            fit.fwhm
        );
    }

    #[test]
    fn finds_the_line_in_a_wide_selection() {
        // Selection window much wider than the peak, continuum-dominated scale.
        let mut xs = gaussian(6, 20_000, 1332.5, 0.9);
        let mut g = rng(60);
        xs.extend((0..20_000).map(|_| 1332.5 + g.gen_range(-40.0..40.0)));
        let fit = fit_peak(&xs).expect("peak");
        assert_close(fit.fwhm, SIGMA_TO_FWHM * 0.9, 0.06, "fwhm");
    }

    #[test]
    fn handles_integer_quantized_energies() {
        // FW energies are whole LSBs; a pulser peak can be ~1 LSB wide. Rounding
        // adds 1/12 LSB² of variance (Sheppard) — that IS the recorded spectrum.
        for (seed, sigma) in [(9, 1.0f64), (10, 3.0)] {
            let xs: Vec<f64> = gaussian(seed, 40_000, 4045.3, sigma)
                .iter()
                .map(|x| x.round())
                .collect();
            let fit = fit_peak(&xs).expect("peak");
            let want = SIGMA_TO_FWHM * (sigma * sigma + 1.0 / 12.0).sqrt();
            assert_close(fit.fwhm, want, 0.05, "quantized fwhm");
            assert!(
                (fit.centroid - 4045.3).abs() < 0.1,
                "centroid {}",
                fit.centroid
            );
        }
    }

    #[test]
    fn returns_none_without_a_usable_peak() {
        assert!(
            fit_peak(&gaussian(7, 50, 100.0, 1.0)).is_none(),
            "too few events"
        );
        assert!(fit_peak(&vec![42.0; 5_000]).is_none(), "zero width");
    }

    #[test]
    fn a_sample_without_a_line_gets_a_width_comparable_to_its_spread() {
        // Not None: a smeared-out distribution must rank last, not vanish.
        let mut g = rng(8);
        let flat: Vec<f64> = (0..20_000).map(|_| g.gen_range(0.0..1000.0)).collect();
        if let Some(fit) = fit_peak(&flat) {
            assert!(fit.fwhm > 300.0, "fwhm {} for a 1000-wide box", fit.fwhm);
        }
    }
}

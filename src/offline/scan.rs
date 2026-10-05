//! rise × flat-top grid scan over a set of recorded events (TODO 59 §5).
//!
//! Every parameter set is replayed on the SAME events ([`Prepared`] makes that
//! O(1) per event and parameter set) and scored by the width of the photopeak
//! ([`fit_peak`]) **and by how many of the events are still in it**.
//!
//! The second criterion matters: the width is that of the Gaussian core, and a
//! setting that loses events into a tail can keep — or even sharpen — its core.
//! A flat top shorter than the charge-collection time is the textbook case
//! (ballistic deficit): only the fastest pulses stay in the peak, and they form
//! a very narrow one. [`ScanReport::best`] therefore only considers points that
//! keep at least [`MIN_RELATIVE_PEAK_FRACTION`] of the best peak content.
//!
//! The peak content is counted in a window of the SAME width at every point:
//! ± [`CONTENT_WINDOW_FWHMS`] × the narrowest credible FWHM on the grid, around
//! each point's centroid — wide enough to hold any reasonably good line whole,
//! narrow enough that a ballistic-deficit tail falls outside. Counting inside
//! each point's own fit window is wrong: a point broadened by noise has a wide
//! window that swallows the low-energy shoulder, so it would set the bar and
//! disqualify the good, narrow points (seen with the periodic pickup on ELIADE
//! SN01, 2026-10-01).
//!
//! Nothing is silently degraded:
//!
//! - a parameter set whose trapezoid windows do not fit in the record is
//!   reported as [`PointOutcome::Window`], never evaluated on stand-in data;
//! - events sitting on the tail of an earlier pulse (their pre-trigger pedestal
//!   is not the baseline) are excluded and **counted**
//!   ([`ScanReport::events_tilt_rejected`]).

use rayon::prelude::*;

use super::peak::{fit_peak, median_and_sigma, PeakFit};
use super::trap::{Prepared, TrapParams, WindowError};

/// Events whose pedestal tilt is further than this many robust σ from the
/// population median are treated as pile-up and excluded.
const TILT_CUT_SIGMA: f64 = 5.0;

/// A point is eligible as "best" only if its peak holds at least this fraction
/// of the largest peak content found on the grid (same events at every point).
pub const MIN_RELATIVE_PEAK_FRACTION: f64 = 0.95;

/// Half width of the common content window in units of the reference FWHM.
pub const CONTENT_WINDOW_FWHMS: f64 = 3.0;

/// A fitted point sets the common content window only if its own fit holds at
/// least this fraction of the events — a narrow core that lost most events to
/// a tail (ballistic deficit) must not define "narrow".
const CONTENT_REFERENCE_MIN_FIT_FRACTION: f64 = 0.5;

/// First tail sample used by [`measure_decay_tau`], relative to the trigger:
/// clear of the charge-collection time (HPGe ≲ 400 ns) at 2–4 ns/sample.
const TAIL_START: usize = 128;

/// One recorded waveform to replay.
#[derive(Debug, Clone)]
pub struct ScanEvent {
    /// Raw ADC trace (`analog_probe1` = Input).
    pub samples: Vec<i16>,
    /// Trigger sample index (first asserted sample of `D0 = Trigger`).
    pub trigger: usize,
}

/// What happened at one grid point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointOutcome {
    /// Photopeak fitted; energies are in input ADC counts.
    Fit(PeakFit),
    /// The record cannot support this parameter set.
    Window(WindowError),
    /// Replayed, but the energies do not form a fittable peak.
    NoPeak,
}

/// One evaluated parameter set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanPoint {
    pub params: TrapParams,
    pub outcome: PointOutcome,
    /// Events replayed at this point (after the pile-up cut).
    pub events: usize,
    /// Fraction of the replayed events within the grid-wide content window
    /// ([`ScanReport::content_half_width`]) of this point's centroid.
    pub content: Option<f64>,
}

impl ScanPoint {
    /// `FWHM / centroid`, when the point was fitted.
    pub fn rel_fwhm(&self) -> Option<f64> {
        match self.outcome {
            PointOutcome::Fit(f) => Some(f.rel_fwhm()),
            _ => None,
        }
    }

    /// Fraction of the replayed events in the peak, counted in the grid-wide
    /// window (see the module docs). Compare it across grid points: a drop
    /// means events leaking into a tail or the line being broadened.
    pub fn peak_fraction(&self) -> Option<f64> {
        self.content
    }

    /// Background-subtracted counts of this point's own fit over the replayed
    /// events (the fit window scales with the fitted width).
    fn fit_fraction(&self) -> Option<f64> {
        match self.outcome {
            PointOutcome::Fit(f) if self.events > 0 => Some(f.counts / self.events as f64),
            _ => None,
        }
    }
}

/// Result of [`scan`].
#[derive(Debug, Clone)]
pub struct ScanReport {
    /// One entry per grid point, in grid order.
    pub points: Vec<ScanPoint>,
    /// Events handed to the scan.
    pub events_total: usize,
    /// Events excluded by the pedestal-tilt (pile-up) cut.
    pub events_tilt_rejected: usize,
    /// Half width of the content window, relative to the centroid:
    /// [`CONTENT_WINDOW_FWHMS`] × the narrowest relative FWHM among points whose
    /// own fit holds at least half of the events. `None` when nothing was fitted.
    pub content_half_width: Option<f64>,
}

impl ScanReport {
    /// Largest peak fraction on the grid (the reference for [`Self::keeps_peak`]).
    pub fn max_peak_fraction(&self) -> Option<f64> {
        self.points
            .iter()
            .filter_map(ScanPoint::peak_fraction)
            .max_by(f64::total_cmp)
    }

    /// Does `point` keep (nearly) all the events in its peak, relative to the
    /// best point of the grid?
    pub fn keeps_peak(&self, point: &ScanPoint) -> bool {
        match (point.peak_fraction(), self.max_peak_fraction()) {
            (Some(f), Some(max)) => f >= MIN_RELATIVE_PEAK_FRACTION * max,
            _ => false,
        }
    }

    /// The point with the smallest relative FWHM among those that keep the peak.
    pub fn best(&self) -> Option<&ScanPoint> {
        self.points
            .iter()
            .filter(|p| self.keeps_peak(p))
            .min_by(|a, b| {
                a.rel_fwhm()
                    .unwrap_or(f64::MAX)
                    .total_cmp(&b.rel_fwhm().unwrap_or(f64::MAX))
            })
    }
}

/// Cartesian product `rises × flats` (both in samples); every other field is
/// taken from `base`.
pub fn grid(base: &TrapParams, rises: &[usize], flats: &[usize]) -> Vec<TrapParams> {
    rises
        .iter()
        .flat_map(|&rise| {
            flats.iter().map(move |&flat| TrapParams {
                rise,
                flat,
                ..*base
            })
        })
        .collect()
}

/// `true` for events whose pedestal tilt is consistent with the population
/// (i.e. not riding on an earlier pulse's tail).
fn tilt_accept(tilts: &[f64]) -> Vec<bool> {
    match median_and_sigma(tilts) {
        Some((med, sigma)) => {
            let cut = (TILT_CUT_SIGMA * sigma).max(1e-9);
            tilts.iter().map(|t| (t - med).abs() <= cut).collect()
        }
        None => vec![true; tilts.len()],
    }
}

/// Replay every parameter set of `grid` on `events` and fit the photopeak.
///
/// `events` must be pre-selected around ONE line (see [`fit_peak`]).
pub fn scan(events: &[ScanEvent], grid: &[TrapParams]) -> ScanReport {
    // energies[event][point]; NaN marks "window does not fit".
    let replayed: Vec<(f64, Vec<f64>)> = events
        .par_iter()
        .map(|ev| {
            let prep = Prepared::new(&ev.samples, ev.trigger);
            let energies = grid
                .iter()
                .map(|p| prep.energy(p).unwrap_or(f64::NAN))
                .collect();
            (prep.pedestal_tilt(), energies)
        })
        .collect();

    let tilts: Vec<f64> = replayed.iter().map(|(t, _)| *t).collect();
    let accept = tilt_accept(&tilts);
    let kept: Vec<usize> = (0..events.len()).filter(|&i| accept[i]).collect();

    let fitted: Vec<(ScanPoint, Vec<f64>)> = grid
        .par_iter()
        .enumerate()
        .map(|(g, params)| {
            let energies: Vec<f64> = kept
                .iter()
                .map(|&i| replayed[i].1[g])
                .filter(|e| !e.is_nan())
                .collect();
            // A point is only meaningful if (essentially) every event supports it.
            let outcome = if 100 * energies.len() < 99 * kept.len() {
                let err = kept.iter().find_map(|&i| {
                    params
                        .check_window(events[i].samples.len(), events[i].trigger)
                        .err()
                });
                match err {
                    Some(e) => PointOutcome::Window(e),
                    None => PointOutcome::NoPeak,
                }
            } else {
                fit_peak(&energies).map_or(PointOutcome::NoPeak, PointOutcome::Fit)
            };
            let point = ScanPoint {
                params: *params,
                outcome,
                events: energies.len(),
                content: None,
            };
            (point, energies)
        })
        .collect();

    let content_half_width =
        content_reference(fitted.iter().map(|(p, _)| p)).map(|w| CONTENT_WINDOW_FWHMS * w);
    let points = fitted
        .into_iter()
        .map(|(mut point, energies)| {
            if let (PointOutcome::Fit(f), Some(w)) = (point.outcome, content_half_width) {
                let half = w * f.centroid.abs();
                let inside = energies
                    .iter()
                    .filter(|&&e| (e - f.centroid).abs() <= half)
                    .count();
                point.content =
                    (!energies.is_empty()).then(|| inside as f64 / energies.len() as f64);
            }
            point
        })
        .collect();

    ScanReport {
        points,
        events_total: events.len(),
        events_tilt_rejected: events.len() - kept.len(),
        content_half_width,
    }
}

/// Narrowest relative FWHM among fitted points whose own fit holds at least
/// [`CONTENT_REFERENCE_MIN_FIT_FRACTION`] of the events (falling back to all
/// fitted points if none does).
fn content_reference<'a>(points: impl Iterator<Item = &'a ScanPoint> + Clone) -> Option<f64> {
    let narrowest = |credible_only: bool| {
        points
            .clone()
            .filter(|p| {
                !credible_only
                    || p.fit_fraction().unwrap_or(0.0) >= CONTENT_REFERENCE_MIN_FIT_FRACTION
            })
            .filter_map(ScanPoint::rel_fwhm)
            .min_by(f64::total_cmp)
    };
    narrowest(true).or_else(|| narrowest(false))
}

/// Measure the preamp decay constant τ (in samples) from the recorded pulses
/// (TODO 59 §4.3: the pole-zero is measured, not searched). Averages the
/// pedestal-subtracted, trigger-aligned tails of the pile-up-free events and
/// fits `ln(v)` linearly. `None` when the tail is too short or not decaying.
pub fn measure_decay_tau(events: &[ScanEvent]) -> Option<f64> {
    let preps: Vec<Prepared> = events
        .par_iter()
        .map(|ev| Prepared::new(&ev.samples, ev.trigger))
        .collect();
    let tilts: Vec<f64> = preps.iter().map(Prepared::pedestal_tilt).collect();
    let accept = tilt_accept(&tilts);
    let used: Vec<&Prepared> = preps
        .iter()
        .zip(&accept)
        .filter(|(_, &a)| a)
        .map(|(p, _)| p)
        .collect();
    let tail_len = used
        .iter()
        .map(|p| p.len().saturating_sub(p.trigger()))
        .min()?;
    if tail_len < TAIL_START + 64 {
        return None;
    }

    // Least squares of ln(mean tail) against the sample offset.
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for j in TAIL_START..tail_len {
        let mean = used.iter().map(|p| p.sample(p.trigger() + j)).sum::<f64>() / used.len() as f64;
        if mean <= 0.0 {
            return None;
        }
        let (x, y) = (j as f64, mean.ln());
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
        n += 1.0;
    }
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    (slope.is_finite() && slope < 0.0).then(|| -1.0 / slope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::testutil::{noisy_event, rng};
    use rand::Rng;

    const LEN: usize = 4000;
    const T0: usize = 1500;
    const TAU: f64 = 12500.0;

    fn base() -> TrapParams {
        TrapParams::from_ns(4000.0, 1000.0, TAU * 4.0, 80.0, 1, 256, 4.0)
    }

    /// `n` events of fixed amplitude; charge collection time drawn from
    /// `collect` (samples), white noise `sigma`.
    fn events(
        seed: u64,
        n: usize,
        trigger: usize,
        collect: std::ops::RangeInclusive<usize>,
        sigma: f64,
    ) -> Vec<ScanEvent> {
        let mut g = rng(seed);
        (0..n)
            .map(|_| {
                let c = g.gen_range(collect.clone());
                ScanEvent {
                    samples: noisy_event(&mut g, LEN, trigger, TAU, 1970.0, -8104.0, c, sigma),
                    trigger,
                }
            })
            .collect()
    }

    fn fwhm(point: &ScanPoint) -> f64 {
        match point.outcome {
            PointOutcome::Fit(f) => f.fwhm,
            other => panic!("expected a fit, got {other:?}"),
        }
    }

    #[test]
    fn grid_is_the_cartesian_product() {
        let g = grid(&base(), &[250, 500], &[100, 200, 300]);
        assert_eq!(g.len(), 6);
        assert_eq!((g[0].rise, g[0].flat), (250, 100));
        assert_eq!((g[5].rise, g[5].flat), (500, 300));
        assert!(g
            .iter()
            .all(|p| p.pz_multiplier == base().pz_multiplier && p.peak_pct == 80.0));
    }

    #[test]
    fn white_noise_width_falls_as_one_over_sqrt_rise() {
        // Pure series (white) noise: FWHM ∝ √(2/k). Rise 300 → 1200 samples halves it.
        let ev = events(1, 3000, T0, 0..=0, 2.0);
        let report = scan(&ev, &grid(&base(), &[300, 1200], &[100]));
        let ratio = fwhm(&report.points[0]) / fwhm(&report.points[1]);
        assert!(
            (1.7..2.3).contains(&ratio),
            "FWHM(300)/FWHM(1200) = {ratio:.2}, expected ≈ 2"
        );
        // Absolute scale: FWHM = 2.355 · σ·√(2/k) ADC counts.
        let want = 2.354_82 * 2.0 * (2.0f64 / 1200.0).sqrt();
        assert!(
            (fwhm(&report.points[1]) / want - 1.0).abs() < 0.15,
            "FWHM(1200) = {} vs {want}",
            fwhm(&report.points[1])
        );
        assert_eq!(report.best().map(|p| p.params.rise), Some(1200));
    }

    #[test]
    fn flat_top_shorter_than_charge_collection_is_penalised() {
        // Collection time varies 8…100 samples. A 16-sample flat top samples the
        // energy before the charge is in (ballistic deficit): only the fastest
        // ~6 % of the pulses stay in the peak — and they form a NARROW one, so
        // ranking by core width alone would pick the wrong setting.
        let ev = events(2, 1500, T0, 8..=100, 0.3);
        let report = scan(&ev, &grid(&base(), &[600], &[16, 200]));
        let (short, long) = (&report.points[0], &report.points[1]);
        assert!(
            long.peak_fraction().expect("fit") > 0.9,
            "{:?}",
            long.peak_fraction()
        );
        if let Some(f) = short.peak_fraction() {
            assert!(
                f < 0.3,
                "short flat top keeps {f} of the events in its peak"
            );
        }
        assert!(!report.keeps_peak(short) && report.keeps_peak(long));
        assert_eq!(report.best().map(|p| p.params.flat), Some(200));
    }

    #[test]
    fn pickup_broadened_points_do_not_raise_the_eligibility_bar() {
        // ELIADE SN01 run 22: periodic pickup. A rise that is a multiple of the
        // pickup period nulls it (narrow line); other rises pass it and the line
        // becomes a broad bell whose own wide fit also swallows the low-energy
        // tail — so its "fraction in the peak" is the HIGHEST on the grid. That
        // must not make the clean, narrow point look like it loses events.
        let period = 450.0;
        let mut g = rng(6);
        let ev: Vec<ScanEvent> = (0..2500)
            .map(|i| {
                // 15 % low-side shoulder a few line widths below the peak
                // (incomplete charge collection / Compton under the line).
                let amp = if i % 7 == 0 {
                    g.gen_range(1955.0..1966.0)
                } else {
                    1970.0
                };
                let phase = g.gen_range(0.0..std::f64::consts::TAU);
                let mut s = noisy_event(&mut g, LEN, T0, TAU, amp, -8104.0, 0, 20.0);
                for (n, x) in s.iter_mut().enumerate() {
                    let w = std::f64::consts::TAU * n as f64 / period + phase;
                    *x += (12.0 * w.sin()).round() as i16;
                }
                ScanEvent {
                    samples: s,
                    trigger: T0,
                }
            })
            .collect();
        let report = scan(&ev, &grid(&base(), &[450, 675], &[100]));
        let (nulled, passed) = (&report.points[0], &report.points[1]);
        assert!(
            fwhm(passed) > 1.4 * fwhm(nulled),
            "passed {} vs nulled {}",
            fwhm(passed),
            fwhm(nulled)
        );
        assert!(
            report.keeps_peak(nulled),
            "content nulled {:?} passed {:?}",
            nulled.peak_fraction(),
            passed.peak_fraction()
        );
        assert_eq!(report.best().map(|p| p.params.rise), Some(450));
    }

    #[test]
    fn unsupported_points_are_reported_not_faked() {
        // es2 July geometry: pre-trigger 252 samples. rise 100 fits, rise 1250 does not.
        let ev = events(3, 600, 252, 0..=0, 1.0);
        let report = scan(&ev, &grid(&base(), &[100, 1250], &[250]));
        assert!(matches!(report.points[0].outcome, PointOutcome::Fit(_)));
        assert_eq!(
            report.points[1].outcome,
            PointOutcome::Window(WindowError::PreTriggerTooShort {
                need: 1299,
                have: 252
            })
        );
        assert_eq!(report.best().map(|p| p.params.rise), Some(100));
    }

    #[test]
    fn pileup_tail_events_are_excluded_and_counted() {
        let clean = events(4, 1200, T0, 0..=0, 1.0);
        let reference = fwhm(&scan(&clean, &grid(&base(), &[800], &[100])).points[0]);

        // Put every 8th event on the decaying tail of an earlier pulse.
        let mut mixed = clean.clone();
        for ev in mixed.iter_mut().step_by(8) {
            for (i, x) in ev.samples.iter_mut().enumerate() {
                *x += (400.0 * (-(i as f64) / TAU).exp()).round() as i16;
            }
        }
        let report = scan(&mixed, &grid(&base(), &[800], &[100]));
        assert_eq!(report.events_total, 1200);
        assert_eq!(report.events_tilt_rejected, 150);
        assert!((fwhm(&report.points[0]) / reference - 1.0).abs() < 0.10);
    }

    #[test]
    fn decay_constant_is_measured_from_the_tail() {
        let ev = events(5, 300, T0, 10..=60, 1.0);
        let tau = measure_decay_tau(&ev).expect("tau");
        assert!((tau / TAU - 1.0).abs() < 0.02, "tau = {tau}, want {TAU}");
    }
}

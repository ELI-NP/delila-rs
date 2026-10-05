//! FW-side trapezoid scan (TODO 59 §8) — the hardware-free half of
//! `pha_fw_scan`.
//!
//! The SW replay ([`super::scan`]) can only explore rise times that fit in the
//! recorded pre-trigger, ≤ 4 µs on a V1725 (§5.7), and the run 22 segments
//! were still improving there. The FW scan has no such limit: it re-programs
//! the digitizer, takes a short list-mode run per grid point and measures the
//! FW energy resolution directly. Everything here is pure:
//!
//! - [`grid`] snaps the requested rise × flat-top grid to the FW step;
//! - [`set_board_trap`] / [`with_channel_traps`] edit a board config;
//! - [`find_co60`] finds the ⁶⁰Co pair in a FW spectrum whose gain is not
//!   known in advance (and may change between points) — by the energy RATIO of
//!   the two lines, never by "the tallest line": 1173 keV is often the taller
//!   one, and noise triggers crowd the bottom codes (§5.7 item 7);
//! - [`measure_channel`] fits both lines; [`best_point`] ranks the grid.

use std::collections::BTreeMap;

use super::peak::{fit_peak, PeakFit};
use super::scan::MIN_RELATIVE_PEAK_FRACTION;
use crate::config::DigitizerConfig;

/// ⁶⁰Co γ-ray energies (keV).
pub const CO60_LOW_KEV: f64 = 1173.228;
pub const CO60_HIGH_KEV: f64 = 1332.492;

/// Lowest FW code considered a γ line (noise triggers sit at E ≈ 1..20).
const MIN_CODE: usize = 64;
/// Codes at and above this are overflow / saturation markers.
const MAX_CODE: usize = 0x7FFF;
/// Half width of the line-finding density window, relative to the code.
/// A ⁶⁰Co line on an HPGe is ~0.25 % FWHM, so this holds most of a line.
const DENSITY_HALF_WIDTH: f64 = 0.0025;
/// …but never narrower than this many codes (low-gain spectra).
const DENSITY_MIN_HALF_CODES: f64 = 2.0;
/// Candidate lines must be at least this far apart (relative).
const CANDIDATE_SEPARATION: f64 = 0.02;
/// How many candidate lines are searched for the ⁶⁰Co pair.
const CANDIDATES: usize = 8;
/// Accepted deviation of the measured 1332/1173 ratio from the true one.
/// The FW energy has no offset to speak of, so 1 % is generous; the nearest
/// impostor, the two Compton edges (1118/963 keV), is 2.2 % off.
const PAIR_RATIO_TOLERANCE: f64 = 0.01;
/// Events within ± this fraction of a line's code are handed to the fit.
/// For 1173 keV this stays clear of the 1332 keV Compton edge (1118 keV).
const FIT_WINDOW: f64 = 0.04;

/// FW parameter step for a sampling period: the DPP-PHA trapezoid times move in
/// 4-sample steps (DevTree `increment`: 8 ns on x730, 16 ns on x725).
pub fn trap_step_ns(sample_ns: f64) -> u32 {
    (4.0 * sample_ns).round().max(1.0) as u32
}

/// Round `ns` to the nearest positive multiple of `step`.
pub fn snap_ns(ns: f64, step: u32) -> u32 {
    let step = step.max(1);
    ((ns / step as f64).round().max(1.0) as u32) * step
}

/// The rise × flat-top grid in FW units, rise-major, duplicates (after
/// snapping) dropped.
pub fn grid(rises_ns: &[f64], flats_ns: &[f64], step: u32) -> Vec<(u32, u32)> {
    let mut points = Vec::new();
    for &r in rises_ns {
        for &f in flats_ns {
            let p = (snap_ns(r, step), snap_ns(f, step));
            if !points.contains(&p) {
                points.push(p);
            }
        }
    }
    points
}

/// Program one trapezoid on every channel of a board: the defaults, plus any
/// per-channel override that sets its own value (it would win otherwise).
pub fn set_board_trap(config: &mut DigitizerConfig, rise_ns: u32, flat_ns: u32) {
    config.channel_defaults.trap_rise_time_ns = Some(rise_ns);
    config.channel_defaults.trap_flat_top_ns = Some(flat_ns);
    for ch in config.channel_overrides.values_mut() {
        if ch.trap_rise_time_ns.is_some() {
            ch.trap_rise_time_ns = Some(rise_ns);
        }
        if ch.trap_flat_top_ns.is_some() {
            ch.trap_flat_top_ns = Some(flat_ns);
        }
    }
}

/// `original` with a per-channel trapezoid override for every channel in
/// `picks` (`channel → (rise, flat)`); all other settings untouched.
pub fn with_channel_traps(
    original: &DigitizerConfig,
    picks: &BTreeMap<u8, (u32, u32)>,
) -> DigitizerConfig {
    let mut config = original.clone();
    for (&ch, &(rise, flat)) in picks {
        let entry = config.channel_overrides.entry(ch).or_default();
        entry.trap_rise_time_ns = Some(rise);
        entry.trap_flat_top_ns = Some(flat);
    }
    config
}

/// Line candidates, densest first: `(code, events within ±DENSITY_HALF_WIDTH)`,
/// each at least [`CANDIDATE_SEPARATION`] from a denser one.
pub fn line_candidates(energies: &[u16], n: usize) -> Vec<(f64, u32)> {
    let mut prefix = vec![0u32; MAX_CODE + 1];
    for &e in energies {
        let e = e as usize;
        if (MIN_CODE..MAX_CODE).contains(&e) {
            prefix[e + 1] += 1;
        }
    }
    for i in 1..prefix.len() {
        prefix[i] += prefix[i - 1];
    }
    let in_range = |lo: f64, hi: f64| {
        let lo = (lo.ceil().max(0.0) as usize).min(MAX_CODE);
        let hi = ((hi.floor() + 1.0).max(0.0) as usize).min(MAX_CODE);
        prefix[hi].saturating_sub(prefix[lo])
    };
    let mut density: Vec<(usize, u32)> = (MIN_CODE..MAX_CODE)
        .map(|c| {
            let half = (c as f64 * DENSITY_HALF_WIDTH).max(DENSITY_MIN_HALF_CODES);
            (c, in_range(c as f64 - half, c as f64 + half))
        })
        .filter(|&(_, d)| d > 0)
        .collect();
    // Densest first; ties to the lower code so the result is deterministic.
    density.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut lines: Vec<(f64, u32)> = Vec::new();
    for (c, d) in density {
        if lines.len() == n {
            break;
        }
        let c = c as f64;
        if lines
            .iter()
            .all(|&(l, _)| (c - l).abs() > CANDIDATE_SEPARATION * l)
        {
            lines.push((c, d));
        }
    }
    lines
}

/// FW codes of the ⁶⁰Co lines `(1173 keV, 1332 keV)`: the candidate pair whose
/// ratio matches 1332.492 / 1173.228 within 1 %, the pair with the stronger
/// weaker line if several match. `None` when no pair matches.
pub fn find_co60(energies: &[u16]) -> Option<(f64, f64)> {
    let ratio = CO60_HIGH_KEV / CO60_LOW_KEV;
    let lines = line_candidates(energies, CANDIDATES);
    let mut best: Option<(u32, (f64, f64))> = None;
    for &(a, da) in &lines {
        for &(b, db) in &lines {
            if b <= a || ((b / a) / ratio - 1.0).abs() > PAIR_RATIO_TOLERANCE {
                continue;
            }
            let score = da.min(db);
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, (a, b)));
            }
        }
    }
    best.map(|(_, pair)| pair)
}

/// Fit the line near FW code `center` (events within ±[`FIT_WINDOW`]).
pub fn fit_line(energies: &[u16], center: f64) -> Option<PeakFit> {
    let (lo, hi) = (center * (1.0 - FIT_WINDOW), center * (1.0 + FIT_WINDOW));
    let values: Vec<f64> = energies
        .iter()
        .map(|&e| e as f64)
        .filter(|&e| e >= lo && e <= hi)
        .collect();
    fit_peak(&values)
}

/// FWHM in keV, calibrating the FW scale through zero at the line itself.
pub fn fwhm_kev(fit: &PeakFit, line_kev: f64) -> f64 {
    fit.fwhm * line_kev / fit.centroid
}

/// One channel at one grid point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMeasurement {
    /// All events of the channel in the run (the trigger does not depend on
    /// the trapezoid, so this normalises the peak content between points).
    pub events: usize,
    /// The 1332 keV line (ranking line).
    pub high: Option<PeakFit>,
    /// The 1173 keV line (cross-check).
    pub low: Option<PeakFit>,
}

impl ChannelMeasurement {
    /// FWHM at 1332 keV, keV.
    pub fn fwhm_kev_high(&self) -> Option<f64> {
        self.high.map(|f| fwhm_kev(&f, CO60_HIGH_KEV))
    }

    /// FWHM at 1173 keV, keV.
    pub fn fwhm_kev_low(&self) -> Option<f64> {
        self.low.map(|f| fwhm_kev(&f, CO60_LOW_KEV))
    }

    /// 1332 keV peak content per event — what a too-short flat top
    /// (ballistic deficit) or pile-up loses.
    pub fn peak_fraction(&self) -> Option<f64> {
        match (self.high, self.events) {
            (Some(f), n) if n > 0 => Some(f.counts / n as f64),
            _ => None,
        }
    }
}

/// Find the ⁶⁰Co lines in one channel's FW energies and fit both.
pub fn measure_channel(energies: &[u16]) -> ChannelMeasurement {
    let (low, high) = match find_co60(energies) {
        Some((l, h)) => (fit_line(energies, l), fit_line(energies, h)),
        None => (None, None),
    };
    ChannelMeasurement {
        events: energies.len(),
        high,
        low,
    }
}

/// Index of the best point: the narrowest 1332 keV line among points that keep
/// at least [`MIN_RELATIVE_PEAK_FRACTION`] of the largest peak content —
/// FWHM alone rewards ballistic deficit (§5.6), as in the SW scan.
pub fn best_point(points: &[ChannelMeasurement]) -> Option<usize> {
    let max_fraction = points
        .iter()
        .filter_map(ChannelMeasurement::peak_fraction)
        .fold(None, |m: Option<f64>, f| Some(m.map_or(f, |m| m.max(f))))?;
    points
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.peak_fraction()
                .is_some_and(|f| f >= MIN_RELATIVE_PEAK_FRACTION * max_fraction)
        })
        .filter_map(|(i, m)| m.fwhm_kev_high().map(|w| (i, w)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Whether a point keeps enough of the peak to be ranked (see [`best_point`]).
pub fn keeps_peak(point: &ChannelMeasurement, all: &[ChannelMeasurement]) -> bool {
    let max_fraction = all
        .iter()
        .filter_map(ChannelMeasurement::peak_fraction)
        .fold(0.0f64, f64::max);
    point
        .peak_fraction()
        .is_some_and(|f| f >= MIN_RELATIVE_PEAK_FRACTION * max_fraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelConfig, FirmwareType};
    use crate::offline::testutil::rng;
    use rand::Rng;
    use rand_distr::{Distribution, Normal};

    /// A ⁶⁰Co FW spectrum: both lines at `gain` codes/keV with FWHM
    /// `fwhm_kev`, a falling Compton continuum, and a pile of noise triggers
    /// at E ≈ 1..20. `n_high` events in the 1332 keV line; the 1173 line is
    /// 1.2 × taller (as on SN01).
    fn co60_spectrum(seed: u64, gain: f64, fwhm_kev: f64, n_high: usize) -> Vec<u16> {
        let mut r = rng(seed);
        let sigma = fwhm_kev / 2.354_820_045 * gain;
        let mut out = Vec::new();
        for (kev, n) in [(CO60_HIGH_KEV, n_high), (CO60_LOW_KEV, n_high * 6 / 5)] {
            let g = Normal::new(kev * gain, sigma).expect("sigma");
            out.extend((0..n).map(|_| g.sample(&mut r).round() as u16));
        }
        // Compton continuum up to the 1332 edge (1118 keV), 1/E-ish.
        for _ in 0..n_high * 6 {
            let kev: f64 = 1118.0 * r.gen::<f64>().powf(2.0);
            out.push((kev * gain).round() as u16);
        }
        // Noise triggers, far more numerous than any line.
        out.extend((0..n_high * 10).map(|_| r.gen_range(1..20u16)));
        out
    }

    #[test]
    fn trap_step_is_four_samples() {
        assert_eq!(trap_step_ns(4.0), 16); // V1725
        assert_eq!(trap_step_ns(2.0), 8); // x730
    }

    #[test]
    fn grid_snaps_to_the_fw_step_and_drops_duplicates() {
        let g = grid(&[3300.0, 3296.0, 6590.0], &[1200.0], 16);
        assert_eq!(g, vec![(3296, 1200), (6592, 1200)]);
        assert_eq!(snap_ns(1.0, 16), 16, "never zero");
        let g = grid(&[2000.0, 4000.0], &[800.0, 1200.0], 16);
        assert_eq!(
            g,
            vec![(2000, 800), (2000, 1200), (4000, 800), (4000, 1200)]
        );
    }

    #[test]
    fn set_board_trap_reaches_overriding_channels_only_where_they_override() {
        let mut cfg = DigitizerConfig::new(0, "test", FirmwareType::PHA1);
        cfg.channel_defaults.trap_rise_time_ns = Some(3008);
        cfg.channel_defaults.trap_flat_top_ns = Some(1008);
        cfg.channel_defaults.trigger_threshold = Some(70);
        cfg.channel_overrides.insert(
            3,
            ChannelConfig {
                trap_rise_time_ns: Some(5000),
                trigger_threshold: Some(200),
                ..Default::default()
            },
        );
        cfg.channel_overrides.insert(
            5,
            ChannelConfig {
                trigger_threshold: Some(90),
                ..Default::default()
            },
        );
        set_board_trap(&mut cfg, 6592, 1200);
        assert_eq!(cfg.channel_defaults.trap_rise_time_ns, Some(6592));
        assert_eq!(cfg.channel_defaults.trap_flat_top_ns, Some(1200));
        assert_eq!(cfg.channel_defaults.trigger_threshold, Some(70));
        let ch3 = &cfg.channel_overrides[&3];
        assert_eq!(ch3.trap_rise_time_ns, Some(6592), "override must follow");
        assert_eq!(ch3.trap_flat_top_ns, None, "no new override created");
        assert_eq!(ch3.trigger_threshold, Some(200));
        assert_eq!(cfg.channel_overrides[&5].trap_rise_time_ns, None);
    }

    #[test]
    fn with_channel_traps_adds_overrides_and_keeps_the_rest() {
        let mut cfg = DigitizerConfig::new(0, "test", FirmwareType::PHA1);
        cfg.channel_defaults.trap_rise_time_ns = Some(3296);
        cfg.channel_overrides.insert(
            2,
            ChannelConfig {
                trigger_threshold: Some(55),
                ..Default::default()
            },
        );
        let picks = BTreeMap::from([(2u8, (6592u32, 1200u32)), (7, (4000, 800))]);
        let out = with_channel_traps(&cfg, &picks);
        assert_eq!(out.channel_defaults.trap_rise_time_ns, Some(3296));
        assert_eq!(out.channel_overrides[&2].trap_rise_time_ns, Some(6592));
        assert_eq!(out.channel_overrides[&2].trigger_threshold, Some(55));
        assert_eq!(out.channel_overrides[&7].trap_flat_top_ns, Some(800));
        assert!(!out.channel_overrides.contains_key(&0));
    }

    #[test]
    fn finds_the_co60_pair_at_unknown_gain_despite_taller_1173_and_noise() {
        for (seed, gain) in [(1, 2.5), (2, 4.0), (3, 6.03), (4, 9.0)] {
            let e = co60_spectrum(seed, gain, 3.0, 3000);
            let (lo, hi) = find_co60(&e).expect("pair");
            assert!(
                (lo / (CO60_LOW_KEV * gain) - 1.0).abs() < 0.002,
                "gain {gain}: 1173 at {lo}"
            );
            assert!(
                (hi / (CO60_HIGH_KEV * gain) - 1.0).abs() < 0.002,
                "gain {gain}: 1332 at {hi}"
            );
        }
    }

    #[test]
    fn a_single_line_is_not_a_co60_pair() {
        let mut r = rng(9);
        let g = Normal::new(5000.0f64, 6.0).expect("sigma");
        let e: Vec<u16> = (0..5000).map(|_| g.sample(&mut r).round() as u16).collect();
        assert_eq!(find_co60(&e), None);
        assert_eq!(find_co60(&[]), None);
    }

    #[test]
    fn measure_channel_recovers_the_true_resolution() {
        let gain = 6.03;
        let e = co60_spectrum(11, gain, 3.0, 6000);
        let m = measure_channel(&e);
        let w = m.fwhm_kev_high().expect("1332 fit");
        assert!((w / 3.0 - 1.0).abs() < 0.06, "FWHM@1332 = {w}");
        let w = m.fwhm_kev_low().expect("1173 fit");
        assert!((w / 3.0 - 1.0).abs() < 0.06, "FWHM@1173 = {w}");
        let f = m.peak_fraction().expect("fraction");
        let truth = 6000.0 / e.len() as f64;
        assert!((f / truth - 1.0).abs() < 0.05, "fraction {f} vs {truth}");
    }

    fn point(events: usize, counts: f64, fwhm_codes: f64) -> ChannelMeasurement {
        let centroid = CO60_HIGH_KEV * 6.0;
        ChannelMeasurement {
            events,
            high: Some(PeakFit {
                centroid,
                fwhm: fwhm_codes,
                counts,
            }),
            low: None,
        }
    }

    #[test]
    fn best_point_skips_narrow_points_that_lost_the_peak() {
        let points = [
            point(100_000, 5000.0, 20.0),
            point(100_000, 4000.0, 15.0), // narrowest, but lost 20 % (ballistic deficit)
            point(100_000, 4990.0, 18.0), // best eligible
            ChannelMeasurement {
                events: 100_000,
                high: None,
                low: None,
            },
        ];
        assert_eq!(best_point(&points), Some(2));
        assert!(!keeps_peak(&points[1], &points));
        assert!(keeps_peak(&points[2], &points));
        assert!(!keeps_peak(&points[3], &points));
        assert_eq!(best_point(&points[3..]), None);
    }
}

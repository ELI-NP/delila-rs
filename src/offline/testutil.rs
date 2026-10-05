//! Synthetic waveforms shared by the offline-module tests.

use rand::rngs::StdRng;
use rand::SeedableRng;
use rand_distr::{Distribution, Normal};

/// Deterministic RNG for reproducible tests.
pub fn rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

/// One digitized preamp record: pedestal `ped`, a pulse of amplitude `amp` whose
/// charge is collected linearly over `collect` samples starting at `t0`
/// (`collect == 0` = instantaneous step), exponential decay `tau` (samples),
/// plus white Gaussian noise `sigma` (ADC counts), rounded to `i16`.
#[allow(clippy::too_many_arguments)]
pub fn noisy_event(
    rng: &mut StdRng,
    len: usize,
    t0: usize,
    tau: f64,
    amp: f64,
    ped: f64,
    collect: usize,
    sigma: f64,
) -> Vec<i16> {
    let noise = Normal::new(0.0, sigma.max(f64::MIN_POSITIVE)).expect("valid sigma");
    let decay = (-1.0 / tau).exp();
    let n_collect = collect.max(1);
    let mut v = 0.0f64;
    (0..len)
        .map(|n| {
            v *= decay;
            if n >= t0 && n < t0 + n_collect {
                v += amp / n_collect as f64;
            }
            let x = ped + v + if sigma > 0.0 { noise.sample(rng) } else { 0.0 };
            x.round() as i16
        })
        .collect()
}

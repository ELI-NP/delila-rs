// hits_gen.hpp — synthetic DAQ-like hit stream for ROOT write benchmarks.
// Plain random test data (NOT a detector simulation). Deterministic per seed.
//
// Hit layout mirrors delila-rs root_sink: module/channel (UChar_t), energy /
// energy_short (UShort_t), timestamp_ns (Double_t) -> 14 bytes raw payload.
#pragma once
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <random>
#include <vector>

struct Hit {
  uint8_t module;
  uint8_t channel;
  uint16_t energy;
  uint16_t energy_short;
  double timestamp_ns;
};

struct GenConfig {
  std::size_t n = 30'000'000;
  double rate_hz = 1.0e6;   // aggregate rate -> Poisson gaps, mean 1/rate
  int n_modules = 10;
  int ch_per_module = 15;   // 150 channels
  uint64_t seed = 20261103;
  // "board order": emulate a merger that forwards per-board batches without a
  // global time sort — within a window of `window_ns` hits are grouped by
  // module (time-ordered inside each module, NOT across modules).
  bool board_order = false;
  double window_ns = 10.0e6;  // 10 ms
};

inline std::vector<Hit> generate_hits(const GenConfig& cfg) {
  std::mt19937_64 rng(cfg.seed);
  const int nch = cfg.n_modules * cfg.ch_per_module;

  // Uneven channel rates: log-normal spread + a few hot channels.
  std::vector<double> w(nch);
  std::lognormal_distribution<double> ln(0.0, 0.8);
  for (int i = 0; i < nch; ++i) w[i] = ln(rng);
  for (int i = 0; i < nch; i += 37) w[i] *= 6.0;  // hot channels
  std::discrete_distribution<int> pick(w.begin(), w.end());

  // Per-channel energy model: a Gaussian peak + exponential continuum below it.
  struct ChE { double peak, sigma, frac_peak, slope, short_ratio; };
  std::vector<ChE> che(nch);
  std::uniform_real_distribution<double> u01(0, 1);
  for (int i = 0; i < nch; ++i) {
    che[i].peak = 2000 + 10000 * u01(rng);
    che[i].sigma = che[i].peak * (0.01 + 0.03 * u01(rng));
    che[i].frac_peak = 0.2 + 0.4 * u01(rng);
    che[i].slope = che[i].peak * (0.3 + 0.5 * u01(rng));
    che[i].short_ratio = 0.2 + 0.3 * u01(rng);
  }

  std::exponential_distribution<double> gap(cfg.rate_hz / 1.0e9);  // per ns
  std::normal_distribution<double> gauss(0, 1);
  std::uniform_int_distribution<int> fine(0, 1023);

  std::vector<Hit> hits;
  hits.reserve(cfg.n);
  double t = 1.0e9;  // start at 1 s
  for (std::size_t k = 0; k < cfg.n; ++k) {
    t += gap(rng);
    const int ch = pick(rng);
    const ChE& e = che[ch];
    double en;
    if (u01(rng) < e.frac_peak)
      en = e.peak + e.sigma * gauss(rng);
    else
      en = -e.slope * std::log(1.0 - u01(rng));
    if (en < 0) en = 0;
    if (en > 65000) en = 65000;
    const uint16_t ei = static_cast<uint16_t>(en);
    double es = ei * (e.short_ratio + 0.02 * gauss(rng));
    if (es < 0) es = 0;
    // Timestamp: 2 ns coarse grid + 10-bit fine timestamp (CAEN-like), so the
    // double has structured but non-trivial mantissa bits.
    const double ts = std::floor(t / 2.0) * 2.0 + fine(rng) / 512.0;
    Hit h;
    h.module = static_cast<uint8_t>(ch / cfg.ch_per_module);
    h.channel = static_cast<uint8_t>(ch % cfg.ch_per_module);
    h.energy = ei;
    h.energy_short = static_cast<uint16_t>(es);
    h.timestamp_ns = ts;
    hits.push_back(h);
  }
  // Fine timestamps may invert neighbours on the same 2 ns tick: make it a
  // true sorted stream (root_sink's Sorter output is fully sorted).
  std::sort(hits.begin(), hits.end(),
            [](const Hit& a, const Hit& b) { return a.timestamp_ns < b.timestamp_ns; });

  if (cfg.board_order) {
    std::size_t i = 0;
    while (i < hits.size()) {
      const double t0 = hits[i].timestamp_ns;
      std::size_t j = i;
      while (j < hits.size() && hits[j].timestamp_ns < t0 + cfg.window_ns) ++j;
      std::stable_sort(hits.begin() + i, hits.begin() + j,
                       [](const Hit& a, const Hit& b) { return a.module < b.module; });
      i = j;
    }
  }
  return hits;
}

// Count adjacent timestamp inversions (0 == perfectly time ordered).
inline std::size_t count_inversions(const std::vector<double>& ts) {
  std::size_t n = 0;
  for (std::size_t i = 1; i < ts.size(); ++i)
    if (ts[i] < ts[i - 1]) ++n;
  return n;
}

// cfd_emu.hpp — software model of the V1730 / V1725 DPP-PSD digital CFD, for
// scanning CFD parameters offline on recorded waveforms (TODO 90).
//
// FW reference: UM4380 rev6 (725-730 DPP-PSD registers, legacy/, local only):
//   p.14 CFD settings 0x1n3C: [7:0] delay in samples (2 ns on 730), [9:8]
//        fraction 25/50/75/100 %, [11:10] interpolation points = the n-th
//        sample before and after the zero crossing.
//   p.29 fine time stamp (10 bits, EXTRAS option 010); option 101 gives the CFD
//        values at the zero crossing instead (DELILA forces 010, see handle.rs).
//   p.30 input smoothing 0x1n84 [15:12]: "number of averaged samples" 1 (off),
//        2, 4, 8, 16; the smoothed input feeds the CFD and the LED trigger.
//   p.38 analog probe: Trace 2 = CFD (only in CFD discrimination mode).
// What the manual does not pin down is a MODEL CHOICE here, checked against the
// FW in TODO 90 P2 (CFD probe overlay + per-event fine time) before any scan
// result is used:
//   - smoothing = causal moving average of n samples (not exponential),
//   - c[i] = f * s[i] - s[i - D] on the baseline-subtracted, polarity-corrected
//     input; the timing crossing is c going from > 0 to <= 0 (the PSD1 decoder's
//     SW fine time uses the same orientation: before > mid > after),
//   - arming: the search starts at the first sample with s >= threshold,
//   - baseline = mean of the first n samples of the record.
//
// Pure functions, no ROOT. Tests: test_cfd_emu.cpp.
// License: BSD-3-Clause (same as delila-rs).

#pragma once

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <vector>

namespace cfdtune {

enum class Polarity { Positive, Negative };

/// One CFD setting, in the FW's own units.
struct CfdParams {
  int delay = 1;          // samples (UM4380 p.14 [7:0])
  int fraction_pct = 50;  // 25, 50, 75 or 100 (p.14 [9:8])
  int smooth = 1;         // averaged samples: 1 (off), 2, 4, 8, 16 (p.30)
  int interp = 0;         // 0..3: the (interp+1)-th sample before/after (p.14 [11:10])
};

/// Where the CFD crosses zero, in samples from the start of the record.
struct Crossing {
  bool found = false;
  double pos = 0.0;  // fractional sample position
  int after = -1;    // first sample with c <= 0

  double time_ns(double ns_per_sample) const { return pos * ns_per_sample; }
  /// The FW's 10-bit fine time: fraction of a sample after the sample before
  /// the crossing (UM4380 p.29).
  int fine10() const {
    double f = pos - std::floor(pos);
    int v = static_cast<int>(std::floor(f * 1024.0));
    return std::clamp(v, 0, 1023);
  }
};

/// Difference of two fractions of a sample, wrapped to (-0.5, 0.5].
inline double frac_diff(double a, double b) {
  double d = a - b;
  return d - std::ceil(d - 0.5);
}

/// Baseline-subtracted input with the pulse made positive-going. The baseline
/// is the mean of the first `n_baseline` samples (fewer if the record is short).
inline std::vector<double> prepare(const std::vector<short>& adc, Polarity pol, int n_baseline) {
  std::vector<double> s(adc.size());
  if (adc.empty()) return s;
  size_t nb = std::min<size_t>(adc.size(), static_cast<size_t>(std::max(1, n_baseline)));
  double base = 0.0;
  for (size_t i = 0; i < nb; ++i) base += adc[i];
  base /= static_cast<double>(nb);
  const double sign = pol == Polarity::Positive ? 1.0 : -1.0;
  for (size_t i = 0; i < adc.size(); ++i) s[i] = sign * (adc[i] - base);
  return s;
}

/// Causal moving average of `n` samples; before the record starts the first
/// sample is repeated. n <= 1 returns the input unchanged.
inline std::vector<double> moving_average(const std::vector<double>& s, int n) {
  if (n <= 1 || s.empty()) return s;
  std::vector<double> out(s.size());
  double sum = s[0] * n;  // window filled with s[0]
  for (size_t i = 0; i < s.size(); ++i) {
    long j = static_cast<long>(i) - n;  // sample leaving the window
    sum += s[i] - (j >= 0 ? s[static_cast<size_t>(j)] : s[0]);
    out[i] = sum / n;
  }
  return out;
}

/// Zero crossing of c[i] = f*s[i] - s[i-D] (s already smoothed), searched from
/// the first sample at or after `from` with s >= arm_threshold. Linear
/// interpolation between the (interp+1)-th samples before and after the
/// crossing. `from` > 0 skips noise above the threshold before the FW trigger
/// (noisy ThGEM records, pf-qaq run 17); c still looks back D samples before it.
inline Crossing cfd_crossing(const std::vector<double>& s, const CfdParams& p, double arm_threshold, long from = 0) {
  Crossing out;
  const long n = static_cast<long>(s.size());
  if (n < 2 || p.delay < 1) return out;
  const double f = p.fraction_pct / 100.0;
  auto c = [&](long i) { return f * s[i] - (i - p.delay >= 0 ? s[i - p.delay] : 0.0); };

  long arm = -1;
  for (long i = std::max(0L, from); i < n; ++i)
    if (s[i] >= arm_threshold) {
      arm = i;
      break;
    }
  if (arm < 0) return out;

  for (long i = std::max(1L, arm); i < n; ++i) {
    if (c(i - 1) > 0.0 && c(i) <= 0.0) {
      long b = i - 1 - p.interp, a = i + p.interp;
      if (b < 0 || a >= n) return out;
      double cb = c(b), ca = c(a);
      if (cb == ca) return out;
      out.found = true;
      out.after = static_cast<int>(i);
      out.pos = b + cb / (cb - ca) * static_cast<double>(a - b);
      return out;
    }
  }
  return out;
}

}  // namespace cfdtune

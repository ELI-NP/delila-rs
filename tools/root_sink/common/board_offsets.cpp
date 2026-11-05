// board_offsets.cpp — offset of each board from a fixed reference channel, with
// one pulser moved from board to board during a run (TODO 77 §6.8).
//
//   g++ -O2 -std=c++17 board_offsets.cpp -o board_offsets      (no ROOT needed)
//   ./board_offsets 0 0 15 /data/N_Wall/run0005_*.delila
//      (reference module, reference channel, pulser channel, files)
//
// The reference gets the pulser's TTL all the time; the pulse goes to the pulser
// channel of one board at a time. Each pulser hit is paired with the nearest
// reference hit (within 0.45 period):  dt = t(module M, pulser ch) - t(reference).
// Per board: median, RMS, slope over its window (a slope = no common clock),
// medians of 5 slices, 1 ns histogram bins, hits outside +-20 ns, pulser energy.
// Subtract the delay between the TTL and the pulse yourself (2026-11-05: 250 ns).
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <map>
#include <utility>
#include <vector>

#include "../../delila2root/TDelila.hpp"

struct Hit {
  double t;
  int e;
};

static double quantile(std::vector<double> v, double q) {
  if (v.empty()) return NAN;
  const size_t i = std::min(v.size() - 1, static_cast<size_t>(q * v.size()));
  std::nth_element(v.begin(), v.begin() + i, v.end());
  return v[i];
}
static double median(const std::vector<double>& v) { return quantile(v, 0.5); }

int main(int argc, char** argv) {
  if (argc < 5) {
    std::fprintf(stderr, "usage: %s ref_module ref_channel pulser_channel file.delila...\n", argv[0]);
    return 2;
  }
  const int ref_module = std::atoi(argv[1]), ref_channel = std::atoi(argv[2]), pulser_channel = std::atoi(argv[3]);

  std::map<std::pair<int, int>, std::vector<Hit>> hits;
  long long total = 0;
  for (int i = 4; i < argc; ++i) {
    tdelila::TDelila d(argv[i]);
    if (!d.good()) {
      std::printf("skip %s: %s\n", argv[i], d.error().c_str());
      continue;
    }
    tdelila::Event ev;
    while (d.next(ev)) {
      ++total;
      hits[{ev.module(), ev.channel()}].push_back({ev.timestamp_ns(), static_cast<int>(ev.energy())});
    }
  }
  for (auto& kv : hits)
    std::sort(kv.second.begin(), kv.second.end(), [](const Hit& a, const Hit& b) { return a.t < b.t; });

  std::printf("events %lld\n\nmodule ch      n  first_s   last_s    rate_Hz  median_E\n", total);
  for (const auto& kv : hits) {
    const auto& v = kv.second;
    std::vector<double> e;
    for (const auto& h : v) e.push_back(h.e);
    const double span = (v.back().t - v.front().t) * 1e-9;
    std::printf("m%d ch%-2d %7zu %8.3f %8.3f %10.1f %9.0f\n", kv.first.first, kv.first.second, v.size(),
                v.front().t * 1e-9, v.back().t * 1e-9, span > 0 ? v.size() / span : 0.0, median(e));
  }

  const auto ref_it = hits.find({ref_module, ref_channel});
  if (ref_it == hits.end() || ref_it->second.size() < 2) {
    std::printf("no reference hits (m%d ch%d)\n", ref_module, ref_channel);
    return 1;
  }
  std::vector<double> ref, gaps;
  for (const auto& h : ref_it->second) ref.push_back(h.t);
  for (size_t i = 1; i < ref.size(); ++i) gaps.push_back(ref[i] - ref[i - 1]);
  const double period = median(gaps);
  std::printf("\nreference m%d ch%d: %zu hits, period %.3f us, gaps > 1.5 periods: %ld\n", ref_module, ref_channel,
              ref.size(), period * 1e-3,
              static_cast<long>(std::count_if(gaps.begin(), gaps.end(), [&](double g) { return g > 1.5 * period; })));

  for (const auto& kv : hits) {
    if (kv.first.second != pulser_channel || kv.first.first == ref_module) continue;
    std::vector<double> dt, when;
    for (const auto& h : kv.second) {
      const auto it = std::lower_bound(ref.begin(), ref.end(), h.t);
      double nearest = NAN;
      if (it != ref.end()) nearest = *it;
      if (it != ref.begin() && (std::isnan(nearest) || h.t - *(it - 1) < nearest - h.t)) nearest = *(it - 1);
      if (std::fabs(h.t - nearest) > 0.45 * period) continue;
      dt.push_back(h.t - nearest);
      when.push_back(h.t * 1e-9);
    }
    std::printf("\nm%d: %zu hits, matched %zu (%.1f %%)\n", kv.first.first, kv.second.size(), dt.size(),
                100.0 * dt.size() / kv.second.size());
    if (dt.size() < 10) continue;

    const double mid = median(dt);
    std::vector<double> core, core_t;
    for (size_t i = 0; i < dt.size(); ++i) {
      if (std::fabs(dt[i] - mid) < 20) {
        core.push_back(dt[i]);
        core_t.push_back(when[i]);
      }
    }
    double sx = 0, sy = 0, sxx = 0, sxy = 0, syy = 0;
    for (size_t i = 0; i < core.size(); ++i) {
      sx += core_t[i];
      sy += core[i];
      sxx += core_t[i] * core_t[i];
      sxy += core_t[i] * core[i];
      syy += core[i] * core[i];
    }
    const double n = core.size(), mean = sy / n, den = n * sxx - sx * sx;
    std::printf("  median %.3f ns, RMS %.3f, q05 %.3f, q95 %.3f, slope %+.4f ns/s, t %.2f..%.2f s\n", mid,
                std::sqrt(std::max(0.0, syy / n - mean * mean)), quantile(core, 0.05), quantile(core, 0.95),
                den > 0 ? (n * sxy - sx * sy) / den : NAN, core_t.front(), core_t.back());
    std::printf("  slices:");
    for (size_t s = 0; s < 5; ++s) {
      const std::vector<double> slice(core.begin() + core.size() * s / 5, core.begin() + core.size() * (s + 1) / 5);
      std::printf(" %.3f", median(slice));
    }
    std::map<long, long> bins;
    for (const double x : dt) ++bins[static_cast<long>(std::floor(x))];
    std::printf("\n  1 ns bins with >= 0.5 %%:");
    for (const auto& b : bins)
      if (b.second >= 0.005 * dt.size()) std::printf(" [%ld]=%ld", b.first, b.second);
    std::vector<double> e;
    for (const auto& h : kv.second) e.push_back(h.e);
    std::printf("\n  outside +-20 ns: %zu; energy median %.0f, q05 %.0f, q95 %.0f\n", dt.size() - core.size(), median(e),
                quantile(e, 0.05), quantile(e, 0.95));
  }
  return 0;
}

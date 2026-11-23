// make_test_run_11b.C — a synthetic 11B_208Pb run as a hits tree, for
// rehearsing thgem_sink end to end without the detectors (replay it with
// ../common/test_publisher). Not physics: just the structure the display has
// to show, with known answers.
//
//   root -l -b -q 'make_test_run_11b.C+("thgem_test.root", 2.0)'
//
// Board layout = config/channel_map_11B_208Pb.txt (draft):
//   module 0 VX2730: ch0 beam (1/100 of the RF), ch1-7 LaBr3, ch8-31 LS
//   modules 1-3: ThGEM n on module 1 + (n-1)/3, channels 5*((n-1)%3) + 0..4 =
//   T, XL, XR, YU, YD; ch15 pulser
//
// What is in it (rates per second of data time) and what thgem_sink must show:
//   * beam: every 100th bunch, T = 199.95 ns (the clock has to measure it)
//   * reactions (2 kHz) on random bunches: two fragments into ThGEMs i and
//     i + 4 (i = 1..4, the "facing" pairs), triggers at bunch + 20 + u and
//     bunch + 40 - u (u uniform 0..10 ns, 0.5 ns jitter) -> dt_T<i+4>_vs_T<i>
//     is an anti-correlated line from (20, 40) to (30, 30); pair_hits fills
//     (i, i+4) only. ThGEM n images a spot at x0 = -120 + 30 n, y0 = 90 - 20 n
//     ns (sigma 5 ns); each arm is there with 95 %. 30 % of the reactions with
//     a neutron in one LS (L = 1.5 m, 0.5-5 MeV), 20 % with a prompt gamma in
//     one LaBr3 (bunch + 3 ns)
//   * ThGEM singles: 300 Hz per ThGEM, uniform over +-200 ns, random time
//   * LS singles: gamma flash 50 kHz (bunch + L/c), neutrons 20 kHz
//   * LaBr3 singles 10 kHz (bunch + 3 ns)
//   * pulser 100 Hz on modules 1-3 ch15 together
// Arms arrive 150 ns after their trigger plus or minus half the position:
// t_XL = t_T + 150 - x/2, t_XR = t_T + 150 + x/2 (so x = t_XR - t_XL), the
// same for y with YU / YD.
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

#include "TFile.h"
#include "TRandom3.h"
#include "TTree.h"

namespace {
struct Hit {
  UChar_t module, channel;
  UShort_t energy, energy_short;
  Double_t t;
};
UShort_t clamp16(double x) { return static_cast<UShort_t>(std::min(65535.0, std::max(0.0, x))); }
}  // namespace

void make_test_run_11b(const char* out = "thgem_test.root", double seconds = 2.0,
                       unsigned seed = 4357) {
  const double T = 199.95;       // true RF period [ns]
  const double t_start = 1.0e6;  // first bunch at 1 ms
  const double L_m = 1.5;        // LS flight path [m]
  const double arm_delay = 150.0;
  TRandom3 rng(seed);
  std::vector<Hit> hits;
  const double t_end = t_start + seconds * 1e9;
  auto bunch = [&](double t) { return t_start + std::floor((t - t_start) / T) * T; };
  auto ls = [&](double t, bool neutron, double e) {
    const double psd = neutron ? rng.Gaus(0.25, 0.03) : rng.Gaus(0.10, 0.03);
    const UChar_t ch = static_cast<UChar_t>(8 + rng.Integer(24));
    hits.push_back({0, ch, clamp16(e), clamp16(e * (1.0 - psd)), t + rng.Gaus(0, 0.7)});
  };
  auto tof_n = [&](double En_MeV) { return L_m * 72.3 / std::sqrt(En_MeV); };
  // ThGEM n fires at t, imaging (x, y); each arm is there with 95 %.
  auto thgem = [&](int n, double t, double x, double y) {
    const UChar_t mod = static_cast<UChar_t>(1 + (n - 1) / 3);
    const int base = 5 * ((n - 1) % 3);
    hits.push_back({mod, static_cast<UChar_t>(base), clamp16(rng.Uniform(1000, 10000)), 0, t});
    const double arm[4] = {t + arm_delay - x / 2, t + arm_delay + x / 2, t + arm_delay + y / 2,
                           t + arm_delay - y / 2};  // XL XR YU YD
    for (int a = 0; a < 4; ++a)
      if (rng.Rndm() < 0.95)
        hits.push_back({mod, static_cast<UChar_t>(base + 1 + a), clamp16(rng.Uniform(500, 5000)),
                        0, arm[a]});
  };
  auto spot_x = [](int n) { return -120.0 + 30.0 * n; };
  auto spot_y = [](int n) { return 90.0 - 20.0 * n; };

  // beam, 1/100
  for (double t = t_start; t < t_end; t += 100 * T)
    hits.push_back({0, 0, 1000, 900, t + rng.Gaus(0, 0.3)});
  // reactions: two fragments into a facing pair
  for (double t = t_start + rng.Exp(1e9 / 2000); t < t_end; t += rng.Exp(1e9 / 2000)) {
    const double tb = bunch(t);
    const int i = 1 + static_cast<int>(rng.Integer(4));
    const double u = rng.Uniform(0, 10);
    for (int k = 0; k < 2; ++k) {
      const int n = k == 0 ? i : i + 4;
      const double dt = k == 0 ? 20.0 + u : 40.0 - u;
      thgem(n, tb + dt + rng.Gaus(0, 0.5), spot_x(n) + rng.Gaus(0, 5), spot_y(n) + rng.Gaus(0, 5));
    }
    const double v = rng.Rndm();
    if (v < 0.3) ls(tb + tof_n(rng.Uniform(0.5, 5.0)), true, rng.Uniform(500, 20000));
    else if (v < 0.5)
      hits.push_back({0, static_cast<UChar_t>(1 + rng.Integer(7)),
                      clamp16(rng.Uniform(500, 40000)), 0, tb + 3.0 + rng.Gaus(0, 0.5)});
  }
  // ThGEM singles
  for (int n = 1; n <= 8; ++n)
    for (double t = t_start + rng.Exp(1e9 / 300); t < t_end; t += rng.Exp(1e9 / 300))
      thgem(n, t, rng.Uniform(-200, 200), rng.Uniform(-200, 200));
  // LS singles: gamma flash and neutrons from unseen reactions
  for (double t = t_start + rng.Exp(1e9 / 50000); t < t_end; t += rng.Exp(1e9 / 50000))
    ls(bunch(t) + L_m / 0.2998, false, rng.Uniform(500, 30000));
  for (double t = t_start + rng.Exp(1e9 / 20000); t < t_end; t += rng.Exp(1e9 / 20000))
    ls(bunch(t) + tof_n(rng.Uniform(0.5, 5.0)), true, rng.Uniform(500, 20000));
  // LaBr3
  for (double t = t_start + rng.Exp(1e9 / 10000); t < t_end; t += rng.Exp(1e9 / 10000))
    hits.push_back({0, static_cast<UChar_t>(1 + rng.Integer(7)),
                    clamp16(rng.Uniform(500, 40000)), 0, bunch(t) + 3.0 + rng.Gaus(0, 0.5)});
  // pulser on the three ThGEM boards together
  for (double t = t_start + 1234.0; t < t_end; t += 1e7)
    for (UChar_t m = 1; m <= 3; ++m) hits.push_back({m, 15, 30000, 0, t});

  std::sort(hits.begin(), hits.end(), [](const Hit& a, const Hit& b) { return a.t < b.t; });
  TFile f(out, "RECREATE");
  TTree tree("delila", "synthetic 11B_208Pb hits (make_test_run_11b.C)");
  Hit h;
  tree.Branch("module", &h.module, "module/b");
  tree.Branch("channel", &h.channel, "channel/b");
  tree.Branch("energy", &h.energy, "energy/s");
  tree.Branch("energy_short", &h.energy_short, "energy_short/s");
  tree.Branch("timestamp_ns", &h.t, "timestamp_ns/D");
  for (const Hit& x : hits) {
    h = x;
    tree.Fill();
  }
  tree.Write();
  std::printf("make_test_run_11b: %zu hits over %.2f s -> %s (T = %.3f ns)\n", hits.size(),
              seconds, out, T);
}

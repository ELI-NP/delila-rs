// make_test_run.C — a synthetic N_Wall run as a hits tree, for rehearsing
// nwall_sink end to end without the detectors (replay it with
// ../common/test_publisher). Not physics: just the time and energy structure
// the display has to show.
//
//   root -l -b -q 'make_test_run.C("nwall_test.root", 2.0)'
//
// Board layout (TODO 79 §5.5, the provisional config/channel_map_N_Wall.txt):
//   module 0 VX2730: ch0 beam (1/100 of the RF), ch1-7 LaBr3, ch8-31 LS
//   module 1 dE sector ch0-15, modules 2-4 dE rings ch0-14 (ch15 pulser),
//   module 5 E sector ch0-15
//
// What is in it (rates per second of data time):
//   * beam: every 100th bunch, T = 199.95 ns (not the nominal 199.98, so the
//     clock has to measure it), 0.3 ns jitter
//   * reactions (2 kHz) on random bunches: an E sector hit + the dE sector
//     above it (same index) + a dE ring, 15 ns after the bunch, 3 ns jitter;
//     two PID bands (dE ~ 1/E); 30 % of them with a neutron in one LS
//     (L = 1.5 m, 0.5-5 MeV), 20 % with a prompt gamma in one LS
//   * LS singles: gamma flash 50 kHz (t = bunch + 5 ns), neutrons 20 kHz
//   * LaBr3 prompt gammas 10 kHz (bunch + 3 ns)
//   * pulser 100 Hz on modules 2-4 ch15 together
// LS pulse shape: (E - Es)/E ≈ 0.25 for neutrons, 0.10 for gammas.
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

void make_test_run(const char* out = "nwall_test.root", double seconds = 2.0,
                   unsigned seed = 4357) {
  const double T = 199.95;        // true RF period [ns]
  const double t_start = 1.0e6;   // first bunch at 1 ms
  const double L_m = 1.5;         // LS flight path [m]
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

  // beam, 1/100
  for (double t = t_start; t < t_end; t += 100 * T)
    hits.push_back({0, 0, 1000, 900, t + rng.Gaus(0, 0.3)});
  // reactions with the Si telescope
  for (double t = t_start + rng.Exp(1e9 / 2000); t < t_end; t += rng.Exp(1e9 / 2000)) {
    const double tb = bunch(t);
    const double tsi = tb + 15.0;
    const int sec = static_cast<int>(rng.Integer(16));
    const int ring = static_cast<int>(rng.Integer(45));
    const double E = rng.Uniform(1000, 20000);
    const double band = rng.Rndm() < 0.7 ? 1.0 : 3.5;  // "protons" and "alphas"
    const double dE = band * 2.0e6 / (E + 2000.0) + rng.Gaus(0, 60);
    hits.push_back({5, static_cast<UChar_t>(sec), clamp16(E), 0, tsi + rng.Gaus(0, 3)});
    hits.push_back({1, static_cast<UChar_t>(sec), clamp16(dE), 0, tsi + rng.Gaus(0, 3)});
    hits.push_back({static_cast<UChar_t>(2 + ring / 15), static_cast<UChar_t>(ring % 15),
                    clamp16(dE * rng.Uniform(0.9, 1.1)), 0, tsi + rng.Gaus(0, 3)});
    const double u = rng.Rndm();
    if (u < 0.3) ls(tb + tof_n(rng.Uniform(0.5, 5.0)), true, rng.Uniform(500, 20000));
    else if (u < 0.5) ls(tb + L_m / 0.2998, false, rng.Uniform(500, 30000));
  }
  // LS singles: gamma flash and neutrons from unseen reactions
  for (double t = t_start + rng.Exp(1e9 / 50000); t < t_end; t += rng.Exp(1e9 / 50000))
    ls(bunch(t) + L_m / 0.2998, false, rng.Uniform(500, 30000));
  for (double t = t_start + rng.Exp(1e9 / 20000); t < t_end; t += rng.Exp(1e9 / 20000))
    ls(bunch(t) + tof_n(rng.Uniform(0.5, 5.0)), true, rng.Uniform(500, 20000));
  // LaBr3
  for (double t = t_start + rng.Exp(1e9 / 10000); t < t_end; t += rng.Exp(1e9 / 10000))
    hits.push_back({0, static_cast<UChar_t>(1 + rng.Integer(7)),
                    clamp16(rng.Uniform(500, 40000)), 0, bunch(t) + 3.0 + rng.Gaus(0, 0.5)});
  // pulser on the three ring boards together
  for (double t = t_start + 1234.0; t < t_end; t += 1e7)
    for (UChar_t m = 2; m <= 4; ++m) hits.push_back({m, 15, 30000, 0, t});

  std::sort(hits.begin(), hits.end(), [](const Hit& a, const Hit& b) { return a.t < b.t; });
  TFile f(out, "RECREATE");
  TTree tree("delila", "synthetic N_Wall hits (make_test_run.C)");
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
  std::printf("make_test_run: %zu hits over %.2f s -> %s (T = %.3f ns)\n", hits.size(), seconds,
              out, T);
}

// p2_check.C — TODO 90 P2: does cfd_emu.hpp reproduce the V1730 FW CFD time?
// Per event: the software crossing in the recorded input vs the FW timestamp's
// fraction of a sample (10-bit fine time). Several model variants side by side.
// Input: a delila2root file of a run with waveforms in SINGLE trace (Input only,
// full rate) and the channel's FW CFD settings passed as arguments.
//
//   delila2root run.delila -o run_wf.root
//   root -l -b -q -e '.L p2_check.C+' -e 'p2_check("run_wf.root", 1, 15, 10, 50, 8, 100)'
//
// 2026-11-20 pf-qaq DAQ_Test run 12 (TTL pulser on V1730 ch15; delay 10 samples,
// 50 %, smooth 8, threshold 100): moving average = FW, RMS 0.0008 samples
// (0.002 ns), median +0.0006 (10-bit fine-time truncation), one integer offset
// for 5000/5000 events on each of 3 boards. Exponential smoothing is off by 0.48
// samples. License: BSD-3-Clause (same as delila-rs).
#include <TFile.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <map>
#include <vector>

#include "cfd_emu.hpp"

using namespace cfdtune;

// Exponential (IIR) smoothing variant: y += (x - y) / n.
static std::vector<double> exp_smooth(const std::vector<double>& s, int n) {
  if (n <= 1 || s.empty()) return s;
  std::vector<double> y(s.size());
  double acc = s[0];
  for (size_t i = 0; i < s.size(); ++i) {
    acc += (s[i] - acc) / n;
    y[i] = acc;
  }
  return y;
}

struct Stat {
  std::vector<double> d;  // frac differences, samples
  std::map<int, int> nint;
  int missing = 0;
  void add(double v) { d.push_back(v); }
  void print(const char* name) const {
    if (d.empty()) {
      printf("  %-34s no crossings (missing %d)\n", name, missing);
      return;
    }
    std::vector<double> v = d;
    std::sort(v.begin(), v.end());
    double mean = 0, rms = 0;
    for (double x : v) mean += x;
    mean /= v.size();
    for (double x : v) rms += (x - mean) * (x - mean);
    rms = std::sqrt(rms / v.size());
    double med = v[v.size() / 2];
    double in01 = 0;
    for (double x : v)
      if (std::fabs(x - med) < 0.05) ++in01;
    printf("  %-34s n=%6zu  median %+7.4f  mean %+7.4f  RMS %.4f samples (%.3f ns)  |d-med|<0.05: %5.1f %%  missing %d  int offsets:",
           name, v.size(), med, mean, rms, rms * 2.0, 100.0 * in01 / v.size(), missing);
    for (auto& [k, c] : nint) printf(" %d:%d", k, c);
    printf("\n");
  }
};

void p2_check(const char* file, int module = 1, int channel = 15, int delay = 10, int frac = 50, int smooth = 8,
              double thr = 100, int nbase = 64, int maxev = 5000) {
  TFile in(file);
  TTree* t = (TTree*)in.Get("delila");
  UChar_t mod, ch;
  Bool_t hw;
  Double_t nsps, ts;
  std::vector<short>* a1 = nullptr;
  std::vector<short>* a2 = nullptr;
  t->SetBranchAddress("module", &mod);
  t->SetBranchAddress("channel", &ch);
  t->SetBranchAddress("has_waveform", &hw);
  t->SetBranchAddress("ns_per_sample", &nsps);
  t->SetBranchAddress("timestamp_ns", &ts);
  t->SetBranchAddress("analog_probe1", &a1);
  t->SetBranchAddress("analog_probe2", &a2);

  struct Variant {
    const char* name;
    bool expo;
    int sm;
    int interp;
    double thr_scale;
  };
  std::vector<Variant> vars = {
      {"moving avg, smooth as set", false, smooth, 0, 1.0},
      {"exponential, smooth as set", true, smooth, 0, 1.0},
      {"moving avg, smooth 1", false, 1, 0, 1.0},
      {"moving avg, smooth as set, interp1", false, smooth, 1, 1.0},
  };
  std::vector<Stat> st(vars.size());
  int n = 0;
  size_t wlen = 0;
  double dt = 0;
  for (Long64_t i = 0; i < t->GetEntries() && n < maxev; ++i) {
    t->GetEntry(i);
    if (mod != module || ch != channel || !hw || a1->size() < 50) continue;
    ++n;
    wlen = a1->size();
    dt = nsps;
    double fw_frac = ts / nsps - std::floor(ts / nsps);
    auto s0 = prepare(*a1, Polarity::Positive, nbase);
    for (size_t k = 0; k < vars.size(); ++k) {
      const auto& v = vars[k];
      auto s = v.expo ? exp_smooth(s0, v.sm) : moving_average(s0, v.sm);
      Crossing c = cfd_crossing(s, CfdParams{delay, frac, v.sm, v.interp}, thr * v.thr_scale);
      if (!c.found) {
        ++st[k].missing;
        continue;
      }
      double sw_frac = c.pos - std::floor(c.pos);
      double d = frac_diff(sw_frac, fw_frac);
      st[k].add(d);
      st[k].nint[(int)std::lround(c.pos - fw_frac - d)]++;
    }
  }
  printf("%s m%d c%d: %d waveforms, %zu samples, %.2f ns/sample; FW setting delay %d samples, %d %%, smooth %d, thr %.0f\n",
         file, module, channel, n, wlen, dt, delay, frac, smooth, thr);
  for (size_t k = 0; k < vars.size(); ++k) st[k].print(vars[k].name);
}

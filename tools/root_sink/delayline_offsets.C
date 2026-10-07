// delayline_offsets.C — measure the per-arm time offsets that centre the
// delay-line XY images, and print them as a ready-to-use timeSettings.json
// for root_sink --time-calib (TODO 70).
//
// Input: hits trees with branches module/channel/timestamp_ns — delila2root
// output of a .delila (raw times), or a root_sink .root (then the offsets it
// was written with are already applied, and the numbers printed here are the
// RESIDUALS: a correct table gives |offset| < 0.5 ns).
//
//   root -l -b -q 'delayline_offsets.C("run0056_*.root","1,2,3,5,4","6,7,8,9,10","ts.json","run 56")'
//   root -l -b -q 'delayline_offsets.C+(...)'           (compiled, faster on big runs)
//   root -l -b -q -e '.L delayline_offsets.C' -e 'delayline_centres_built("run0057_0000_X.root")'
//
// Channel lists use root_sink's --xy1-ch/--xy2-ch order T,XL,XR,YU,YD; an empty
// xy2 means a single plane.
//
// Method (per plane):
//   * per trigger, the NEAREST hit of each arm within ±win (ties -> the later
//     hit: the same rule as root_sink's builder, eb_core.hpp nearest_in);
//   * only 4-fold events (both X arms and both Y arms) — the ones that make
//     the 2D image;
//   * cx, cy = mid-of-edges (midpoint of the 1 % and 99 % quantiles) of the
//     displayed x = t_XR - t_XL and y = t_YU - t_YD;
//   * offsets XL = -cx/2, XR = +cx/2, YU = +cy/2, YD = -cy/2, trigger = 0.
// Why this way:
//   * quantiles move rigidly with a constant shift, so aligned x = x - cx puts
//     the displayed image's mid-of-edges at exactly 0 (per-arm mids would only
//     get close: mid(XR) - mid(XL) != mid(XR - XL));
//   * only arm DIFFERENCES enter x/y — the triggers and the LaBr3 channel keep
//     offset 0, so TOF, dt_trig and dt1/dt2 are untouched;
//   * the symmetric split keeps each axis' time sum (dt_XL + dt_XR) unchanged;
//   * orientation-agnostic: taking the Y wires the other way round flips the
//     sign of cy AND which wire is YU, giving the same per-channel offsets.
// The absolute arm-to-trigger delay (side3 ~88-90 ns, pf-qaq 119-146 ns) is
// deliberately NOT restored — that would need shifting both arms of an axis
// together, which changes the sums and cannot be read off a single run.
//
// Output tree: plane-1 trigger = root (offset 0); plane-2 trigger refs it with
// 0 (one timing domain); arms ref their own trigger; every OTHER channel seen in
// the input gets 0 under the root, so root_sink's "no entry" warning later means
// "a channel appeared that was not there at calibration time".
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <set>
#include <string>
#include <vector>

#include "TChain.h"
#include "TFile.h"
#include "TList.h"
#include "TNamed.h"
#include "TTree.h"

namespace dlo {

struct Plane {
  int t = -1, xl = -1, xr = -1, yu = -1, yd = -1;
  bool on() const { return t >= 0; }
};

inline bool parse_plane(const std::string& s, Plane& p) {
  if (s.empty()) return true;  // plane off
  int v[5];
  if (std::sscanf(s.c_str(), "%d,%d,%d,%d,%d", &v[0], &v[1], &v[2], &v[3], &v[4]) != 5) return false;
  for (int x : v)
    if (x < 0 || x > 255) return false;
  p.t = v[0]; p.xl = v[1]; p.xr = v[2]; p.yu = v[3]; p.yd = v[4];
  return true;
}

// eb_core.hpp nearest_in: nearest within ±win, ties -> the successor.
inline bool nearest(const std::vector<double>& v, double t0, double win, double& dt) {
  if (v.empty()) return false;
  auto it = std::lower_bound(v.begin(), v.end(), t0);
  std::size_t idx;
  if (it == v.end()) {
    idx = v.size() - 1;
  } else {
    idx = static_cast<std::size_t>(it - v.begin());
    if (idx > 0 && (t0 - v[idx - 1]) < (v[idx] - t0)) idx = idx - 1;
  }
  double d = v[idx] - t0;
  if (std::fabs(d) > win) return false;
  dt = d;
  return true;
}

struct Q {
  std::size_t n = 0;
  double med = 0, lo = 0, hi = 0, mid = 0;
};
inline Q quant(std::vector<double> v) {
  Q q;
  q.n = v.size();
  if (v.empty()) return q;
  std::sort(v.begin(), v.end());
  auto at = [&](double f) { return v[static_cast<std::size_t>(f * (v.size() - 1))]; };
  q.med = at(0.5);
  q.lo = at(0.01);
  q.hi = at(0.99);
  q.mid = 0.5 * (q.lo + q.hi);
  return q;
}

struct PlaneResult {
  bool ok = false;
  std::size_t full = 0;
  double cx = 0, cy = 0;
  Q x, y, dxl, dxr, dyu, dyd;
};

inline PlaneResult measure(const Plane& p, const std::vector<double> (&t)[256], double win) {
  PlaneResult r;
  std::vector<double> X, Y, XL, XR, YU, YD;
  for (double tr : t[p.t]) {
    double a, b, c, d;
    if (!nearest(t[p.xl], tr, win, a) || !nearest(t[p.xr], tr, win, b) ||
        !nearest(t[p.yu], tr, win, c) || !nearest(t[p.yd], tr, win, d))
      continue;
    X.push_back(b - a);
    Y.push_back(c - d);
    XL.push_back(a); XR.push_back(b); YU.push_back(c); YD.push_back(d);
  }
  r.full = X.size();
  r.x = quant(X); r.y = quant(Y);
  r.dxl = quant(XL); r.dxr = quant(XR); r.dyu = quant(YU); r.dyd = quant(YD);
  r.cx = r.x.mid;
  r.cy = r.y.mid;
  r.ok = r.full >= 100;
  return r;
}

inline void print_q(const char* name, const Q& q) {
  std::printf("    %-16s n=%-7zu median %+8.2f  edges(1/99%%) %+8.1f .. %+8.1f  mid %+8.2f\n", name,
              q.n, q.med, q.lo, q.hi, q.mid);
}

}  // namespace dlo

void delayline_offsets(const char* pattern, const char* xy1 = "1,2,3,5,4",
                       const char* xy2 = "6,7,8,9,10", const char* out_json = "",
                       const char* label = "", int module = 0, double win = 1000.0) {
  dlo::Plane p1, p2;
  if (!dlo::parse_plane(xy1, p1) || !p1.on()) {
    std::printf("delayline_offsets: bad xy1 '%s' (need T,XL,XR,YU,YD)\n", xy1);
    return;
  }
  if (!dlo::parse_plane(xy2, p2)) {
    std::printf("delayline_offsets: bad xy2 '%s'\n", xy2);
    return;
  }
  TChain ch("delila");
  int nfiles = ch.Add(pattern);
  if (nfiles == 0 || ch.GetEntries() == 0) {
    std::printf("delayline_offsets: no 'delila' entries in %s\n", pattern);
    return;
  }
  // Already-aligned input (root_sink .root with --time-calib)?
  bool aligned_input = false;
  {
    TFile* f0 = ch.GetFile();
    TTree* t0 = f0 ? dynamic_cast<TTree*>(f0->Get("delila")) : nullptr;
    TNamed* ts = t0 ? dynamic_cast<TNamed*>(t0->GetUserInfo()->FindObject("timeSettings")) : nullptr;
    if (ts && std::string(ts->GetTitle()).find("\"applied\":true") != std::string::npos)
      aligned_input = true;
  }
  UChar_t mod = 0, chn = 0;
  Double_t ts = 0;
  ch.SetBranchStatus("*", false);
  ch.SetBranchStatus("module", true);
  ch.SetBranchStatus("channel", true);
  ch.SetBranchStatus("timestamp_ns", true);
  ch.SetBranchAddress("module", &mod);
  ch.SetBranchAddress("channel", &chn);
  ch.SetBranchAddress("timestamp_ns", &ts);
  static std::vector<double> t[256];
  for (auto& v : t) v.clear();
  std::set<int> seen;
  const Long64_t n = ch.GetEntries();
  for (Long64_t i = 0; i < n; ++i) {
    ch.GetEntry(i);
    if (mod != module) continue;
    t[chn].push_back(ts);
    seen.insert(chn);
  }
  for (auto& v : t) std::sort(v.begin(), v.end());

  std::printf("delayline_offsets: %s  (%d file(s), %lld hits, module %d, window ±%.0f ns)%s\n",
              label[0] ? label : pattern, nfiles, n, module, win,
              aligned_input ? "\n  NOTE: input is ALIGNED (root_sink --time-calib) — the "
                              "offsets below are RESIDUALS"
                            : "");
  dlo::PlaneResult r1 = dlo::measure(p1, t, win), r2;
  if (p2.on()) r2 = dlo::measure(p2, t, win);
  auto report = [&](const char* name, const dlo::Plane& p, const dlo::PlaneResult& r) {
    std::printf("  %s (trig ch%d, XL ch%d, XR ch%d, YU ch%d, YD ch%d): %zu 4-fold events%s\n", name,
                p.t, p.xl, p.xr, p.yu, p.yd, r.full, r.ok ? "" : "  -- TOO FEW (<100), offsets 0");
    dlo::print_q("x = XR - XL", r.x);
    dlo::print_q("y = YU - YD", r.y);
    dlo::print_q("XL - trig", r.dxl);
    dlo::print_q("XR - trig", r.dxr);
    dlo::print_q("YU - trig", r.dyu);
    dlo::print_q("YD - trig", r.dyd);
    if (r.ok)
      std::printf("    centre (cx, cy) = (%+.3f, %+.3f) ns -> XL %+.3f  XR %+.3f  YU %+.3f  YD %+.3f\n",
                  r.cx, r.cy, -r.cx / 2, +r.cx / 2, +r.cy / 2, -r.cy / 2);
  };
  report("plane 1", p1, r1);
  if (p2.on()) report("plane 2", p2, r2);

  // Build the tree-form JSON.
  struct E { int ch, ref; double off; };
  std::vector<E> es;
  std::set<int> done;
  auto add = [&](int c, int ref, double off) {
    if (c < 0 || done.count(c)) return;
    es.push_back({c, ref, off});
    done.insert(c);
  };
  add(p1.t, -1, 0.0);
  if (p2.on()) add(p2.t, p1.t, 0.0);
  auto arms = [&](const dlo::Plane& p, const dlo::PlaneResult& r) {
    double cx = r.ok ? r.cx : 0.0, cy = r.ok ? r.cy : 0.0;
    add(p.xl, p.t, -cx / 2);
    add(p.xr, p.t, +cx / 2);
    add(p.yu, p.t, +cy / 2);
    add(p.yd, p.t, -cy / 2);
  };
  arms(p1, r1);
  if (p2.on()) arms(p2, r2);
  for (int c : seen) add(c, p1.t, 0.0);

  char buf[512];
  std::string js = "{\n  \"$schema\": \"../schemas/timeSettings.schema.json\",\n  \"_comment\": [\n";
  std::snprintf(buf, sizeof buf,
                "    \"delayline_offsets.C on %s (module %d): centres each delay-line XY image "
                "(mid of 1/99 %% edges) by splitting its raw centre over the two arms of each axis; "
                "triggers and other channels 0.\",\n",
                label[0] ? label : pattern, module);
  js += buf;
  std::snprintf(buf, sizeof buf, "    \"raw centres%s: plane1 (%+.3f, %+.3f) ns from %zu events",
                aligned_input ? " (RESIDUAL: input already aligned)" : "", r1.cx, r1.cy, r1.full);
  js += buf;
  if (p2.on()) {
    std::snprintf(buf, sizeof buf, "; plane2 (%+.3f, %+.3f) ns from %zu events", r2.cx, r2.cy, r2.full);
    js += buf;
  }
  js += ".\",\n    \"aligned_ts = raw_ts - offset_ns (positive = channel arrives late).\"\n  ],\n";
  js += "  \"version\": \"1.0\",\n  \"entries\": [\n";
  for (std::size_t i = 0; i < es.size(); ++i) {
    if (es[i].ref < 0)
      std::snprintf(buf, sizeof buf, "    {\"module\": %d, \"channel\": %d, \"ref\": null, \"offset_ns\": %.4f}",
                    module, es[i].ch, es[i].off);
    else
      std::snprintf(buf, sizeof buf,
                    "    {\"module\": %d, \"channel\": %d, \"ref\": [%d, %d], \"offset_ns\": %.4f}", module,
                    es[i].ch, module, es[i].ref, es[i].off + 0.0);  // +0.0: no "-0.0000"
    js += buf;
    js += (i + 1 < es.size()) ? ",\n" : "\n";
  }
  js += "  ]\n}\n";
  std::printf("\n%s", js.c_str());
  if (out_json && out_json[0]) {
    FILE* f = std::fopen(out_json, "w");
    if (!f) {
      std::printf("delayline_offsets: cannot write %s\n", out_json);
      return;
    }
    std::fputs(js.c_str(), f);
    std::fclose(f);
    std::printf("delayline_offsets: wrote %s\n", out_json);
  }
}

// Mid-of-edges image centres from a root_sink built tree (4-fold per plane).
void delayline_centres_built(const char* file, const char* tree = "built") {
  TFile* f = TFile::Open(file, "READ");
  TTree* b = f ? dynamic_cast<TTree*>(f->Get(tree)) : nullptr;
  if (!b) {
    std::printf("delayline_centres_built: no tree '%s' in %s\n", tree, file);
    return;
  }
  std::printf("delayline_centres_built: %s (%lld built events)\n", file, b->GetEntries());
  for (int p = 1; p <= 2; ++p) {
    std::vector<double> X, Y;
    Float_t x = 0, y = 0;
    b->SetBranchStatus("*", false);
    b->SetBranchStatus(Form("x%d", p), true);
    b->SetBranchStatus(Form("y%d", p), true);
    b->SetBranchAddress(Form("x%d", p), &x);
    b->SetBranchAddress(Form("y%d", p), &y);
    for (Long64_t i = 0; i < b->GetEntries(); ++i) {
      b->GetEntry(i);
      if (std::isnan(x) || std::isnan(y)) continue;
      X.push_back(x);
      Y.push_back(y);
    }
    b->ResetBranchAddresses();
    dlo::Q qx = dlo::quant(X), qy = dlo::quant(Y);
    std::printf("  plane %d: %zu 4-fold  centre (x, y) = (%+.3f, %+.3f) ns  edges x %+.1f..%+.1f  "
                "y %+.1f..%+.1f\n",
                p, X.size(), qx.mid, qy.mid, qx.lo, qx.hi, qy.lo, qy.hi);
  }
  f->Close();
}

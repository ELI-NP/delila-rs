// cfd_scan.hpp — the pure logic of the offline CFD scan for ThGEM delay lines
// (TODO 90 P3), on top of the FW model in cfd_emu.hpp (checked against the FW
// in P2: RMS 0.002 ns on V1730 pulser waveforms).
//
//   1. parse_channel_settings: a digitizer JSON (defaults + overrides, the
//      Operator's format) -> the FW CFD setting of every channel.
//   2. make_hit / HitWave::retime: one recorded waveform; its time under any
//      other CFD setting = FW time + (model crossing(new) - model crossing(FW)).
//   3. group_thgem: trigger + the nearest hit of each arm within +-window (the
//      thgem_sink rule: thgem_reco.hpp nearest_in).
//   4. time_sums / sigma68: S_X = t_XL + t_XR - 2 t_T, S_Y = t_YU + t_YD - 2 t_T.
//      A delay line makes them position-independent, so their width is the
//      timing resolution — no known positions needed.
//   5. coordinate_descent: full grid search of the arm setting (common to the
//      four arms of one ThGEM) with the trigger fixed, then of the trigger with
//      the arms fixed, repeated until nothing changes.
//   6. patch_json: the result as channel_overrides in the digitizer JSON's names.
//
// No ROOT. Tests: test_cfd_scan.cpp. License: BSD-3-Clause (same as delila-rs).

#pragma once

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <functional>
#include <limits>
#include <map>
#include <stdexcept>
#include <string>
#include <vector>

#include "TDelila.hpp"
#include "cfd_emu.hpp"

namespace cfdtune {

// ---------------------------------------------------------------------------
// 1. FW settings from the digitizer JSON
// ---------------------------------------------------------------------------

struct ChannelFw {
  bool enabled = true;
  bool cfd_mode = true;  // DISCR_MODE_CFD; LED channels are not modelled
  Polarity pol = Polarity::Negative;
  CfdParams p;
  double threshold = 0;
};

struct ChannelSettings {
  int module = -1;
  std::map<int, ChannelFw> ch;
  std::vector<std::string> errors;
};

namespace scan_detail {
using JV = tdelila::json::Value;

inline std::string text(const JV* v) {
  if (!v) return "";
  if (v->t == JV::T::Str) return v->str;
  if (v->t == JV::T::Num) {
    char b[64];
    std::snprintf(b, sizeof(b), "%g", v->num);
    return b;
  }
  if (v->t == JV::T::Bool) return v->b ? "True" : "False";
  return "";
}

inline bool truthy(const std::string& s) { return s == "True" || s == "true" || s == "TRUE" || s == "1"; }

// Value of `key` for one channel: the override if present, else the default.
inline const JV* pick(const JV* over, const JV* defs, const char* key) {
  if (over) {
    if (const JV* v = over->find(key)) return v;
  }
  return defs ? defs->find(key) : nullptr;
}

inline int enum_suffix(const std::string& s, const std::string& prefix, int& out) {
  if (s.compare(0, prefix.size(), prefix) != 0) return 0;
  try {
    size_t used = 0;
    out = std::stoi(s.substr(prefix.size()), &used);
    return used == s.size() - prefix.size() ? 1 : 0;
  } catch (...) {
    return 0;
  }
}
}  // namespace scan_detail

inline ChannelSettings parse_channel_settings(const std::string& json_text, double ns_per_sample) {
  using namespace scan_detail;
  ChannelSettings out;
  JV root;
  try {
    root = tdelila::json::parse(json_text);
  } catch (const std::exception& e) {
    out.errors.push_back(std::string("digitizer JSON: ") + e.what());
    return out;
  }
  if (root.t != JV::T::Obj) {
    out.errors.push_back("digitizer JSON: not an object");
    return out;
  }
  const JV* id = root.find("digitizer_id");
  out.module = id && id->t == JV::T::Num ? static_cast<int>(id->num) : -1;
  const JV* nch = root.find("num_channels");
  int n = nch && nch->t == JV::T::Num ? static_cast<int>(nch->num) : 16;
  const JV* defs = root.find("channel_defaults");
  const JV* overs = root.find("channel_overrides");
  for (int c = 0; c < n; ++c) {
    const JV* over = overs ? overs->find(std::to_string(c)) : nullptr;
    ChannelFw f;
    const std::string where = "module " + std::to_string(out.module) + " ch" + std::to_string(c) + ": ";
    std::string en = text(pick(over, defs, "enabled"));
    f.enabled = en.empty() ? true : truthy(en);
    std::string pol = text(pick(over, defs, "polarity"));
    f.pol = pol == "POLARITY_POSITIVE" ? Polarity::Positive : Polarity::Negative;
    if (!pol.empty() && pol != "POLARITY_POSITIVE" && pol != "POLARITY_NEGATIVE")
      out.errors.push_back(where + "polarity " + pol);
    std::string mode = text(pick(over, defs, "discriminator_mode"));
    f.cfd_mode = mode.empty() || mode == "DISCR_MODE_CFD";

    const JV* d = pick(over, defs, "cfd_delay_ns");
    double dns = d && d->t == JV::T::Num ? d->num : 0.0;
    double ds = dns / ns_per_sample;
    if (std::fabs(ds - std::round(ds)) > 1e-9 || ds < 1) {
      out.errors.push_back(where + "cfd_delay_ns " + text(d) + " is not a whole number (>= 1) of " +
                           std::to_string(ns_per_sample) + " ns samples");
    }
    f.p.delay = static_cast<int>(std::lround(ds));

    std::string fr = text(pick(over, defs, "cfd_fraction"));
    int frac = 0;
    if (!enum_suffix(fr, "CFD_FRACTLIST_", frac) || (frac != 25 && frac != 50 && frac != 75 && frac != 100))
      out.errors.push_back(where + "cfd_fraction " + fr);
    f.p.fraction_pct = frac;

    std::string sm = text(pick(over, defs, "input_smoothing"));
    int smooth = 1;
    if (!sm.empty() &&
        (!enum_suffix(sm, "CFD_SMOOTH_EXP_", smooth) ||
         (smooth != 1 && smooth != 2 && smooth != 4 && smooth != 8 && smooth != 16)))
      out.errors.push_back(where + "input_smoothing " + sm);
    f.p.smooth = smooth;

    const JV* ip = pick(over, defs, "cfd_interpolation_point");
    f.p.interp = ip && ip->t == JV::T::Num ? static_cast<int>(ip->num) : 0;
    const JV* thr = pick(over, defs, "trigger_threshold");
    f.threshold = thr && thr->t == JV::T::Num ? thr->num : 0.0;
    out.ch[c] = f;
  }
  return out;
}

// ---------------------------------------------------------------------------
// 2. One recorded hit, re-timed under another setting
// ---------------------------------------------------------------------------

constexpr int kSmooths[5] = {1, 2, 4, 8, 16};

inline int smooth_index(int n) {
  for (int i = 0; i < 5; ++i)
    if (kSmooths[i] == n) return i;
  return -1;
}

/// True when every sample pair (2k, 2k+1) holds the same value: a DIG1
/// dual-trace record (VPROBE_CFD etc. on). The FW then samples each trace every
/// 4 ns and writes each value twice, while the record still says 2 ns/sample
/// (pf-qaq DAQ_Test run 17, 2026-11-21). Noise makes a single-trace record fail
/// this within the first few pairs.
inline bool pairs_duplicated(const std::vector<short>& a) {
  if (a.size() < 8 || a.size() % 2 != 0) return false;
  for (size_t i = 0; i + 1 < a.size(); i += 2)
    if (a[i] != a[i + 1]) return false;
  return true;
}

enum class Undouble { Linear, Cubic };

/// 2 ns samples rebuilt from a dual-trace record: the even samples keep the
/// recorded values, each odd sample is interpolated half-way to the next value
/// (Linear: the mean; Cubic: 4-point Lagrange, (-v[k-1] + 9 v[k] + 9 v[k+1] -
/// v[k+2]) / 16, linear where a neighbour is missing). The last odd sample
/// repeats the last value. A record that is not dual trace is returned as is.
/// Whether this is good enough is measured, not assumed: p2_delila compares the
/// model on the rebuilt record with the FW time.
inline std::vector<short> undouble(const std::vector<short>& a, Undouble mode) {
  if (!pairs_duplicated(a)) return a;
  const long m = static_cast<long>(a.size() / 2);  // distinct values
  auto v = [&](long k) { return static_cast<double>(a[static_cast<size_t>(2 * k)]); };
  std::vector<short> out(a.size());
  for (long k = 0; k < m; ++k) {
    out[static_cast<size_t>(2 * k)] = a[static_cast<size_t>(2 * k)];
    double mid;
    if (k + 1 >= m)
      mid = v(k);
    else if (mode == Undouble::Cubic && k >= 1 && k + 2 < m)
      mid = (-v(k - 1) + 9.0 * v(k) + 9.0 * v(k + 1) - v(k + 2)) / 16.0;
    else
      mid = 0.5 * (v(k) + v(k + 1));
    out[static_cast<size_t>(2 * k + 1)] = static_cast<short>(std::lround(mid));
  }
  return out;
}

struct HitWave {
  double t_fw = 0;    // FW timestamp, ns
  double pos_fw = 0;  // model crossing at the FW setting, samples
  std::vector<std::vector<double>> sm;  // the prepared input under each kSmooths value

  // Time of this hit under `p`: the FW time moved by the model's difference.
  bool retime(const CfdParams& p, double threshold, double ns_per_sample, double& t) const {
    int k = smooth_index(p.smooth);
    if (k < 0 || sm.empty()) return false;
    Crossing c = cfd_crossing(sm[static_cast<size_t>(k)], p, threshold);
    if (!c.found) return false;
    t = t_fw + (c.pos - pos_fw) * ns_per_sample;
    return true;
  }
};

/// Builds a hit from its raw waveform. False when the model does not time it at
/// the FW setting (then the FW time cannot be related to the waveform).
/// crop_pre / crop_post > 0 keep only that many samples before / after the FW
/// crossing (memory: five smoothed copies per hit), and the crossing is found
/// again on the cropped copy; keep crop_pre well above the longest scanned
/// delay plus the longest smoothing.
inline bool make_hit(const std::vector<short>& adc, const ChannelFw& fw, double t_fw, double ns_per_sample,
                     int n_baseline, HitWave& out, int crop_pre = 0, int crop_post = 0) {
  (void)ns_per_sample;
  const int k = smooth_index(fw.p.smooth);
  if (k < 0) return false;
  std::vector<double> s = prepare(adc, fw.pol, n_baseline);
  Crossing c = cfd_crossing(moving_average(s, fw.p.smooth), fw.p, fw.threshold);
  if (!c.found) return false;
  if (crop_pre > 0 && crop_post > 0) {
    long a = std::max(0L, static_cast<long>(std::floor(c.pos)) - crop_pre);
    long b = std::min(static_cast<long>(s.size()), static_cast<long>(std::floor(c.pos)) + crop_post);
    s = std::vector<double>(s.begin() + a, s.begin() + b);
  }
  out.sm.clear();
  for (int n : kSmooths) out.sm.push_back(moving_average(s, n));
  c = cfd_crossing(out.sm[static_cast<size_t>(k)], fw.p, fw.threshold);
  if (!c.found) return false;
  out.t_fw = t_fw;
  out.pos_fw = c.pos;
  return true;
}

// ---------------------------------------------------------------------------
// 3. ThGEM grouping
// ---------------------------------------------------------------------------

/// Hits of one electrode channel, times ascending, with an index into the
/// caller's hit list.
struct ChannelHits {
  std::vector<double> t;
  std::vector<int> hit;
};

/// One ThGEM event: hit indices for T, XL, XR, YU, YD (-1 = no hit).
struct ThgemEvent {
  int hit[5] = {-1, -1, -1, -1, -1};
};

/// The nearest hit to t0 within +-window (thgem_reco.hpp nearest_in: ties go to
/// the later hit).
inline int nearest_hit(const ChannelHits& c, double t0, double window) {
  if (c.t.empty()) return -1;
  auto it = std::lower_bound(c.t.begin(), c.t.end(), t0);
  size_t i;
  if (it == c.t.end()) {
    i = c.t.size() - 1;
  } else {
    i = static_cast<size_t>(it - c.t.begin());
    if (i > 0 && (t0 - c.t[i - 1]) < (c.t[i] - t0)) i = i - 1;
  }
  return std::fabs(c.t[i] - t0) <= window ? c.hit[i] : -1;
}

/// `ch` = the five electrodes in the order T, XL, XR, YU, YD. One event per
/// trigger hit.
inline std::vector<ThgemEvent> group_thgem(const std::vector<ChannelHits>& ch, double window) {
  std::vector<ThgemEvent> out;
  if (ch.size() < 5) return out;
  for (size_t i = 0; i < ch[0].t.size(); ++i) {
    ThgemEvent e;
    e.hit[0] = ch[0].hit[i];
    for (int k = 1; k < 5; ++k) e.hit[k] = nearest_hit(ch[static_cast<size_t>(k)], ch[0].t[i], window);
    out.push_back(e);
  }
  return out;
}

// ---------------------------------------------------------------------------
// 4. Time sums and their width
// ---------------------------------------------------------------------------

struct TimeSums {
  std::vector<double> x, y;
};

/// `times` indexed by hit; a missing hit (-1) or a NaN time leaves that event
/// out of that sum only.
inline TimeSums time_sums(const std::vector<ThgemEvent>& ev, const std::vector<double>& times) {
  TimeSums s;
  auto ok = [&](int h) { return h >= 0 && static_cast<size_t>(h) < times.size() && !std::isnan(times[h]); };
  for (const auto& e : ev) {
    if (!ok(e.hit[0])) continue;
    double tt = times[e.hit[0]];
    if (ok(e.hit[1]) && ok(e.hit[2])) s.x.push_back(times[e.hit[1]] + times[e.hit[2]] - 2 * tt);
    if (ok(e.hit[3]) && ok(e.hit[4])) s.y.push_back(times[e.hit[3]] + times[e.hit[4]] - 2 * tt);
  }
  return s;
}

/// Half of the central 68 % interval (16th to 84th percentile, linear
/// interpolation). Robust against the tails of wrong arm choices. NaN if empty.
inline double sigma68(std::vector<double> v) {
  if (v.empty()) return std::numeric_limits<double>::quiet_NaN();
  std::sort(v.begin(), v.end());
  auto q = [&](double p) {
    double x = p * static_cast<double>(v.size() - 1);
    size_t i = static_cast<size_t>(std::floor(x));
    if (i + 1 >= v.size()) return v.back();
    return v[i] + (x - static_cast<double>(i)) * (v[i + 1] - v[i]);
  };
  return (q(0.84) - q(0.16)) / 2.0;
}

// ---------------------------------------------------------------------------
// 5. Search
// ---------------------------------------------------------------------------

struct Grid {
  std::vector<int> delays;     // samples
  std::vector<int> fractions;  // %
  std::vector<int> smooths;    // averaged samples
};

struct SearchResult {
  CfdParams t, arm;
  double cost = std::numeric_limits<double>::infinity();
  int rounds = 0;
  long evaluations = 0;
};

using CostFn = std::function<double(const CfdParams& trig, const CfdParams& arm)>;

/// Alternating full grid searches: arms with the trigger fixed, then the
/// trigger with the arms fixed, until a round changes nothing (or max_rounds).
/// A non-finite cost (e.g. too few events) is never chosen.
inline SearchResult coordinate_descent(const Grid& g, CfdParams t0, CfdParams arm0, const CostFn& cost,
                                       int max_rounds) {
  SearchResult r;
  r.t = t0;
  r.arm = arm0;
  r.cost = cost(r.t, r.arm);
  ++r.evaluations;
  auto best_of = [&](bool arms) {
    CfdParams best = arms ? r.arm : r.t;
    double best_cost = std::isfinite(r.cost) ? r.cost : std::numeric_limits<double>::infinity();
    for (int d : g.delays)
      for (int f : g.fractions)
        for (int s : g.smooths) {
          CfdParams p{d, f, s, arms ? r.arm.interp : r.t.interp};
          double c = arms ? cost(r.t, p) : cost(p, r.arm);
          ++r.evaluations;
          if (std::isfinite(c) && c < best_cost - 1e-12) {
            best_cost = c;
            best = p;
          }
        }
    bool changed = best.delay != (arms ? r.arm : r.t).delay ||
                   best.fraction_pct != (arms ? r.arm : r.t).fraction_pct ||
                   best.smooth != (arms ? r.arm : r.t).smooth;
    (arms ? r.arm : r.t) = best;
    r.cost = best_cost;
    return changed;
  };
  for (r.rounds = 1; r.rounds <= max_rounds; ++r.rounds) {
    bool a = best_of(true);
    bool t = best_of(false);
    if (!a && !t) break;
  }
  if (r.rounds > max_rounds) r.rounds = max_rounds;
  return r;
}

// ---------------------------------------------------------------------------
// 6. Result as a digitizer-JSON patch
// ---------------------------------------------------------------------------

struct PatchEntry {
  int module;
  int channel;
  CfdParams p;
};

/// {"<module>": {"channel_overrides": {"<ch>": {"cfd_delay_ns": .., "cfd_fraction": "CFD_FRACTLIST_..",
///  "input_smoothing": "CFD_SMOOTH_EXP_.."}}}} — the keys of the Operator's digitizer JSON.
inline std::string patch_json(const std::vector<PatchEntry>& entries, double ns_per_sample) {
  std::map<int, std::map<int, CfdParams>> by;
  for (const auto& e : entries) by[e.module][e.channel] = e.p;
  std::string s = "{\n";
  bool first_m = true;
  for (const auto& [m, chans] : by) {
    s += std::string(first_m ? "" : ",\n") + "  \"" + std::to_string(m) + "\": {\"channel_overrides\": {\n";
    first_m = false;
    bool first_c = true;
    for (const auto& [c, p] : chans) {
      char b[256];
      std::snprintf(b, sizeof(b),
                    "%s    \"%d\": {\"cfd_delay_ns\": %g, \"cfd_fraction\": \"CFD_FRACTLIST_%d\", "
                    "\"input_smoothing\": \"CFD_SMOOTH_EXP_%d\"}",
                    first_c ? "" : ",\n", c, p.delay * ns_per_sample, p.fraction_pct, p.smooth);
      s += b;
      first_c = false;
    }
    s += "\n  }}";
  }
  s += "\n}\n";
  return s;
}

}  // namespace cfdtune

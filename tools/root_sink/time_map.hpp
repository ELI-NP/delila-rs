// time_map.hpp — pure helpers for time_map.C (TODO 72 board time offsets,
// TODO 71 Phase D anti-Compton pairing). No ROOT: unit-tested in
// test_time_map.cpp, driven by time_map.C (which only reads the hits tree and
// prints). See the top of time_map.C for the usage and the conventions.
//
// Conventions (shared with time_offsets.hpp / timeSettings.json):
//   * dt = t(channel) - t(reference)  (positive = the channel arrives LATE),
//     so a measured peak IS the timeSettings offset_ns (aligned = raw - offset).
//   * Anti-Compton: dt = t(shield) - t(detector), the event_def v1.1 window is
//     relative to the detector hit: shield in [t_det - pre, t_det + post].
//   * Hardware start delay (DIG1 start_delay 16 ns, DIG2 rundelay 8 ns): with
//     sign = +1 (default), increasing a board's delay by D makes that board's
//     timestamps SMALLER by D (its counter starts D later) — see time_map.C.
//
// Coincidences are counted over ALL pairs within +-W (every reference hit in
// the window, not only the nearest): the accidental background is then exactly
// flat, which the side-band subtraction relies on. "pairing=nearest" is kept
// as an option (one entry per channel hit; its background is flat only while
// rate_ref * 2W << 1).
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef ROOTSINK_TIME_MAP_HPP
#define ROOTSINK_TIME_MAP_HPP

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <map>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "event_def.hpp"  // edef_detail::parse_tags ("@def.json:tag" specs); time_offsets.hpp

namespace rootsink {
namespace timemap {

// ===========================================================================
// 1. Hit times per (module, channel)
// ===========================================================================
struct HitTimes {
  std::map<uint16_t, std::vector<double>> t;  // mc_key -> times (sorted after sort_all)
  long long n = 0;
  double t_min = std::numeric_limits<double>::infinity();
  double t_max = -std::numeric_limits<double>::infinity();

  void add(uint16_t key, double ts) {
    t[key].push_back(ts);
    ++n;
    if (ts < t_min) t_min = ts;
    if (ts > t_max) t_max = ts;
  }
  void sort_all() {
    for (auto& kv : t) std::sort(kv.second.begin(), kv.second.end());
  }
  std::set<uint16_t> keys() const {
    std::set<uint16_t> s;
    for (const auto& kv : t)
      if (!kv.second.empty()) s.insert(kv.first);
    return s;
  }
  long long hits(uint16_t key) const {
    auto it = t.find(key);
    return it == t.end() ? 0 : static_cast<long long>(it->second.size());
  }
  const std::vector<double>& of(uint16_t key) const {
    static const std::vector<double> empty;
    auto it = t.find(key);
    return it == t.end() ? empty : it->second;
  }
  double span_s() const { return n > 1 ? (t_max - t_min) * 1e-9 : 0.0; }
};

// ===========================================================================
// 2. Spec parsing: "M:C" lists, groups, steps, pulsers, options
// ===========================================================================
inline std::string trim(const std::string& s) {
  std::size_t a = s.find_first_not_of(" \t\r\n");
  if (a == std::string::npos) return "";
  std::size_t b = s.find_last_not_of(" \t\r\n");
  return s.substr(a, b - a + 1);
}

inline std::vector<std::string> split(const std::string& s, char sep) {
  std::vector<std::string> out;
  std::size_t start = 0;
  while (true) {
    std::size_t p = s.find(sep, start);
    out.push_back(s.substr(start, p == std::string::npos ? std::string::npos : p - start));
    if (p == std::string::npos) break;
    start = p + 1;
  }
  return out;
}

// Decimal integer in [lo, hi] (digits only, optional leading '-').
inline bool parse_int(const std::string& s0, long lo, long hi, long& out) {
  std::string s = trim(s0);
  if (s.empty() || s.size() > 12) return false;
  std::size_t i = (s[0] == '-') ? 1 : 0;
  if (i == s.size()) return false;
  for (std::size_t k = i; k < s.size(); ++k)
    if (s[k] < '0' || s[k] > '9') return false;
  long v = std::stol(s);
  if (v < lo || v > hi) return false;
  out = v;
  return true;
}

// "M:C" -> mc_key.
inline bool parse_mc(const std::string& s, uint16_t& key) {
  std::vector<std::string> p = split(trim(s), ':');
  long m = 0, c = 0;
  if (p.size() != 2 || !parse_int(p[0], 0, 255, m) || !parse_int(p[1], 0, 255, c)) return false;
  key = mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(c));
  return true;
}

inline std::string mc_str(uint16_t key) {  // "9:0" (the spec syntax)
  return std::to_string(key >> 8) + ":" + std::to_string(key & 0xFF);
}

// Channel list: comma-separated "M:C" | "M:A-B" (inclusive range, every
// channel whether it has hits or not) | "M:*" (the channels of module M that
// are in `present`). Appends to `out`; a channel listed twice is an error.
inline bool parse_mc_list(const std::string& s, const std::set<uint16_t>& present,
                          std::vector<uint16_t>& out, std::string& err) {
  std::set<uint16_t> seen(out.begin(), out.end());
  auto push = [&](uint16_t k) {
    if (!seen.insert(k).second) {
      err = "channel " + mc_str(k) + " listed twice";
      return false;
    }
    out.push_back(k);
    return true;
  };
  if (trim(s).empty()) {
    err = "empty channel list";
    return false;
  }
  for (const std::string& item0 : split(s, ',')) {
    std::string item = trim(item0);
    std::vector<std::string> p = split(item, ':');
    long m = 0;
    if (p.size() != 2 || !parse_int(p[0], 0, 255, m)) {
      err = "bad channel '" + item + "' (expected M:C, M:A-B or M:*)";
      return false;
    }
    std::string c = trim(p[1]);
    if (c == "*") {
      bool any = false;
      for (uint16_t k : present)
        if ((k >> 8) == m) {
          any = true;
          if (!push(k)) return false;
        }
      if (!any) {
        err = "'" + item + "': module " + std::to_string(m) + " has no hits";
        return false;
      }
      continue;
    }
    std::size_t dash = c.find('-');
    long a = 0, b = 0;
    if (dash == std::string::npos) {
      if (!parse_int(c, 0, 255, a)) {
        err = "bad channel '" + item + "'";
        return false;
      }
      b = a;
    } else if (!parse_int(c.substr(0, dash), 0, 255, a) ||
               !parse_int(c.substr(dash + 1), 0, 255, b) || a > b) {
      err = "bad range '" + item + "' (expected M:A-B with A <= B)";
      return false;
    }
    for (long ch = a; ch <= b; ++ch)
      if (!push(mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(ch)))) return false;
  }
  return true;
}

struct Group {
  std::string name;
  std::vector<uint16_t> chans;
};

// Event-definition tags -> groups: `json_text` is an event_def (v1.1 "tags"
// table; the rest of the file is not checked), `names` the tags to use.
inline bool groups_from_tags(const std::string& json_text, const std::vector<std::string>& names,
                             std::vector<Group>& out, std::string& err) {
  tdelila::json::Value root;
  try {
    root = tdelila::json::Parser(json_text).parse();
  } catch (const std::exception& e) {
    err = std::string("malformed JSON (") + e.what() + ")";
    return false;
  }
  EventDef d;
  std::vector<std::string> errors;
  edef_detail::parse_tags(root, d, errors);
  if (!errors.empty()) {
    err = errors.front();
    return false;
  }
  for (const std::string& n0 : names) {
    std::string n = trim(n0);
    auto it = d.tags.find(n);
    if (it == d.tags.end()) {
      err = "unknown tag '" + n + "'" + (d.tags.empty() ? " (the file has no 'tags' table)" : "");
      return false;
    }
    out.push_back(Group{n, it->second});
  }
  return true;
}

// Detector groups:
//   "name=M:C,M:C;name2=M:C-D"  one group per ';' item
//   "M:C,M:C-D"                 one group per channel, named m<M>c<C>
//   "@def.json:tagA,tagB"       one group per event_def tag (file read by caller:
//                               pass its text as `tag_file_text`)
// A channel in two groups is an error.
inline bool parse_groups(const std::string& spec0, const std::set<uint16_t>& present,
                         const std::string& tag_file_text, std::vector<Group>& out,
                         std::string& err) {
  std::string spec = trim(spec0);
  out.clear();
  if (spec.empty()) {
    err = "empty group spec";
    return false;
  }
  if (spec[0] == '@') {
    std::size_t colon = spec.rfind(':');
    if (colon == std::string::npos || colon + 1 >= spec.size()) {
      err = "'@file:tag,tag' needs the tag names after ':'";
      return false;
    }
    if (!groups_from_tags(tag_file_text, split(spec.substr(colon + 1), ','), out, err))
      return false;
  } else if (spec.find('=') != std::string::npos) {
    std::set<std::string> names;
    for (const std::string& item0 : split(spec, ';')) {
      std::string item = trim(item0);
      if (item.empty()) continue;  // tolerate a trailing ';'
      std::size_t eq = item.find('=');
      if (eq == std::string::npos) {
        err = "group '" + item + "' has no 'name='";
        return false;
      }
      Group g;
      g.name = trim(item.substr(0, eq));
      if (g.name.empty()) {
        err = "empty group name in '" + item + "'";
        return false;
      }
      if (!names.insert(g.name).second) {
        err = "group '" + g.name + "' defined twice";
        return false;
      }
      if (!parse_mc_list(item.substr(eq + 1), present, g.chans, err)) {
        err = "group '" + g.name + "': " + err;
        return false;
      }
      out.push_back(g);
    }
  } else {
    std::vector<uint16_t> ks;
    if (!parse_mc_list(spec, present, ks, err)) return false;
    for (uint16_t k : ks) out.push_back(Group{mc_name(k), {k}});
  }
  if (out.empty()) {
    err = "no groups";
    return false;
  }
  std::map<uint16_t, std::string> owner;
  for (const Group& g : out)
    for (uint16_t k : g.chans) {
      auto it = owner.find(k);
      if (it != owner.end()) {
        err = "channel " + mc_str(k) + " is in groups '" + it->second + "' and '" + g.name + "'";
        return false;
      }
      owner[k] = g.name;
    }
  return true;
}

// Hardware delay step per module: "" (16 everywhere), "8" (8 everywhere),
// "16,10=8,11=8" (default 16, modules 10 and 11 use 8).
struct StepSpec {
  int def = 16;
  std::map<int, int> per_module;
  int of(int module) const {
    auto it = per_module.find(module);
    return it == per_module.end() ? def : it->second;
  }
};

inline bool parse_steps(const std::string& s, StepSpec& st, std::string& err) {
  st = StepSpec{};
  if (trim(s).empty()) return true;
  bool def_seen = false;
  for (const std::string& item0 : split(s, ',')) {
    std::string item = trim(item0);
    std::size_t eq = item.find('=');
    long m = 0, v = 0;
    if (eq == std::string::npos) {
      if (def_seen || !parse_int(item, 1, 65536, v)) {
        err = "bad step '" + item + "' (one default step N > 0, then M=N items)";
        return false;
      }
      def_seen = true;
      st.def = static_cast<int>(v);
    } else {
      if (!parse_int(item.substr(0, eq), 0, 255, m) || !parse_int(item.substr(eq + 1), 1, 65536, v)) {
        err = "bad step '" + item + "' (expected M=N, N > 0 ns)";
        return false;
      }
      if (st.per_module.count(static_cast<int>(m))) {
        err = "step for module " + std::to_string(m) + " given twice";
        return false;
      }
      st.per_module[static_cast<int>(m)] = static_cast<int>(v);
    }
  }
  return true;
}

// Pulser channel per module: "" -> the LAST (highest) channel of every module
// present in the data (TODO 72 §5: the last channel of each board carries the
// common pulser); otherwise "M:C,M:C,..." with at most one per module.
inline bool parse_pulsers(const std::string& s, const std::set<uint16_t>& present,
                          std::map<int, uint16_t>& out, std::string& err) {
  out.clear();
  if (trim(s).empty()) {
    for (uint16_t k : present) out[k >> 8] = k;  // ascending: the last one wins
    return true;
  }
  std::vector<uint16_t> ks;
  if (!parse_mc_list(s, present, ks, err)) return false;
  for (uint16_t k : ks) {
    if (out.count(k >> 8)) {
      err = "two pulser channels for module " + std::to_string(k >> 8);
      return false;
    }
    out[k >> 8] = k;
  }
  return true;
}

// Options "key=value key=value" (whitespace- or ';'-separated). Unknown keys are
// an error, so a typo is never silently ignored.
struct Options {
  std::map<std::string, std::string> kv;
  bool has(const std::string& k) const { return kv.count(k) != 0; }
  std::string str(const std::string& k, const std::string& def) const {
    auto it = kv.find(k);
    return it == kv.end() ? def : it->second;
  }
};

inline bool parse_options(const std::string& s, const std::set<std::string>& allowed, Options& o,
                          std::string& err) {
  o.kv.clear();
  std::string cur;
  std::vector<std::string> items;
  for (char c : s) {
    if (c == ' ' || c == '\t' || c == ';' || c == '\n') {
      if (!cur.empty()) items.push_back(cur);
      cur.clear();
    } else {
      cur += c;
    }
  }
  if (!cur.empty()) items.push_back(cur);
  for (const std::string& it : items) {
    std::size_t eq = it.find('=');
    if (eq == std::string::npos || eq == 0) {
      err = "option '" + it + "' is not key=value";
      return false;
    }
    std::string k = it.substr(0, eq);
    if (!allowed.count(k)) {
      std::string a;
      for (const std::string& x : allowed) a += (a.empty() ? "" : " ") + x;
      err = "unknown option '" + k + "' (known: " + a + ")";
      return false;
    }
    if (o.kv.count(k)) {
      err = "option '" + k + "' given twice";
      return false;
    }
    o.kv[k] = it.substr(eq + 1);
  }
  return true;
}

// Numeric option in [lo, hi]; absent -> `def`.
inline bool opt_num(const Options& o, const std::string& k, double def, double lo, double hi,
                    double& out, std::string& err) {
  out = def;
  if (!o.has(k)) return true;
  const std::string v = o.str(k, "");
  char* end = nullptr;
  double x = std::strtod(v.c_str(), &end);
  if (v.empty() || !end || *end != '\0' || !std::isfinite(x) || x < lo || x > hi) {
    char buf[160];
    std::snprintf(buf, sizeof buf, "option %s=%s: expected a number in [%g, %g]", k.c_str(),
                  v.c_str(), lo, hi);
    err = buf;
    return false;
  }
  out = x;
  return true;
}

// ===========================================================================
// 3. Δt histogram + filling
// ===========================================================================
struct DtHist {
  double W = 0.0;    // range [-W, +W]
  double bin = 1.0;  // adjusted so that nbins * bin == 2W exactly
  std::vector<double> n;
  long long entries = 0;

  void init(double W_, double bin_) {
    W = W_;
    int nb = static_cast<int>(std::llround(2.0 * W_ / bin_));
    if (nb < 1) nb = 1;
    bin = 2.0 * W_ / nb;
    n.assign(static_cast<std::size_t>(nb), 0.0);
    entries = 0;
  }
  int nbins() const { return static_cast<int>(n.size()); }
  double center(int i) const { return -W + (i + 0.5) * bin; }
  double left(int i) const { return -W + i * bin; }
  void fill(double dt) {
    if (!(dt >= -W && dt <= W)) return;
    int i = static_cast<int>(std::floor((dt + W) / bin));
    if (i >= nbins()) i = nbins() - 1;  // dt == +W
    if (i < 0) i = 0;
    n[static_cast<std::size_t>(i)] += 1.0;
    ++entries;
  }
  void add(const DtHist& o) {
    for (std::size_t i = 0; i < n.size() && i < o.n.size(); ++i) n[i] += o.n[i];
    entries += o.entries;
  }
};

// Every pair (x, r) with |x - r| <= W -> fill x - r. Both vectors sorted.
// Two-pointer sweep: O(|x| + |ref| + pairs).
inline void fill_all_pairs(const std::vector<double>& x, const std::vector<double>& ref,
                           DtHist& h) {
  const double W = h.W;
  std::size_t lo = 0;
  for (double t : x) {
    while (lo < ref.size() && ref[lo] < t - W) ++lo;
    for (std::size_t j = lo; j < ref.size() && ref[j] <= t + W; ++j) h.fill(t - ref[j]);
  }
}

// Each x hit against its NEAREST ref hit (ties -> the later ref hit, as
// eb_core.hpp nearest_in), filled when within +-W.
inline void fill_nearest(const std::vector<double>& x, const std::vector<double>& ref, DtHist& h) {
  if (ref.empty()) return;
  std::size_t idx = 0;  // first ref >= t (monotone in t)
  for (double t : x) {
    while (idx < ref.size() && ref[idx] < t) ++idx;
    double best;
    if (idx == ref.size())
      best = t - ref[idx - 1];
    else if (idx > 0 && (t - ref[idx - 1]) < (ref[idx] - t))
      best = t - ref[idx - 1];
    else
      best = t - ref[idx];
    if (std::fabs(best) <= h.W) h.fill(best);
  }
}

// Several x channels against one ref in one sweep: `stream` = (time, index)
// sorted by time; fills hs[index] with x - ref for all pairs within +-W.
inline void fill_stream(const std::vector<std::pair<double, int>>& stream,
                        const std::vector<double>& ref, std::vector<DtHist>& hs) {
  if (hs.empty()) return;
  const double W = hs[0].W;
  std::size_t lo = 0;
  for (double r : ref) {
    while (lo < stream.size() && stream[lo].first < r - W) ++lo;
    for (std::size_t j = lo; j < stream.size() && stream[j].first <= r + W; ++j)
      hs[static_cast<std::size_t>(stream[j].second)].fill(stream[j].first - r);
  }
}

// ===========================================================================
// 4. Peak over a flat accidental background
// ===========================================================================
struct PeakOpts {
  double nsigma = 5.0;  // significance threshold of the background-subtracted signal
  double search = 0.5;  // peak searched in |dt| < search * W
  double sb = 0.75;     // side bands |dt| >= sb * W give the flat accidental level
  double cover = 0.90;  // central fraction of the excess reported as [q_lo, q_hi]
  double multi = 0.5;   // a 2nd significant peak with >= multi x the 1st's signal: ambiguous
};

struct Peak {
  bool candidate = false;    // a positive excess was located
  bool significant = false;  // z >= nsigma, signal > 0, not cut by the search edge, single peak
  double mode = 0, centroid = 0, fwhm = 0, lo_half = 0, hi_half = 0;
  double median = 0;                    // 50 % quantile of the excess (robust for plateaus)
  double q_lo = 0, q_hi = 0;            // central `cover` of the excess (bg-subtracted)
  double region_lo = 0, region_hi = 0;  // excess > max/20 (see find_peak), within search
  double signal = 0, signal_err = 0, z = 0;
  double bg_per_bin = 0, bg_err = 0;  // accidental level per bin (and its error)
  double sb_asym_z = 0;               // (left - right side band) / sigma
  double sb_asym_rel = 0;             // (left - right) / mean of the two
  bool flat = true;                   // false: |sb_asym_z| > 5 AND |sb_asym_rel| > 10 % (a note only)
  bool multiple = false;              // a 2nd significant peak >= multi x the 1st (not used)
  double second_at = 0, second_ratio = 0, second_z = 0;  // best other peak (outside the region)
  long long entries = 0;
  int scale = 0;         // boxcar width (bins) that maximised the detection scan
  bool at_edge = false;  // half maximum not reached inside the search region
  std::string note;
};

// Algorithm:
//  1. accidental level beta = mean of the side-band bins (Poisson error); a
//     non-flat background (left vs right side band: > 5 sigma AND > 10 %) is
//     noted, not rejected (dead-time dips / after-pulse bumps beside a strong
//     peak are harmless);
//  2. detection: boxcars of 1, 2, 4, ... bins slid over the search region; the
//     window maximising z = (N - k beta) / sqrt(N + k^2 var(beta)) gives the
//     position and the scale k (sharp pulser and broad HPGe peaks alike);
//  3. mode = maximum of the excess near that window, smoothed only as much as
//     the statistics need (<= k/4 bins), refined by a parabola; FWHM from the
//     half-maximum crossings of the same smoothed excess (interpolated);
//  4. region = where the excess smoothed over ~1 FWHM stays above 1/20 of its
//     maximum (so a walk tail is included), at least FWHM/2 beyond the
//     half-maximum points: signal = sum(n - beta), centroid = excess-weighted
//     mean, median = 50 % quantile of the excess, z with the background error,
//     and the central `cover` quantiles;
//  5. the scan repeated outside the region: a second significant peak with
//     >= `multi` x the first's signal makes the result ambiguous (not used).
inline Peak find_peak(const DtHist& h, const PeakOpts& o) {
  Peak p;
  p.entries = h.entries;
  const int nb = h.nbins();
  const double W = h.W;
  double sumL = 0, sumR = 0;
  int nL = 0, nR = 0, s0 = -1, s1 = -1;
  for (int i = 0; i < nb; ++i) {
    double c = h.center(i);
    if (c <= -o.sb * W) {
      sumL += h.n[static_cast<std::size_t>(i)];
      ++nL;
    } else if (c >= o.sb * W) {
      sumR += h.n[static_cast<std::size_t>(i)];
      ++nR;
    }
    if (std::fabs(c) < o.search * W) {
      if (s0 < 0) s0 = i;
      s1 = i;
    }
  }
  if (nL + nR == 0 || s0 < 0 || s1 - s0 + 1 < 4) {
    p.note = "no side bands or search region (check bin/search/sb)";
    return p;
  }
  const double beta = (sumL + sumR) / (nL + nR);
  const double varb = beta / (nL + nR);
  p.bg_per_bin = beta;
  p.bg_err = std::sqrt(varb);
  if (nL > 0 && nR > 0) {
    double bl = sumL / nL, br = sumR / nR;
    double den = std::sqrt(bl / nL + br / nR);
    p.sb_asym_z = den > 0 ? (bl - br) / den : 0.0;
    p.sb_asym_rel = (bl + br) > 0 ? 2.0 * (bl - br) / (bl + br) : 0.0;
    // Reported, not used to reject: real data has dead-time dips and
    // after-pulse bumps next to a strong peak (BGO), harmless at that level.
    p.flat = !(std::fabs(p.sb_asym_z) > 5 && std::fabs(p.sb_asym_rel) > 0.10);
  }
  if (h.entries == 0) {
    p.note = "empty";
    return p;
  }
  // 2. multi-scale boxcar scan
  const int ns = s1 - s0 + 1;
  std::vector<double> P(static_cast<std::size_t>(ns) + 1, 0.0);
  for (int k = 0; k < ns; ++k)
    P[static_cast<std::size_t>(k) + 1] = P[static_cast<std::size_t>(k)] + h.n[static_cast<std::size_t>(s0 + k)];
  // Best boxcar over the search region, skipping boxes that touch [x0, x1]
  // (bin indices relative to s0; x0 > x1 = nothing skipped).
  auto scan = [&](int x0, int x1, double& bestZ, int& bj, int& bk, double& bS) {
    bestZ = 0;
    bj = -1;
    bk = 0;
    bS = 0;
    for (int k = 1; k <= std::max(1, ns / 4); k *= 2) {
      for (int j = 0; j + k <= ns; ++j) {
        if (x0 <= x1 && j <= x1 && j + k - 1 >= x0) continue;
        double N = P[static_cast<std::size_t>(j + k)] - P[static_cast<std::size_t>(j)];
        double S = N - k * beta;
        double var = N + double(k) * k * varb;
        if (S <= 0 || var <= 0) continue;
        double Z = S / std::sqrt(var);
        if (Z > bestZ) {
          bestZ = Z;
          bj = j;
          bk = k;
          bS = S;
        }
      }
    }
  };
  double bestZ = 0, bS = 0;
  int bj = -1, bk = 0;
  scan(1, 0, bestZ, bj, bk, bS);
  if (bj < 0) {
    p.note = "no excess over the accidental level";
    return p;
  }
  p.candidate = true;
  p.scale = bk;
  // 3. smoothed excess, mode, half-maximum crossings. Smoothing only as much as
  //    the statistics need (~10 % relative noise per smoothed bin at the peak
  //    level), at most k/4 bins: a fixed wide boxcar would drag the mode of an
  //    asymmetric (walk-tailed) peak towards its tail.
  const double hbar = (P[static_cast<std::size_t>(bj + bk)] - P[static_cast<std::size_t>(bj)]) / bk - beta;
  const double w_need = hbar > 0 ? 100.0 * (hbar + beta) / (hbar * hbar) : 1.0;
  int m = static_cast<int>(std::ceil((std::min(w_need, 1e6) - 1.0) / 2.0));
  m = std::max(0, std::min(m, bk / 4));
  auto smooth = [&](int half, std::vector<double>& out) {
    out.assign(static_cast<std::size_t>(nb), 0.0);
    for (int i = 0; i < nb; ++i) {
      double s = 0;
      int cnt = 0;
      for (int d = -half; d <= half; ++d) {
        int q = i + d;
        if (q < 0 || q >= nb) continue;
        s += h.n[static_cast<std::size_t>(q)];
        ++cnt;
      }
      out[static_cast<std::size_t>(i)] = s / cnt - beta;
    }
  };
  std::vector<double> e;
  smooth(m, e);
  auto E = [&](int i) { return e[static_cast<std::size_t>(i)]; };
  int a0 = std::max(s0, s0 + bj - bk), a1 = std::min(s1, s0 + bj + 2 * bk - 1);
  int imax = a0;
  for (int i = a0; i <= a1; ++i)
    if (E(i) > E(imax)) imax = i;
  double mode = h.center(imax);
  if (imax - 1 >= s0 && imax + 1 <= s1) {
    double y0 = E(imax - 1), y1 = E(imax), y2 = E(imax + 1);
    double den = y0 - 2 * y1 + y2;
    if (den < 0) {
      double d = 0.5 * (y0 - y2) / den;
      if (d > 0.5) d = 0.5;
      if (d < -0.5) d = -0.5;
      mode += d * h.bin;
    }
  }
  p.mode = mode;
  const double H = E(imax) / 2;
  if (!(H > 0)) {
    p.note = "smoothed excess not positive at the maximum";
    p.candidate = false;
    return p;
  }
  int i = imax;
  while (i - 1 >= s0 && E(i - 1) >= H) --i;
  double lo;
  if (i - 1 < s0) {
    p.at_edge = true;
    lo = h.left(s0);
  } else {
    lo = h.center(i - 1) + (H - E(i - 1)) / (E(i) - E(i - 1)) * h.bin;
  }
  i = imax;
  while (i + 1 <= s1 && E(i + 1) >= H) ++i;
  double hi;
  if (i + 1 > s1) {
    p.at_edge = true;
    hi = h.left(s1) + h.bin;
  } else {
    hi = h.center(i) + (E(i) - H) / (E(i) - E(i + 1)) * h.bin;
  }
  p.lo_half = lo;
  p.hi_half = hi;
  p.fwhm = hi - lo;
  // 4. region = where the excess, smoothed over ~1 FWHM, stays above 1/20 of
  //    its maximum (follows a walk tail), at least [lo_half - FWHM/2,
  //    hi_half + FWHM/2]; clipped to the search region.
  {
    std::vector<double> et;
    const int mt = std::max(1, static_cast<int>(std::lround(0.5 * p.fwhm / h.bin)));
    smooth(mt, et);
    auto ET = [&](int q) { return et[static_cast<std::size_t>(q)]; };
    double Ht = ET(imax);
    for (int q = imax - 1; q <= imax + 1; ++q)
      if (q >= s0 && q <= s1) Ht = std::max(Ht, ET(q));
    const double thr = 0.05 * Ht;
    int a = imax, b = imax;
    while (a - 1 >= s0 && ET(a - 1) >= thr) --a;
    while (b + 1 <= s1 && ET(b + 1) >= thr) ++b;
    p.region_lo = std::min(h.left(a), lo - 0.5 * p.fwhm);
    p.region_hi = std::max(h.left(b) + h.bin, hi + 0.5 * p.fwhm);
  }
  p.region_lo = std::max(p.region_lo, h.left(s0));
  p.region_hi = std::min(p.region_hi, h.left(s1) + h.bin);
  double S = 0, N = 0, sx = 0;
  int nreg = 0, r0 = -1, r1 = -1;
  for (int q = s0; q <= s1; ++q) {
    double c = h.center(q);
    if (c < p.region_lo || c > p.region_hi) continue;
    if (r0 < 0) r0 = q;
    r1 = q;
    double w = h.n[static_cast<std::size_t>(q)] - beta;
    S += w;
    N += h.n[static_cast<std::size_t>(q)];
    sx += w * c;
    ++nreg;
  }
  double var = N + double(nreg) * nreg * varb;
  p.signal = S;
  p.signal_err = std::sqrt(std::max(0.0, var));
  p.z = var > 0 ? S / std::sqrt(var) : 0.0;
  p.centroid = S > 0 ? sx / S : mode;
  if (S > 0 && r0 >= 0) {
    const double ta = 0.5 * (1.0 - o.cover) * S, tb = 0.5 * (1.0 + o.cover) * S, tm = 0.5 * S;
    double cum = 0;
    bool fa = false, fb = false, fm = false;
    for (int q = r0; q <= r1; ++q) {
      double w = h.n[static_cast<std::size_t>(q)] - beta;
      double next = cum + w;
      auto at = [&](double target) {
        double f = w > 0 ? (target - cum) / w : 1.0;
        if (f < 0) f = 0;
        if (f > 1) f = 1;
        return h.left(q) + f * h.bin;
      };
      if (!fa && next >= ta) {
        p.q_lo = at(ta);
        fa = true;
      }
      if (!fm && next >= tm) {
        p.median = at(tm);
        fm = true;
      }
      if (!fb && next >= tb) {
        p.q_hi = at(tb);
        fb = true;
      }
      cum = next;
    }
    if (!fa) p.q_lo = p.region_lo;
    if (!fm) p.median = p.centroid;
    if (!fb) p.q_hi = p.region_hi;
  }
  // 5. another peak outside the region? (a comb of re-trigger artifacts, two
  //    timing populations): comparable and significant -> ambiguous, not used.
  if (r0 >= 0) {
    double z2 = 0, S2 = 0;
    int j2 = -1, k2 = 0;
    scan(r0 - s0, r1 - s0, z2, j2, k2, S2);
    if (j2 >= 0) {
      p.second_z = z2;
      p.second_ratio = bS > 0 ? S2 / bS : 0.0;
      p.second_at = h.center(s0 + j2) + 0.5 * (k2 - 1) * h.bin;
      p.multiple = z2 >= o.nsigma && p.second_ratio >= o.multi;
    }
  }
  p.significant = S > 0 && p.z >= o.nsigma && !p.at_edge && !p.multiple;
  char buf[160];
  if (p.at_edge) {
    p.note = "peak runs into the search edge (widen window_ns)";
  } else if (p.multiple) {
    std::snprintf(buf, sizeof buf, "MULTIPLE peaks: another at %+.0f ns with %.0f %% of the signal (z %.1f)",
                  p.second_at, 100 * p.second_ratio, p.second_z);
    p.note = buf;
  } else if (!p.significant) {
    p.note = "not significant";
  }
  if (!p.flat) {
    std::snprintf(buf, sizeof buf, "side bands differ by %+.0f %% (z %.1f)", 100 * p.sb_asym_rel,
                  p.sb_asym_z);
    p.note += std::string(p.note.empty() ? "" : "; ") + buf;
  }
  return p;
}

// ===========================================================================
// 5. Per-channel offsets against one reference (time_map_offsets)
// ===========================================================================
// Which number of a peak is "the" offset. mode = most probable dt (fast, large
// signals; noisy on a flat-topped peak), median = 50 % of the excess (robust for
// any shape), centroid = mean (pulled by walk tails).
enum class PeakValue { Mode, Centroid, Median };

inline const char* peak_value_name(PeakValue v) {
  switch (v) {
    case PeakValue::Mode: return "mode";
    case PeakValue::Centroid: return "centroid";
    case PeakValue::Median: return "median";
  }
  return "?";
}

inline bool parse_peak_value(const std::string& s, PeakValue& v) {
  for (PeakValue x : {PeakValue::Mode, PeakValue::Centroid, PeakValue::Median})
    if (s == peak_value_name(x)) {
      v = x;
      return true;
    }
  return false;
}

struct MeasureOpts {
  double bin = 2.0;
  PeakOpts peak;
  bool nearest = false;                  // pairing=nearest
  PeakValue value = PeakValue::Median;   // peak=median|mode|centroid (median: robust on flat tops)
};

inline double peak_value(const Peak& p, PeakValue v) {
  switch (v) {
    case PeakValue::Centroid: return p.centroid;
    case PeakValue::Median: return p.median;
    case PeakValue::Mode: break;
  }
  return p.mode;
}

struct ChannelPeak {
  uint16_t key = 0;
  long long hits = 0;
  Peak p;
};

inline Peak measure_pair(const std::vector<double>& x, const std::vector<double>& ref, double W,
                         const MeasureOpts& o) {
  DtHist h;
  h.init(W, o.bin);
  if (o.nearest)
    fill_nearest(x, ref, h);
  else
    fill_all_pairs(x, ref, h);
  return find_peak(h, o.peak);
}

// Every channel with hits except `ref` and `skip`, in key order. With
// `board_sum`, also the peak of each module's SUMMED histogram (its measured
// channels together: a board offset even when no single channel has the
// statistics).
inline std::vector<ChannelPeak> measure_vs_ref(const HitTimes& ht, uint16_t ref, double W,
                                               const MeasureOpts& o,
                                               const std::set<uint16_t>& skip = {},
                                               std::map<int, Peak>* board_sum = nullptr) {
  std::vector<ChannelPeak> out;
  const std::vector<double>& r = ht.of(ref);
  std::map<int, DtHist> sums;
  for (const auto& kv : ht.t) {
    if (kv.first == ref || skip.count(kv.first) || kv.second.empty()) continue;
    ChannelPeak cp;
    cp.key = kv.first;
    cp.hits = static_cast<long long>(kv.second.size());
    DtHist h;
    h.init(W, o.bin);
    if (o.nearest)
      fill_nearest(kv.second, r, h);
    else
      fill_all_pairs(kv.second, r, h);
    cp.p = find_peak(h, o.peak);
    out.push_back(cp);
    if (board_sum) {
      auto it = sums.find(kv.first >> 8);
      if (it == sums.end())
        sums[kv.first >> 8] = h;
      else
        it->second.add(h);
    }
  }
  if (board_sum) {
    board_sum->clear();
    for (const auto& kv : sums) (*board_sum)[kv.first] = find_peak(kv.second, o.peak);
  }
  return out;
}

inline double median(std::vector<double> v) {
  if (v.empty()) return 0.0;
  std::sort(v.begin(), v.end());
  std::size_t k = v.size() / 2;
  return (v.size() % 2) ? v[k] : 0.5 * (v[k - 1] + v[k]);
}

// Board (module) summary of the significant per-channel peaks. The reference
// channel counts as a detector at exactly 0 on its own board.
struct BoardDet {
  int module = 0;
  int n_chan = 0;  // channels measured (ref included)
  int n_sig = 0;   // significant ones (ref included)
  double median = 0, min = 0, max = 0;
  bool ok = false;
  bool from_sum = false;  // no significant channel: the board's summed histogram
  Peak sum;               // that summed peak (when board sums were measured)
  double max_fwhm = 0;    // widest significant channel peak
  std::vector<double> values;
};

// Median of the significant channel peaks per module (the ref counts as a
// detector at exactly 0 on its board). A module without a significant channel
// falls back to its summed histogram (`sums`, may be null) when that one is
// significant — flagged `from_sum`.
inline std::map<int, BoardDet> board_detector_offsets(const std::vector<ChannelPeak>& cps,
                                                      uint16_t ref, PeakValue v,
                                                      const std::map<int, Peak>* sums = nullptr) {
  std::map<int, BoardDet> b;
  b[ref >> 8].values.push_back(0.0);
  b[ref >> 8].n_chan = 1;
  for (const ChannelPeak& cp : cps) {
    BoardDet& d = b[cp.key >> 8];
    ++d.n_chan;
    if (!cp.p.significant) continue;
    d.values.push_back(peak_value(cp.p, v));
    d.max_fwhm = std::max(d.max_fwhm, cp.p.fwhm);
  }
  for (auto& kv : b) {
    BoardDet& d = kv.second;
    d.module = kv.first;
    d.n_sig = static_cast<int>(d.values.size());
    if (sums) {
      auto it = sums->find(kv.first);
      if (it != sums->end()) d.sum = it->second;
    }
    d.ok = !d.values.empty();
    if (d.ok) {
      d.median = median(d.values);
      d.min = *std::min_element(d.values.begin(), d.values.end());
      d.max = *std::max_element(d.values.begin(), d.values.end());
    } else if (d.sum.significant) {
      d.ok = true;
      d.from_sum = true;
      d.median = d.min = d.max = peak_value(d.sum, v);
      d.max_fwhm = d.sum.fwhm;
    }
  }
  return b;
}

inline std::string fmt(const char* f, double x) {
  char buf[64];
  std::snprintf(buf, sizeof buf, f, x);
  return buf;
}

// timeSettings.json tree: `ref` = root (offset 0), every significant channel
// refs it with its peak. `input` (may be null) = the table the hits were
// aligned with before measuring: then offset = measured residual + the input
// offset relative to the ref, and a channel without a significant peak KEEPS its
// input relative offset (stated in _comment). Channels without a peak and not in
// the input are listed in _comment only — the file never drops one silently.
inline std::string offsets_tree_json(const std::vector<ChannelPeak>& cps, uint16_t ref,
                                     PeakValue v, const OffsetTable* input,
                                     const std::vector<std::string>& comments) {
  auto in_rel = [&](uint16_t k, bool& has) {
    has = false;
    if (!input) return 0.0;
    auto it = input->abs_ns.find(k);
    auto ir = input->abs_ns.find(ref);
    if (it == input->abs_ns.end()) return 0.0;
    has = true;
    return it->second - (ir == input->abs_ns.end() ? 0.0 : ir->second);
  };
  std::string none, kept;
  std::string entries = "    {\"module\": " + std::to_string(ref >> 8) + ", \"channel\": " +
                        std::to_string(ref & 0xFF) + ", \"ref\": null, \"offset_ns\": 0}";
  for (const ChannelPeak& cp : cps) {
    bool has = false;
    double base = in_rel(cp.key, has);
    double off;
    if (cp.p.significant) {
      off = peak_value(cp.p, v) + base;
    } else if (has) {
      off = base;
      kept += (kept.empty() ? "" : ", ") + mc_name(cp.key);
    } else {
      char buf[96];
      std::snprintf(buf, sizeof buf, "%s (%lld hits, z %.1f)", mc_name(cp.key).c_str(), cp.hits,
                    cp.p.z);
      none += (none.empty() ? "" : ", ") + std::string(buf);
      continue;
    }
    char buf[160];
    std::snprintf(buf, sizeof buf,
                  ",\n    {\"module\": %d, \"channel\": %d, \"ref\": [%d, %d], \"offset_ns\": %.3f}",
                  cp.key >> 8, cp.key & 0xFF, ref >> 8, ref & 0xFF, off + 0.0);
    entries += buf;
  }
  std::string js = "{\n  \"$schema\": \"../schemas/timeSettings.schema.json\",\n  \"_comment\": [\n";
  std::vector<std::string> c = comments;
  c.push_back("aligned_ts = raw_ts - offset_ns (positive = channel arrives late).");
  if (!kept.empty())
    c.push_back("no significant peak now, input offset KEPT: " + kept);
  c.push_back("NO significant peak, not listed (keeps its raw time; root_sink warns once per run): " +
              (none.empty() ? std::string("none") : none));
  for (std::size_t i = 0; i < c.size(); ++i)
    js += "    \"" + json_escape(c[i]) + "\"" + (i + 1 < c.size() ? ",\n" : "\n");
  js += "  ],\n  \"version\": \"1.0\",\n  \"entries\": [\n" + entries + "\n  ]\n}\n";
  return js;
}

// ===========================================================================
// 6. Board plan (time_map_boards) and its verification (time_map_verify)
// ===========================================================================
inline long long steps_of(double x, int step) { return std::llround(x / step); }

struct BoardRow {
  int module = 0;
  int step = 16;
  BoardDet det;
  double det_rel = 0;  // detector offset relative to the ref board
  bool has_pulser = false;
  uint16_t pulser = 0;
  long long pulser_hits = 0;
  Peak pulser_peak;
  bool pulser_ok = false;
  double pulser_dt = 0;  // t(pulser_m) - t(pulser_ref board)
  long long n_steps = 0;
  double delay_change = 0;  // ns, relative to the ref board (ref board = 0)
  double delay_nonneg = 0;  // same plan shifted so the smallest change is 0
  double ts_shift = 0;      // expected timestamp shift = -sign * delay_change
  double residual = 0;      // det_rel + ts_shift (|.| <= step/2 when det ok)
  double expected_pulser_after = 0;
  // The detector offset under each peak definition (index = PeakValue): when
  // their rounded steps differ, the plan depends on the definition (broad or
  // asymmetric peaks: HPGe walk, slow Si) — reported, the chosen one is used.
  bool alt_ok[3] = {false, false, false};
  double alt_det[3] = {0, 0, 0};
  long long alt_steps[3] = {0, 0, 0};
  bool definition_matters() const {
    if (!(alt_ok[0] && alt_ok[1] && alt_ok[2])) return false;
    return alt_steps[0] != alt_steps[1] || alt_steps[0] != alt_steps[2];
  }
};

struct BoardsPlan {
  uint16_t ref = 0;
  int ref_module = 0;
  int sign = 1;
  double W = 0, bin = 0;
  PeakValue value = PeakValue::Mode;
  std::vector<ChannelPeak> chans;  // detector channels (pulsers excluded)
  std::vector<BoardRow> rows;
  std::vector<std::string> warnings;
};

inline Peak measure_pulser_dt(const HitTimes& ht, uint16_t pulser, uint16_t ref_pulser, double W,
                              const MeasureOpts& o) {
  MeasureOpts po = o;
  po.nearest = false;
  return measure_pair(ht.of(pulser), ht.of(ref_pulser), W, po);
}

inline BoardsPlan plan_boards(const HitTimes& ht, uint16_t ref, double W,
                              const std::map<int, uint16_t>& pulsers, const StepSpec& steps,
                              int sign, const MeasureOpts& o) {
  BoardsPlan bp;
  bp.ref = ref;
  bp.ref_module = ref >> 8;
  bp.sign = sign >= 0 ? 1 : -1;
  bp.W = W;
  bp.bin = o.bin;
  bp.value = o.value;
  std::set<uint16_t> pset;
  for (const auto& kv : pulsers) pset.insert(kv.second);
  std::map<int, Peak> sums;
  bp.chans = measure_vs_ref(ht, ref, W, o, pset, &sums);
  std::map<int, BoardDet> det = board_detector_offsets(bp.chans, ref, o.value, &sums);
  std::map<int, BoardDet> alt[3];
  for (int v = 0; v < 3; ++v)
    alt[v] = board_detector_offsets(bp.chans, ref, static_cast<PeakValue>(v), &sums);
  std::set<int> modules;
  for (uint16_t k : ht.keys()) modules.insert(k >> 8);
  for (const auto& kv : pulsers) modules.insert(kv.first);
  const double ref_med = det[bp.ref_module].median;
  auto rp = pulsers.find(bp.ref_module);
  const bool ref_pulser = rp != pulsers.end() && ht.hits(rp->second) > 0;
  if (!ref_pulser)
    bp.warnings.push_back("the reference board m" + std::to_string(bp.ref_module) +
                          " has no pulser hits: no pulser differences can be measured");
  for (int m : modules) {
    BoardRow r;
    r.module = m;
    r.step = steps.of(m);
    auto dit = det.find(m);
    if (dit != det.end()) r.det = dit->second;
    r.det.module = m;
    for (int v = 0; v < 3; ++v) {
      auto ai = alt[v].find(m);
      if (ai == alt[v].end() || !ai->second.ok) continue;
      r.alt_ok[v] = true;
      r.alt_det[v] = ai->second.median - alt[v][bp.ref_module].median;
      r.alt_steps[v] = steps_of(r.alt_det[v], r.step);
    }
    if (r.det.ok) {
      r.det_rel = r.det.median - ref_med;
      r.n_steps = steps_of(r.det_rel, r.step);
      r.ts_shift = -static_cast<double>(r.n_steps * r.step);
      r.delay_change = bp.sign * static_cast<double>(r.n_steps * r.step);
      r.residual = r.det_rel + r.ts_shift;
    }
    auto pit = pulsers.find(m);
    if (pit != pulsers.end()) {
      r.has_pulser = true;
      r.pulser = pit->second;
      r.pulser_hits = ht.hits(pit->second);
      if (ref_pulser && r.pulser_hits > 0) {
        if (m == bp.ref_module) {
          r.pulser_ok = true;
          r.pulser_dt = 0;
        } else {
          r.pulser_peak = measure_pulser_dt(ht, r.pulser, rp->second, W, o);
          r.pulser_ok = r.pulser_peak.significant;
          r.pulser_dt = peak_value(r.pulser_peak, o.value);
        }
      }
    }
    r.expected_pulser_after = r.pulser_dt + r.ts_shift;
    bp.rows.push_back(r);
  }
  double mn = 0;
  for (const BoardRow& r : bp.rows) mn = std::min(mn, r.delay_change);
  for (BoardRow& r : bp.rows) r.delay_nonneg = r.delay_change - mn;
  return bp;
}

inline std::string json_num_or_null(bool ok, double x, const char* f = "%.3f") {
  return ok ? fmt(f, x + 0.0) : std::string("null");
}

// The plan as JSON: the input of time_map_verify (edit delay_change_ns if a
// different change was applied; only DIFFERENCES between boards matter).
inline std::string boards_json(const BoardsPlan& bp, const std::vector<std::string>& comments) {
  std::string js = "{\n  \"tool\": \"time_map_boards\",\n  \"version\": \"1.0\",\n  \"_comment\": [\n";
  std::vector<std::string> c = comments;
  c.push_back("dt = t(channel) - t(reference); detector_offset_ns = median of the board's "
              "significant detector peaks minus the ref board's (positive = board LATE); a board "
              "without a significant channel uses its summed histogram (detector_source).");
  c.push_back(bp.sign > 0
                  ? "sign = +1: increasing a board's start delay by D makes its timestamps SMALLER "
                    "by D (CAEN: the delay postpones the actual run start = timestamp reset)."
                  : "sign = -1: increasing a board's start delay by D makes its timestamps LARGER by D.");
  c.push_back("delay_change_ns = sign * step * round(detector_offset_ns / step), relative to the ref "
              "board; delay_change_nonneg_ns = the same plan + a common constant so that no change is "
              "negative. timestamp_shift_ns = -sign * delay_change_ns.");
  c.push_back("time_map_verify checks: (pulser_dt after - before) == -sign * (delay_change - "
              "delay_change of the ref board) within step/2.");
  for (std::size_t i = 0; i < c.size(); ++i)
    js += "    \"" + json_escape(c[i]) + "\"" + (i + 1 < c.size() ? ",\n" : "\n");
  char buf[1024];
  std::snprintf(buf, sizeof buf,
                "  ],\n  \"sign\": %d,\n  \"ref\": [%d, %d],\n  \"ref_module\": %d,\n  \"window_ns\": "
                "%.17g,\n  \"bin_ns\": %.17g,\n  \"peak\": \"%s\",\n  \"boards\": [\n",
                bp.sign, bp.ref >> 8, bp.ref & 0xFF, bp.ref_module, bp.W, bp.bin,
                peak_value_name(bp.value));
  js += buf;
  for (std::size_t i = 0; i < bp.rows.size(); ++i) {
    const BoardRow& r = bp.rows[i];
    std::string pul = r.has_pulser ? "[" + std::to_string(r.pulser >> 8) + ", " +
                                         std::to_string(r.pulser & 0xFF) + "]"
                                   : "null";
    std::snprintf(
        buf, sizeof buf,
        "    {\"module\": %d, \"step_ns\": %d, \"pulser\": %s, \"detector_offset_ns\": %s, "
        "\"detector_channels\": %d, \"detector_source\": %s, \"pulser_dt_ns\": %s, \"delay_change_ns\": %.0f, "
        "\"delay_change_nonneg_ns\": %.0f, \"timestamp_shift_ns\": %.0f, \"residual_ns\": %s, "
        "\"expected_pulser_dt_after_ns\": %s, \"detector_offset_by_peak_ns\": {\"mode\": %s, "
        "\"centroid\": %s, \"median\": %s}}",
        r.module, r.step, pul.c_str(), json_num_or_null(r.det.ok, r.det_rel).c_str(), r.det.n_sig,
        !r.det.ok ? "null" : r.det.from_sum ? "\"summed histogram\"" : "\"median of channels\"",
        json_num_or_null(r.pulser_ok, r.pulser_dt).c_str(), r.delay_change + 0.0,
        r.delay_nonneg + 0.0, r.ts_shift + 0.0, json_num_or_null(r.det.ok, r.residual).c_str(),
        json_num_or_null(r.pulser_ok, r.expected_pulser_after).c_str(),
        json_num_or_null(r.alt_ok[0], r.alt_det[0]).c_str(), json_num_or_null(r.alt_ok[1], r.alt_det[1]).c_str(),
        json_num_or_null(r.alt_ok[2], r.alt_det[2]).c_str());
    js += buf;
    js += (i + 1 < bp.rows.size()) ? ",\n" : "\n";
  }
  js += "  ]\n}\n";
  return js;
}

// What time_map_verify needs back from the plan file.
struct AppliedPlan {
  int sign = 1;
  int ref_module = 0;
  double W = 0, bin = 2.0;
  PeakValue value = PeakValue::Mode;
  struct Board {
    int module = 0;
    int step = 16;
    bool has_pulser = false;
    uint16_t pulser = 0;
    double delay_change = 0;
    bool has_before = false;
    double pulser_before = 0;
  };
  std::vector<Board> boards;
};

inline bool parse_applied(const std::string& text, AppliedPlan& ap, std::vector<std::string>& errors) {
  using JV = tdelila::json::Value;
  errors.clear();
  ap = AppliedPlan{};
  JV root;
  try {
    root = tdelila::json::Parser(text).parse();
  } catch (const std::exception& e) {
    errors.push_back(std::string("malformed JSON (") + e.what() + ")");
    return false;
  }
  if (root.t != JV::T::Obj) {
    errors.push_back("top level must be an object (time_map_boards output)");
    return false;
  }
  auto num = [&](const JV* v, const std::string& what, double& out) {
    if (!v || v->t != JV::T::Num || !std::isfinite(v->num)) {
      errors.push_back("'" + what + "' must be a finite number");
      return false;
    }
    out = v->num;
    return true;
  };
  double x = 0;
  if (num(root.find("sign"), "sign", x)) {
    if (x != 1 && x != -1) errors.push_back("'sign' must be +1 or -1");
    ap.sign = x < 0 ? -1 : 1;
  }
  if (num(root.find("ref_module"), "ref_module", x)) ap.ref_module = static_cast<int>(x);
  if (num(root.find("window_ns"), "window_ns", x)) {
    if (x <= 0) errors.push_back("'window_ns' must be > 0");
    ap.W = x;
  }
  if (root.find("bin_ns") && num(root.find("bin_ns"), "bin_ns", x)) ap.bin = x;
  const JV* pk = root.find("peak");
  if (pk && (pk->t != JV::T::Str || !parse_peak_value(pk->str, ap.value)))
    errors.push_back("'peak' must be \"mode\", \"centroid\" or \"median\"");
  const JV* bs = root.find("boards");
  if (!bs || bs->t != JV::T::Arr || bs->arr.empty()) {
    errors.push_back("'boards' must be a non-empty array");
    return false;
  }
  std::set<int> seen;
  for (std::size_t i = 0; i < bs->arr.size(); ++i) {
    const JV& b = bs->arr[i];
    const std::string w = "boards[" + std::to_string(i) + "]";
    if (b.t != JV::T::Obj) {
      errors.push_back(w + ": not an object");
      continue;
    }
    AppliedPlan::Board B;
    if (num(b.find("module"), w + ".module", x)) B.module = static_cast<int>(x);
    if (!seen.insert(B.module).second) errors.push_back(w + ": module " + std::to_string(B.module) + " twice");
    if (num(b.find("step_ns"), w + ".step_ns", x)) {
      if (x <= 0) errors.push_back(w + ".step_ns must be > 0");
      B.step = static_cast<int>(x);
    }
    num(b.find("delay_change_ns"), w + ".delay_change_ns", B.delay_change);
    const JV* p = b.find("pulser");
    if (p && p->t == JV::T::Arr && p->arr.size() == 2 && p->arr[0].t == JV::T::Num &&
        p->arr[1].t == JV::T::Num) {
      B.has_pulser = true;
      B.pulser = mc_key(static_cast<uint8_t>(p->arr[0].num), static_cast<uint8_t>(p->arr[1].num));
    } else if (p && p->t != JV::T::Null) {
      errors.push_back(w + ".pulser must be null or [module, channel]");
    }
    const JV* pb = b.find("pulser_dt_ns");
    if (pb && pb->t == JV::T::Num) {
      B.has_before = true;
      B.pulser_before = pb->num;
    }
    ap.boards.push_back(B);
  }
  return errors.empty();
}

struct VerifyRow {
  int module = 0;
  int step = 16;
  bool measured = false;  // before and after pulser peaks both significant
  double before = 0, after = 0, change = 0, expected = 0, diff = 0;
  bool pass = false;
  std::string note;
};

// `before` / `after`: module -> pulser dt peak (vs the ref board's pulser).
// A module missing from `before` falls back to the plan's pulser_dt_ns.
inline std::vector<VerifyRow> verify_boards(const AppliedPlan& ap, const std::map<int, Peak>* before,
                                            const std::map<int, Peak>& after) {
  std::vector<VerifyRow> out;
  double ref_delay = 0;
  for (const auto& b : ap.boards)
    if (b.module == ap.ref_module) ref_delay = b.delay_change;
  for (const auto& b : ap.boards) {
    if (b.module == ap.ref_module || !b.has_pulser) continue;
    VerifyRow v;
    v.module = b.module;
    v.step = b.step;
    v.expected = -ap.sign * (b.delay_change - ref_delay);
    bool hb = false, ha = false;
    if (before) {
      auto it = before->find(b.module);
      if (it != before->end() && it->second.significant) {
        hb = true;
        v.before = peak_value(it->second, ap.value);
      }
    } else if (b.has_before) {
      hb = true;
      v.before = b.pulser_before;
    }
    auto ia = after.find(b.module);
    if (ia != after.end() && ia->second.significant) {
      ha = true;
      v.after = peak_value(ia->second, ap.value);
    }
    v.measured = hb && ha;
    if (!hb) v.note = "no pulser peak BEFORE";
    if (!ha) v.note += std::string(v.note.empty() ? "" : "; ") + "no pulser peak AFTER";
    if (v.measured) {
      v.change = v.after - v.before;
      v.diff = v.change - v.expected;
      v.pass = std::fabs(v.diff) <= 0.5 * v.step;
    }
    out.push_back(v);
  }
  return out;
}

// ===========================================================================
// 7. Anti-Compton map (time_map_ac)
// ===========================================================================
struct AcCell {
  Peak p;
  long long n_det = 0;
  double frac = 0;  // veto fraction = signal / detector hits (excess per detector hit)
  bool sig = false;
};

enum class AcStatus { Assigned, Ambiguous, Silent, NoHits };

struct AcShield {
  uint16_t key = 0;
  long long hits = 0;
  AcStatus status = AcStatus::Silent;
  int group = -1;               // Assigned: the group
  std::vector<int> candidates;  // significant groups, best first
};

struct AcResult {
  std::vector<Group> groups;
  std::vector<uint16_t> shields;
  double W = 0, dominance = 0.5, margin = 20;
  std::vector<long long> group_hits;
  std::vector<std::vector<AcCell>> cell;                   // [group][shield]
  std::map<std::pair<uint16_t, uint16_t>, AcCell> chan;    // (detector ch, shield)
  std::vector<AcShield> sh;
  bool window_ok = false;
  double lo = 0, hi = 0;  // union of the assigned pairs' [q_lo, q_hi]
  double pre = 0, post = 0;
};

inline const char* ac_status_name(AcStatus s) {
  switch (s) {
    case AcStatus::Assigned: return "ASSIGNED";
    case AcStatus::Ambiguous: return "AMBIGUOUS";
    case AcStatus::Silent: return "SILENT";
    case AcStatus::NoHits: return "NO HITS";
  }
  return "?";
}

// Assignment of each shield, from the group-level cells: the significant
// groups ranked by veto fraction; ASSIGNED to the best when every other
// significant group's fraction is < dominance * best, AMBIGUOUS otherwise
// (dominance 0 = any second significant group is ambiguous), SILENT when no
// group is significant. A pure significance rule is not enough on real data:
// gamma cascades make distant shield/detector pairs prompt-coincident too.
inline void assign_shields(AcResult& r) {
  r.sh.clear();
  for (std::size_t s = 0; s < r.shields.size(); ++s) {
    AcShield a;
    a.key = r.shields[s];
    a.hits = 0;
    std::vector<int> c;
    for (std::size_t g = 0; g < r.groups.size(); ++g)
      if (r.cell[g][s].sig) c.push_back(static_cast<int>(g));
    std::sort(c.begin(), c.end(), [&](int x, int y) {
      return r.cell[static_cast<std::size_t>(x)][s].frac > r.cell[static_cast<std::size_t>(y)][s].frac;
    });
    a.candidates = c;
    if (c.empty()) {
      a.status = AcStatus::Silent;
    } else {
      double best = r.cell[static_cast<std::size_t>(c[0])][s].frac;
      std::vector<int> close;
      for (int g : c)
        if (r.cell[static_cast<std::size_t>(g)][s].frac >= r.dominance * best) close.push_back(g);
      if (close.size() == 1) {
        a.status = AcStatus::Assigned;
        a.group = c[0];
      } else {
        a.status = AcStatus::Ambiguous;
        a.candidates = close;
      }
    }
    r.sh.push_back(a);
  }
}

// Suggested window from the ASSIGNED pairs: shield in [t_det - pre, t_det + post]
// covering every pair's central `cover` interval [q_lo, q_hi] plus `margin`.
inline void ac_window(AcResult& r) {
  r.window_ok = false;
  for (std::size_t s = 0; s < r.sh.size(); ++s) {
    if (r.sh[s].status != AcStatus::Assigned) continue;
    const Peak& p = r.cell[static_cast<std::size_t>(r.sh[s].group)][s].p;
    if (!r.window_ok) {
      r.lo = p.q_lo;
      r.hi = p.q_hi;
      r.window_ok = true;
    } else {
      r.lo = std::min(r.lo, p.q_lo);
      r.hi = std::max(r.hi, p.q_hi);
    }
  }
  if (r.window_ok) {
    r.pre = std::ceil(std::max(0.0, r.margin - r.lo));
    r.post = std::ceil(std::max(0.0, r.hi + r.margin));
  }
}

inline AcResult measure_ac(const HitTimes& ht, const std::vector<Group>& groups,
                           const std::vector<uint16_t>& shields, double W, const MeasureOpts& o,
                           double dominance, double margin) {
  AcResult r;
  r.groups = groups;
  r.shields = shields;
  r.W = W;
  r.dominance = dominance;
  r.margin = margin;
  // One time-ordered stream of all shield hits: one sweep per detector channel.
  std::vector<std::pair<double, int>> stream;
  for (std::size_t s = 0; s < shields.size(); ++s)
    for (double t : ht.of(shields[s])) stream.emplace_back(t, static_cast<int>(s));
  std::sort(stream.begin(), stream.end());
  DtHist proto;
  proto.init(W, o.bin);
  r.cell.assign(groups.size(), std::vector<AcCell>(shields.size()));
  r.group_hits.assign(groups.size(), 0);
  for (std::size_t g = 0; g < groups.size(); ++g) {
    std::vector<DtHist> gsum(shields.size(), proto);
    for (uint16_t d : groups[g].chans) {
      const std::vector<double>& td = ht.of(d);
      r.group_hits[g] += static_cast<long long>(td.size());
      std::vector<DtHist> hs(shields.size(), proto);
      fill_stream(stream, td, hs);
      for (std::size_t s = 0; s < shields.size(); ++s) {
        AcCell c;
        c.n_det = static_cast<long long>(td.size());
        c.p = find_peak(hs[s], o.peak);
        c.sig = c.p.significant;
        c.frac = c.n_det > 0 ? c.p.signal / static_cast<double>(c.n_det) : 0.0;
        r.chan[std::make_pair(d, shields[s])] = c;
        gsum[s].add(hs[s]);
      }
    }
    for (std::size_t s = 0; s < shields.size(); ++s) {
      AcCell& c = r.cell[g][s];
      c.n_det = r.group_hits[g];
      c.p = find_peak(gsum[s], o.peak);
      c.sig = c.p.significant;
      c.frac = c.n_det > 0 ? c.p.signal / static_cast<double>(c.n_det) : 0.0;
    }
  }
  assign_shields(r);
  for (std::size_t s = 0; s < shields.size(); ++s) {
    r.sh[s].hits = ht.hits(shields[s]);
    if (r.sh[s].hits == 0) {
      r.sh[s].status = AcStatus::NoHits;
      r.sh[s].group = -1;
      r.sh[s].candidates.clear();
    }
  }
  ac_window(r);
  return r;
}

// "[[8, 0], [8, 1]]"
inline std::string mc_json_list(const std::vector<uint16_t>& ks) {
  std::string s = "[";
  for (std::size_t i = 0; i < ks.size(); ++i)
    s += (i ? ", [" : "[") + std::to_string(ks[i] >> 8) + ", " + std::to_string(ks[i] & 0xFF) + "]";
  return s + "]";
}

// Suggested event_def v1.1 block: {"_comment", "tags", "anti_compton"} — merge
// "tags" and "anti_compton" into the event definition. One pair per group with
// at least one ASSIGNED shield; ambiguous / silent shields and unshielded
// groups are named in _comment, never dropped silently. Returns "" when no
// shield was assigned (an empty pairs list is not a valid anti_compton section).
inline std::string ac_block_json(const AcResult& r, const std::vector<std::string>& comments) {
  if (!r.window_ok) return "";
  std::vector<std::vector<uint16_t>> mine(r.groups.size());
  for (const AcShield& a : r.sh)
    if (a.status == AcStatus::Assigned) mine[static_cast<std::size_t>(a.group)].push_back(a.key);
  std::vector<std::string> c = comments;
  c.push_back("dt = t(shield) - t(detector); window relative to the detector hit: shield in "
              "[t - pre, t + post]. Merge 'tags' and 'anti_compton' into the event definition (v1.1).");
  char buf[320];
  std::snprintf(buf, sizeof buf,
                "window = union of the assigned pairs' central intervals [%+.1f, %+.1f] ns + margin "
                "%.0f ns -> pre %.0f, post %.0f",
                r.lo, r.hi, r.margin, r.pre, r.post);
  c.push_back(buf);
  for (std::size_t s = 0; s < r.sh.size(); ++s) {
    const AcShield& a = r.sh[s];
    if (a.status == AcStatus::Assigned) {
      const AcCell& x = r.cell[static_cast<std::size_t>(a.group)][s];
      std::snprintf(buf, sizeof buf,
                    "%s -> %s: mode %+.1f, centroid %+.1f, FWHM %.1f, central [%+.1f, %+.1f] ns, "
                    "veto %.2f %%, z %.0f",
                    mc_name(a.key).c_str(), r.groups[static_cast<std::size_t>(a.group)].name.c_str(),
                    x.p.mode, x.p.centroid, x.p.fwhm, x.p.q_lo, x.p.q_hi, 100 * x.frac, x.p.z);
      c.push_back(buf);
    }
  }
  for (std::size_t s = 0; s < r.sh.size(); ++s) {
    const AcShield& a = r.sh[s];
    if (a.status == AcStatus::Ambiguous) {
      std::string t = "AMBIGUOUS, not paired: " + mc_name(a.key) + " ->";
      for (int g : a.candidates)
        t += " " + r.groups[static_cast<std::size_t>(g)].name +
             fmt(" (%.2f %%)", 100 * r.cell[static_cast<std::size_t>(g)][s].frac);
      c.push_back(t);
    } else if (a.status == AcStatus::Silent) {
      c.push_back("SILENT, not paired: " + mc_name(a.key) + " (no significant excess with any group)");
    } else if (a.status == AcStatus::NoHits) {
      c.push_back("NO HITS, not paired: " + mc_name(a.key));
    }
  }
  std::string unshielded;
  for (std::size_t g = 0; g < r.groups.size(); ++g)
    if (mine[g].empty()) unshielded += (unshielded.empty() ? "" : ", ") + r.groups[g].name;
  if (!unshielded.empty()) c.push_back("groups with NO assigned shield (no pair): " + unshielded);
  std::string js = "{\n  \"_comment\": [\n";
  for (std::size_t i = 0; i < c.size(); ++i)
    js += "    \"" + json_escape(c[i]) + "\"" + (i + 1 < c.size() ? ",\n" : "\n");
  js += "  ],\n  \"tags\": {\n";
  bool first = true;
  for (std::size_t g = 0; g < r.groups.size(); ++g) {
    if (mine[g].empty()) continue;
    const std::string n = json_escape(r.groups[g].name);
    js += std::string(first ? "" : ",\n") + "    \"" + n + "_det\": " + mc_json_list(r.groups[g].chans) +
          ",\n    \"" + n + "_shield\": " + mc_json_list(mine[g]);
    first = false;
  }
  std::snprintf(buf, sizeof buf,
                "\n  },\n  \"anti_compton\": {\n    \"window_ns\": {\"pre\": %.0f, \"post\": %.0f},\n"
                "    \"pairs\": [\n",
                r.pre, r.post);
  js += buf;
  first = true;
  for (std::size_t g = 0; g < r.groups.size(); ++g) {
    if (mine[g].empty()) continue;
    const std::string n = json_escape(r.groups[g].name);
    js += std::string(first ? "" : ",\n") + "      {\"name\": \"" + n + "\", \"detector\": [\"" + n +
          "_det\"], \"shield\": [\"" + n + "_shield\"]}";
    first = false;
  }
  js += "\n    ]\n  }\n}\n";
  return js;
}

}  // namespace timemap
}  // namespace rootsink

#endif  // ROOTSINK_TIME_MAP_HPP

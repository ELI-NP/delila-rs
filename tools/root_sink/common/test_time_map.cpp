// test_time_map.cpp — ROOT-free unit tests for time_map.hpp (TODO 72 board
// offsets / TODO 71 Phase D anti-Compton map).
//
// Synthetic hit streams with KNOWN board offsets, cable delays, a pulser on the
// last channel of every board and anti-Compton shields; the tests check that
// the measurement finds them, that the board plan rounds to the hardware step
// with the documented sign, that verification PASSes when the planned change
// is applied (and FAILs when it is not, or with the wrong sign), and that the
// suggested anti_compton block parses as an event_def v1.1 section.
//
//   g++ -std=c++17 -O2 -Wall -Wextra test_time_map.cpp -o /tmp/ttm && /tmp/ttm
//
// Same tiny CHECK harness as test_sink_core.cpp (deliberate copy — the test
// binaries stay independent).
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <limits>
#include <map>
#include <random>
#include <set>
#include <string>
#include <vector>

#include "event_def.hpp"
#include "time_map.hpp"
#include "time_offsets.hpp"

using namespace rootsink;
using namespace rootsink::timemap;

// ---------------------------------------------------------------------------
// Tiny assert harness
// ---------------------------------------------------------------------------
static int g_pass = 0;
static int g_fail = 0;

#define CHECK(cond)                                                       \
  do {                                                                    \
    if (cond) {                                                           \
      ++g_pass;                                                           \
    } else {                                                              \
      ++g_fail;                                                           \
      std::printf("FAIL %s:%d  CHECK(%s)\n", __FILE__, __LINE__, #cond);  \
    }                                                                     \
  } while (0)

static bool near(double a, double b, double eps) {
  double d = a - b;
  return (d < 0 ? -d : d) <= eps;
}

static uint16_t K(int m, int c) { return mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(c)); }

// Poisson process of `rate` Hz over [0, T_ns).
static std::vector<double> poisson(std::mt19937_64& rng, double rate_hz, double T_ns) {
  std::vector<double> v;
  if (rate_hz <= 0) return v;
  std::exponential_distribution<double> ex(rate_hz * 1e-9);
  for (double t = ex(rng); t < T_ns; t += ex(rng)) v.push_back(t);
  return v;
}

// ---------------------------------------------------------------------------
// Spec parsing
// ---------------------------------------------------------------------------
static void test_parse_specs() {
  uint16_t k = 0;
  CHECK(parse_mc("9:0", k) && k == K(9, 0));
  CHECK(parse_mc(" 10 : 15 ", k) && k == K(10, 15));
  CHECK(!parse_mc("9", k));
  CHECK(!parse_mc("9:256", k));
  CHECK(!parse_mc("a:1", k));
  CHECK(!parse_mc("1:2:3", k));
  CHECK(mc_str(K(8, 24)) == "8:24");

  std::set<uint16_t> present = {K(8, 0), K(8, 3), K(8, 24), K(9, 1)};
  std::vector<uint16_t> v;
  std::string err;
  CHECK(parse_mc_list("8:*", present, v, err) && v.size() == 3 && v[2] == K(8, 24));
  v.clear();
  CHECK(parse_mc_list("8:0-3, 9:1", present, v, err) && v.size() == 5 && v[1] == K(8, 1));
  v.clear();
  CHECK(!parse_mc_list("8:0-3,8:2", present, v, err) && err.find("twice") != std::string::npos);
  v.clear();
  CHECK(!parse_mc_list("7:*", present, v, err) && err.find("no hits") != std::string::npos);
  v.clear();
  CHECK(!parse_mc_list("8:5-2", present, v, err));
  v.clear();
  CHECK(!parse_mc_list("", present, v, err));

  // Groups: named, per-channel, and from event_def tags.
  std::vector<Group> g;
  CHECK(parse_groups("A1=9:0;C2=10:0,10:1;", present, "", g, err) && g.size() == 2 &&
        g[0].name == "A1" && g[1].chans.size() == 2);
  CHECK(parse_groups("9:0,10:0-1", present, "", g, err) && g.size() == 3 && g[2].name == "m10c1");
  CHECK(!parse_groups("A=9:0;B=9:0", present, "", g, err) && err.find("groups 'A' and 'B'") != std::string::npos);
  CHECK(!parse_groups("A=9:0;A=9:1", present, "", g, err) && err.find("twice") != std::string::npos);
  CHECK(!parse_groups("A=9:0;9:1", present, "", g, err));
  const std::string def = "{\"version\":\"1.1\",\"tags\":{\"C1\":[[0,0,3]],\"C2\":[[1,0],[1,2]],"
                          "\"BGO1\":[[8,0,2]]}}";
  CHECK(parse_groups("@def.json:C1,C2", present, def, g, err) && g.size() == 2 &&
        g[0].chans.size() == 4 && g[1].chans[1] == K(1, 2));
  CHECK(!parse_groups("@def.json:C1,C9", present, def, g, err) && err.find("unknown tag 'C9'") != std::string::npos);
  CHECK(!parse_groups("@def.json", present, def, g, err));

  // Steps.
  StepSpec st;
  CHECK(parse_steps("", st, err) && st.of(3) == 16);
  CHECK(parse_steps("8", st, err) && st.of(3) == 8);
  CHECK(parse_steps("16,10=8,11=8", st, err) && st.of(10) == 8 && st.of(11) == 8 && st.of(0) == 16);
  CHECK(!parse_steps("16,8", st, err));
  CHECK(!parse_steps("10=0", st, err));
  CHECK(!parse_steps("10=8,10=16", st, err));

  // Pulsers: default = last channel of each module present.
  std::map<int, uint16_t> pu;
  CHECK(parse_pulsers("", present, pu, err) && pu.size() == 2 && pu[8] == K(8, 24) && pu[9] == K(9, 1));
  CHECK(parse_pulsers("8:3", present, pu, err) && pu.size() == 1 && pu[8] == K(8, 3));
  CHECK(!parse_pulsers("8:3,8:24", present, pu, err));

  // Options: unknown keys are errors; numbers range-checked.
  Options o;
  CHECK(parse_options("bin=1  nsigma=4;calib=/a b", {"bin", "nsigma", "calib"}, o, err) == false);
  CHECK(parse_options("bin=1  nsigma=4;calib=/a", {"bin", "nsigma", "calib"}, o, err) &&
        o.str("calib", "") == "/a");
  double x = 0;
  CHECK(opt_num(o, "bin", 2, 0.01, 100, x, err) && x == 1);
  CHECK(opt_num(o, "search", 0.5, 0, 1, x, err) && x == 0.5);
  CHECK(!parse_options("bnn=1", {"bin"}, o, err) && err.find("unknown option 'bnn'") != std::string::npos);
  CHECK(parse_options("bin=abc", {"bin"}, o, err) && !opt_num(o, "bin", 2, 0.01, 100, x, err));
  CHECK(!parse_options("bin=1 bin=2", {"bin"}, o, err));
}

// ---------------------------------------------------------------------------
// Histogram filling against brute force
// ---------------------------------------------------------------------------
static void test_fill_vs_brute() {
  std::mt19937_64 rng(7);
  std::vector<double> x = poisson(rng, 2e6, 2e6), r = poisson(rng, 1.5e6, 2e6);
  const double W = 200;
  DtHist a, b, c, d;
  a.init(W, 1);
  b.init(W, 1);
  fill_all_pairs(x, r, a);
  for (double t : x)
    for (double u : r)
      if (std::fabs(t - u) <= W) b.fill(t - u);
  CHECK(a.entries > 1000 && a.entries == b.entries && a.n == b.n);

  c.init(W, 1);
  d.init(W, 1);
  fill_nearest(x, r, c);
  for (double t : x) {
    double best = std::numeric_limits<double>::infinity(), bd = 0;
    for (double u : r) {
      double dd = std::fabs(t - u);
      if (dd < best || (dd == best && u > t)) {  // ties -> the later ref hit
        best = dd;
        bd = t - u;
      }
    }
    if (best <= W) d.fill(bd);
  }
  CHECK(c.entries > 100 && c.entries == d.entries && c.n == d.n);
  CHECK(c.entries < a.entries);

  // fill_stream == fill_all_pairs per channel.
  std::vector<double> y = poisson(rng, 1e6, 2e6);
  std::vector<std::pair<double, int>> st;
  for (double t : x) st.emplace_back(t, 0);
  for (double t : y) st.emplace_back(t, 1);
  std::sort(st.begin(), st.end());
  DtHist proto;
  proto.init(W, 1);
  std::vector<DtHist> hs(2, proto);
  fill_stream(st, r, hs);
  DtHist e;
  e.init(W, 1);
  fill_all_pairs(y, r, e);
  CHECK(hs[0].n == a.n && hs[1].n == e.n);

  // Exact range: nbins * bin == 2W, dt == +W lands in the last bin, beyond is dropped.
  DtHist h;
  h.init(100, 3);  // 66.67 -> 67 bins
  CHECK(h.nbins() == 67 && near(h.nbins() * h.bin, 200, 1e-9));
  h.fill(100);
  h.fill(-100);
  h.fill(100.001);
  CHECK(h.entries == 2 && h.n.back() == 1 && h.n.front() == 1);
}

// ---------------------------------------------------------------------------
// Peak finding
// ---------------------------------------------------------------------------
// ref Poisson at r_ref; x = a partner of a fraction f of ref hits at
// ref + shape() plus independent Poisson x hits at r_x.
template <class Shape>
static void make_corr(std::mt19937_64& rng, double r_ref, double f, double r_x, double T,
                      Shape shape, std::vector<double>& x, std::vector<double>& ref,
                      long long& n_partner) {
  ref = poisson(rng, r_ref, T);
  x = poisson(rng, r_x, T);
  std::uniform_real_distribution<double> u(0, 1);
  n_partner = 0;
  for (double t : ref)
    if (u(rng) < f) {
      x.push_back(t + shape(rng));
      ++n_partner;
    }
  std::sort(x.begin(), x.end());
}

static void test_peak_gaussian() {
  std::mt19937_64 rng(11);
  std::vector<double> x, r;
  long long np = 0;
  std::normal_distribution<double> g(37.3, 3.0);
  make_corr(rng, 20e3, 0.2, 30e3, 2e9, [&](std::mt19937_64& R) { return g(R); }, x, r, np);
  MeasureOpts o;
  Peak p = measure_pair(x, r, 1000, o);
  CHECK(p.significant);
  CHECK(near(p.mode, 37.3, 0.5));
  CHECK(near(p.centroid, 37.3, 0.5));
  CHECK(near(p.median, 37.3, 0.5));
  CHECK(near(p.fwhm, 2.3548 * 3.0, 1.5));
  CHECK(near(p.signal, static_cast<double>(np), 5 * p.signal_err + 0.02 * np));
  CHECK(p.z > 50);
  CHECK(std::fabs(p.sb_asym_z) < 5);
  CHECK(near(p.q_lo, 37.3 - 1.645 * 3, 1.5) && near(p.q_hi, 37.3 + 1.645 * 3, 1.5));
  CHECK(p.region_lo < p.lo_half && p.region_hi > p.hi_half);
  // Nearest pairing finds the same peak.
  o.nearest = true;
  Peak q = measure_pair(x, r, 1000, o);
  CHECK(q.significant && near(q.mode, 37.3, 0.5));
}

static void test_peak_flat_not_significant() {
  int false_pos = 0;
  for (unsigned seed = 1; seed <= 6; ++seed) {
    std::mt19937_64 rng(seed);
    std::vector<double> x = poisson(rng, 30e3, 2e9), r = poisson(rng, 20e3, 2e9);
    Peak p = measure_pair(x, r, 1000, MeasureOpts{});
    if (p.significant) ++false_pos;
    CHECK(std::fabs(p.sb_asym_z) < 5);
  }
  CHECK(false_pos == 0);
  // Empty input.
  Peak e = measure_pair({}, {}, 1000, MeasureOpts{});
  CHECK(!e.candidate && !e.significant);
}

static void test_peak_sharp_and_low_stats() {
  // Pulser-like: sigma 0.2 ns, 2 ns bins.
  std::mt19937_64 rng(3);
  std::vector<double> x, r;
  long long np = 0;
  std::normal_distribution<double> g(-52.6, 0.2);
  make_corr(rng, 1e3, 1.0, 0.0, 1e9, [&](std::mt19937_64& R) { return g(R); }, x, r, np);
  Peak p = measure_pair(x, r, 500, MeasureOpts{});
  CHECK(p.significant && near(p.mode, -52.6, 1.0) && near(p.centroid, -52.6, 1.0));
  CHECK(p.fwhm <= 4.0 && p.bg_per_bin == 0);
  CHECK(near(p.signal, static_cast<double>(np), 0.5));
  // 30 coincidences on zero background: significant (z = sqrt(30) > 5).
  std::vector<double> x2(x.begin(), x.begin() + 30), r2(r.begin(), r.begin() + 30);
  Peak q = measure_pair(x2, r2, 500, MeasureOpts{});
  CHECK(q.significant && near(q.mode, -52.6, 1.0));
  // 10 only: not significant.
  std::vector<double> x3(x.begin(), x.begin() + 10), r3(r.begin(), r.begin() + 10);
  CHECK(!measure_pair(x3, r3, 500, MeasureOpts{}).significant);
}

static void test_peak_broad_asymmetric() {
  // HPGe-like: fast edge at -20 ns, exponential walk tail of 60 ns.
  std::mt19937_64 rng(5);
  std::vector<double> x, r;
  long long np = 0;
  std::normal_distribution<double> g(0, 4.0);
  std::exponential_distribution<double> ex(1.0 / 60);
  make_corr(rng, 10e3, 0.3, 20e3, 2e9, [&](std::mt19937_64& R) { return -20 + g(R) + ex(R); },
            x, r, np);
  MeasureOpts o;
  Peak p = measure_pair(x, r, 2000, o);
  CHECK(p.significant);
  CHECK(p.mode > -25 && p.mode < 10);  // near the onset
  CHECK(p.centroid > p.mode + 15);     // pulled by the tail
  CHECK(p.q_hi > 80 && p.q_lo < -15);  // central 90 % spans the tail
  CHECK(p.fwhm > 30 && p.fwhm < 80);
  CHECK(p.scale >= 8);                 // detected at a broad boxcar
  CHECK(peak_value(p, PeakValue::Centroid) == p.centroid);
  CHECK(peak_value(p, PeakValue::Median) == p.median && peak_value(p, PeakValue::Mode) == p.mode);
  CHECK(p.median > p.mode + 5 && p.median < p.centroid);  // between the two on a walk tail
}

// Background with a slope (more accidentals on one side): noted as not flat,
// the strong peak is still used. Two comparable peaks: ambiguous, not used.
static void test_peak_not_flat_and_multiple() {
  DtHist h;
  h.init(1000, 2);
  std::mt19937_64 rng(13);
  for (int i = 0; i < h.nbins(); ++i) {
    std::poisson_distribution<int> pd(20.0 + 20.0 * i / h.nbins());  // 20 -> 40 per bin
    h.n[static_cast<std::size_t>(i)] = pd(rng);
    h.entries += static_cast<long long>(h.n[static_cast<std::size_t>(i)]);
  }
  for (int i = 495; i < 505; ++i) h.n[static_cast<std::size_t>(i)] += 2000;  // a strong peak at ~0
  Peak p = find_peak(h, PeakOpts{});
  CHECK(p.candidate && !p.flat && p.significant && !p.multiple && near(p.mode, 0, 10));  // 20 ns flat top
  CHECK(p.sb_asym_rel < -0.3 && p.sb_asym_z < -5);
  CHECK(p.note.find("side bands differ") != std::string::npos);
  // The same peak on a flat background is used.
  DtHist f;
  f.init(1000, 2);
  for (int i = 0; i < f.nbins(); ++i) {
    std::poisson_distribution<int> pd(30.0);
    f.n[static_cast<std::size_t>(i)] = pd(rng) + ((i >= 495 && i < 505) ? 200 : 0);
  }
  f.entries = 1;
  Peak q = find_peak(f, PeakOpts{});
  CHECK(q.flat && q.significant && near(q.mode, 0, 6) && q.note.empty());
  // A second peak with 80 % of the first at +300 ns: MULTIPLE, not used.
  DtHist g = f;
  for (int i = 645; i < 655; ++i) g.n[static_cast<std::size_t>(i)] += 160;
  Peak r = find_peak(g, PeakOpts{});
  CHECK(r.multiple && !r.significant && near(r.second_at, 300, 12) && near(r.second_ratio, 0.8, 0.2));
  CHECK(r.note.find("MULTIPLE") != std::string::npos);
  // ... with 25 %: the main peak is used.
  DtHist k = f;
  for (int i = 645; i < 655; ++i) k.n[static_cast<std::size_t>(i)] += 50;
  Peak s2 = find_peak(k, PeakOpts{});
  CHECK(!s2.multiple && s2.significant && near(s2.mode, 0, 6) && s2.second_z > 5);
  // multi > 1 disables the check.
  PeakOpts po;
  po.multi = 2;
  CHECK(find_peak(g, po).significant);
}

static void test_peak_at_search_edge() {
  std::mt19937_64 rng(9);
  std::vector<double> x, r;
  long long np = 0;
  std::normal_distribution<double> g(500, 20);  // W = 1000 -> search |dt| < 500
  make_corr(rng, 20e3, 0.2, 30e3, 1e9, [&](std::mt19937_64& R) { return g(R); }, x, r, np);
  Peak p = measure_pair(x, r, 1000, MeasureOpts{});
  CHECK(p.candidate && p.at_edge && !p.significant);
  CHECK(p.note.find("widen") != std::string::npos);
  // Widened: fine.
  Peak q = measure_pair(x, r, 2500, MeasureOpts{});
  CHECK(q.significant && near(q.mode, 500, 3));
}

// Ten channels on one board, each too weak alone; the board's summed
// histogram still gives the board offset (flagged from_sum).
static void test_board_sum_fallback() {
  std::mt19937_64 rng(21);
  const double T = 2e9;
  HitTimes ht;
  std::vector<double> ref = poisson(rng, 20e3, T);
  for (double t : ref) ht.add(K(0, 0), t);
  std::normal_distribution<double> g(-222, 3);
  std::uniform_real_distribution<double> u(0, 1);
  for (int c = 0; c < 10; ++c) {
    for (double t : poisson(rng, 20e3, T)) ht.add(K(7, c), t);
    for (double t : ref)
      if (u(rng) < 0.0003) ht.add(K(7, c), t + g(rng));
  }
  ht.sort_all();
  std::map<int, Peak> sums;
  std::vector<ChannelPeak> cps = measure_vs_ref(ht, K(0, 0), 1000, MeasureOpts{}, {}, &sums);
  int nsig = 0;
  for (const ChannelPeak& cp : cps) nsig += cp.p.significant ? 1 : 0;
  CHECK(cps.size() == 10 && nsig <= 1);
  CHECK(sums.count(7) && sums[7].significant && near(sums[7].mode, -222, 2));
  std::map<int, BoardDet> bd = board_detector_offsets(cps, K(0, 0), PeakValue::Mode, &sums);
  CHECK(bd[0].ok && bd[0].median == 0 && !bd[0].from_sum);
  if (nsig == 0) CHECK(bd[7].ok && bd[7].from_sum && near(bd[7].median, -222, 2));
  // Without the sums: no board offset.
  std::map<int, BoardDet> bn = board_detector_offsets(cps, K(0, 0), PeakValue::Mode, nullptr);
  CHECK(nsig > 0 || !bn[7].ok);
}

static void test_steps_and_median() {
  CHECK(steps_of(7.9, 16) == 0);
  CHECK(steps_of(8.0, 16) == 1);  // half away from zero
  CHECK(steps_of(-8.0, 16) == -1);
  CHECK(steps_of(23.9, 16) == 1);
  CHECK(steps_of(24.1, 16) == 2);
  CHECK(steps_of(-52, 16) == -3);
  CHECK(steps_of(13, 8) == 2);
  CHECK(median({3, 1, 2}) == 2);
  CHECK(median({4, 1, 2, 3}) == 2.5);
  CHECK(median({}) == 0);
}

// ---------------------------------------------------------------------------
// Offsets JSON round trip through time_offsets.hpp
// ---------------------------------------------------------------------------
static void test_offsets_json_roundtrip() {
  std::vector<ChannelPeak> cps(3);
  cps[0].key = K(0, 1);
  cps[0].p.significant = true;
  cps[0].p.mode = 12.5;
  cps[0].p.centroid = 13.0;
  cps[1].key = K(2, 3);
  cps[1].p.significant = true;
  cps[1].p.mode = -40.25;
  cps[1].p.centroid = -41;
  cps[2].key = K(2, 4);  // no peak
  cps[2].hits = 17;
  std::string js = offsets_tree_json(cps, K(1, 0), PeakValue::Mode, nullptr, {"test"});
  OffsetParseResult pr = parse_time_offsets(js);
  CHECK(pr.errors.empty());
  CHECK(pr.table.roots == 1 && pr.table.abs_ns.size() == 3);
  CHECK(near(pr.table.abs_ns[K(1, 0)], 0, 1e-9));
  CHECK(near(pr.table.abs_ns[K(0, 1)], 12.5, 1e-6));
  CHECK(near(pr.table.abs_ns[K(2, 3)], -40.25, 1e-6));
  CHECK(pr.table.abs_ns.count(K(2, 4)) == 0);
  CHECK(js.find("m2c4 (17 hits") != std::string::npos);  // listed, not silently dropped
  // Centroid choice.
  OffsetParseResult pc = parse_time_offsets(offsets_tree_json(cps, K(1, 0), PeakValue::Centroid, nullptr, {}));
  CHECK(pc.errors.empty() && near(pc.table.abs_ns[K(0, 1)], 13.0, 1e-6));
  // With an input table (residual measurement): totals relative to the ref;
  // a channel without a peak keeps its input offset.
  OffsetParseResult in = parse_time_offsets(
      "{\"version\":\"1.0\",\"entries\":[{\"module\":1,\"channel\":0,\"ref\":null,\"offset_ns\":5},"
      "{\"module\":0,\"channel\":1,\"ref\":[1,0],\"offset_ns\":100},"
      "{\"module\":2,\"channel\":4,\"ref\":[1,0],\"offset_ns\":-7}]}");
  CHECK(in.errors.empty());
  std::string js2 = offsets_tree_json(cps, K(1, 0), PeakValue::Mode, &in.table, {});
  OffsetParseResult p2 = parse_time_offsets(js2);
  CHECK(p2.errors.empty());
  CHECK(near(p2.table.abs_ns[K(0, 1)], 112.5, 1e-6));  // 100 (relative) + 12.5 residual
  CHECK(near(p2.table.abs_ns[K(2, 3)], -40.25, 1e-6)); // not in the input
  CHECK(near(p2.table.abs_ns[K(2, 4)], -7, 1e-6));     // kept
  CHECK(js2.find("input offset KEPT: m2c4") != std::string::npos);
}

// ---------------------------------------------------------------------------
// Boards: synthetic multi-board run with a pulser on the last channel
// ---------------------------------------------------------------------------
struct SynthBoards {
  std::map<int, double> O;     // true board offset (timestamps = true time + O)
  std::map<int, double> e;     // pulser electronics difference per board
  std::map<int, double> shift; // extra timestamp shift applied (the "hardware change")
};

// Modules 0..4, detector channels 0..6, pulser on channel 15. Ref = 1:0.
// Cable delays: ch0..6 = -3..+3 on every board except the ref board, whose
// ref channel 0 is at 0 and ch1..6 = {-3,-2,-1,1,2,3} -> every board's median
// detector delay is exactly 0, so det offset = O_m - O_1.
static HitTimes synth_boards(const SynthBoards& sb, unsigned seed) {
  std::mt19937_64 rng(seed);  // the same seed -> the same physics (before/after)
  const double T = 2e9;
  HitTimes ht;
  std::normal_distribution<double> jit(0, 2.0), pj(0, 0.2);
  std::uniform_real_distribution<double> u(0, 1);
  auto cable = [&](int m, int c) {
    static const double other[7] = {-3, -2, -1, 0, 1, 2, 3};
    static const double refb[7] = {0, -3, -2, -1, 1, 2, 3};
    return m == 1 ? refb[c] : other[c];
  };
  auto off = [&](int m) {
    auto it = sb.shift.find(m);
    return sb.O.at(m) + (it == sb.shift.end() ? 0.0 : it->second);
  };
  for (double t : poisson(rng, 20e3, T))
    for (int m = 0; m <= 4; ++m)
      for (int c = 0; c <= 6; ++c)
        if (u(rng) < 0.3) ht.add(K(m, c), t + off(m) + cable(m, c) + jit(rng));
  for (int m = 0; m <= 4; ++m)
    for (int c = 0; c <= 6; ++c)
      for (double t : poisson(rng, 2e3, T)) ht.add(K(m, c), t + off(m));
  for (double t = 1000; t < T; t += 1e6)  // 1 kHz pulser
    for (int m = 0; m <= 4; ++m) ht.add(K(m, 15), t + off(m) + sb.e.at(m) + pj(rng));
  ht.sort_all();
  return ht;
}

static std::map<int, Peak> pulser_peaks(const HitTimes& ht, const AppliedPlan& ap) {
  std::map<int, Peak> out;
  uint16_t refp = 0;
  for (const auto& b : ap.boards)
    if (b.module == ap.ref_module) refp = b.pulser;
  MeasureOpts o;
  o.bin = ap.bin;
  for (const auto& b : ap.boards)
    if (b.has_pulser && b.module != ap.ref_module)
      out[b.module] = measure_pulser_dt(ht, b.pulser, refp, ap.W, o);
  return out;
}

static const BoardRow* row_of(const BoardsPlan& bp, int m) {
  for (const BoardRow& r : bp.rows)
    if (r.module == m) return &r;
  return nullptr;
}

static void test_boards_plan_and_verify() {
  SynthBoards sb;
  sb.O = {{0, 37.0}, {1, 0.0}, {2, -52.0}, {3, 118.0}, {4, 13.0}};
  sb.e = {{0, 0.4}, {1, 0.0}, {2, -0.6}, {3, 1.0}, {4, 0.2}};
  HitTimes before = synth_boards(sb, 42);
  std::map<int, uint16_t> pul;
  std::string err;
  CHECK(parse_pulsers("", before.keys(), pul, err) && pul.size() == 5 && pul[3] == K(3, 15));
  StepSpec st;
  CHECK(parse_steps("16,4=8", st, err));
  MeasureOpts o;
  BoardsPlan bp = plan_boards(before, K(1, 0), 500, pul, st, +1, o);
  CHECK(bp.rows.size() == 5 && bp.warnings.empty());
  // Pulser channels are not detectors.
  bool pulser_in_det = false;
  for (const ChannelPeak& cp : bp.chans)
    if ((cp.key & 0xFF) == 15) pulser_in_det = true;
  CHECK(!pulser_in_det && bp.chans.size() == 5 * 7 - 1);
  const double want_det[5] = {37, 0, -52, 118, 13};
  const double want_delay[5] = {32, 0, -48, 112, 16};
  const double want_pul[5] = {37.4, 0, -52.6, 119.0, 13.2};
  for (int m = 0; m <= 4; ++m) {
    const BoardRow* r = row_of(bp, m);
    CHECK(r != nullptr);
    if (!r) continue;
    CHECK(r->det.ok && r->det.n_sig == 7);
    CHECK(near(r->det_rel, want_det[m], 0.6));
    CHECK(r->delay_change == want_delay[m]);
    CHECK(r->ts_shift == -want_delay[m]);
    CHECK(near(r->residual, want_det[m] - want_delay[m], 0.6));
    CHECK(std::fabs(r->residual) <= 0.5 * r->step + 0.6);
    CHECK(r->pulser_ok && near(r->pulser_dt, want_pul[m], 0.5));
    CHECK(near(r->expected_pulser_after, want_pul[m] - want_delay[m], 0.5));
    CHECK(r->step == (m == 4 ? 8 : 16));
    // Symmetric peaks: mode, centroid and median give the same plan.
    CHECK(r->alt_ok[0] && r->alt_ok[1] && r->alt_ok[2] && !r->definition_matters());
    CHECK(near(r->alt_det[2], want_det[m], 0.6));
  }
  // Non-negative variant: common constant (+48), differences unchanged.
  CHECK(row_of(bp, 2)->delay_nonneg == 0 && row_of(bp, 1)->delay_nonneg == 48 &&
        row_of(bp, 3)->delay_nonneg == 160);

  // JSON round trip -> AppliedPlan.
  std::string js = boards_json(bp, {"synthetic"});
  AppliedPlan ap;
  std::vector<std::string> errs;
  CHECK(parse_applied(js, ap, errs));
  CHECK(ap.sign == 1 && ap.ref_module == 1 && ap.W == 500 && ap.boards.size() == 5);
  CHECK(ap.boards[3].delay_change == 112 && ap.boards[3].has_pulser && ap.boards[3].pulser == K(3, 15));
  CHECK(ap.boards[4].step == 8 && ap.boards[2].has_before && near(ap.boards[2].pulser_before, -52.6, 0.5));
  CHECK(js.find("\"detector_offset_by_peak_ns\": {\"mode\": ") != std::string::npos);

  // The hardware applies the plan (sign +1: delay +D -> timestamps -D).
  SynthBoards after_sb = sb;
  for (const BoardRow& r : bp.rows) after_sb.shift[r.module] = r.ts_shift;
  HitTimes after = synth_boards(after_sb, 42);
  std::map<int, Peak> pb = pulser_peaks(before, ap), pa = pulser_peaks(after, ap);
  std::vector<VerifyRow> v = verify_boards(ap, &pb, pa);
  CHECK(v.size() == 4);
  int pass = 0;
  for (const VerifyRow& x : v) {
    if (x.pass) ++pass;
    CHECK(x.measured && near(x.change, x.expected, 0.5));
  }
  CHECK(pass == 4);
  // The plan's own before values (no before file): PASS too.
  std::vector<VerifyRow> v2 = verify_boards(ap, nullptr, pa);
  int pass2 = 0;
  for (const VerifyRow& x : v2) pass2 += x.pass ? 1 : 0;
  CHECK(pass2 == 4);
  // After the change the detector offsets are within step/2.
  BoardsPlan bp2 = plan_boards(after, K(1, 0), 500, pul, st, +1, o);
  for (const BoardRow& r : bp2.rows) CHECK(std::fabs(r.det_rel) <= 0.5 * r.step + 0.6 && r.delay_change == 0);

  // Board 3 NOT applied -> FAIL there only.
  SynthBoards miss = after_sb;
  miss.shift[3] = 0;
  std::map<int, Peak> pm = pulser_peaks(synth_boards(miss, 42), ap);
  std::vector<VerifyRow> v3 = verify_boards(ap, &pb, pm);
  for (const VerifyRow& x : v3) CHECK(x.pass == (x.module != 3));
  // Wrong sign applied -> FAIL on every board with a non-zero change.
  SynthBoards wrong = sb;
  for (const BoardRow& r : bp.rows) wrong.shift[r.module] = -r.ts_shift;
  std::map<int, Peak> pw = pulser_peaks(synth_boards(wrong, 42), ap);
  for (const VerifyRow& x : verify_boards(ap, &pb, pw)) CHECK(!x.pass);
  // The non-negative variant applied (edit the JSON): same differences -> PASS.
  AppliedPlan apn = ap;
  for (auto& b : apn.boards) b.delay_change = row_of(bp, b.module)->delay_nonneg;
  SynthBoards nn = sb;
  for (const BoardRow& r : bp.rows) nn.shift[r.module] = -r.delay_nonneg;
  std::map<int, Peak> pn = pulser_peaks(synth_boards(nn, 42), apn);
  int pass4 = 0;
  for (const VerifyRow& x : verify_boards(apn, &pb, pn)) pass4 += x.pass ? 1 : 0;
  CHECK(pass4 == 4);

  // sign = -1: the same timestamp shifts, opposite delay changes.
  BoardsPlan bm = plan_boards(before, K(1, 0), 500, pul, st, -1, o);
  CHECK(row_of(bm, 0)->delay_change == -32 && row_of(bm, 0)->ts_shift == -32);
  CHECK(row_of(bm, 2)->delay_change == 48 && row_of(bm, 2)->delay_nonneg == 160);
  AppliedPlan apm;
  CHECK(parse_applied(boards_json(bm, {}), apm, errs) && apm.sign == -1);
  int pass5 = 0;
  for (const VerifyRow& x : verify_boards(apm, &pb, pa)) pass5 += x.pass ? 1 : 0;
  CHECK(pass5 == 4);  // the hardware moved the timestamps the same way

  // No pulser on the ref board -> warning, rows still planned.
  std::map<int, uint16_t> nopul = pul;
  nopul.erase(1);
  BoardsPlan bw = plan_boards(before, K(1, 0), 500, nopul, st, +1, o);
  CHECK(bw.warnings.size() == 1 && !row_of(bw, 0)->pulser_ok && row_of(bw, 0)->delay_change == 32);

  // parse_applied errors.
  CHECK(!parse_applied("{}", ap, errs));
  CHECK(!parse_applied("{\"sign\": 2, \"ref_module\": 0, \"window_ns\": 10, \"boards\": "
                       "[{\"module\": 0, \"step_ns\": 16, \"delay_change_ns\": 0, \"pulser\": 3}]}",
                       ap, errs) &&
        errs.size() == 2);
}

// ---------------------------------------------------------------------------
// Anti-Compton map
// ---------------------------------------------------------------------------
// Groups A = {0:0, 0:1}, B = {0:2, 0:3}, C = {0:4}; shields 1:0..1:5.
//   1:0 <- A (p 0.3, +80 ns), 1:1 <- A (p 0.2, +75 ns), 1:2 <- B (p 0.3, +85 ns),
//   1:3 independent noise (silent), 1:4 <- A and C equally (ambiguous), 1:5 no hits.
// A "cascade" process makes every detector/shield pair (except 1:3) weakly
// prompt-coincident — significant, but far below the partners.
static HitTimes synth_ac(unsigned seed) {
  std::mt19937_64 rng(seed);
  const double T = 2e9;
  HitTimes ht;
  std::normal_distribution<double> j6(0, 6), j3(0, 3);
  std::uniform_real_distribution<double> u(0, 1);
  for (int d = 0; d <= 4; ++d)
    for (double t : poisson(rng, 20e3, T)) {
      ht.add(K(0, d), t);
      if (d <= 1 && u(rng) < 0.3 - 0.1 * d) ht.add(K(1, d), t + 80 - 5 * d + j6(rng));
      if (d >= 2 && d <= 3 && u(rng) < 0.3) ht.add(K(1, 2), t + 85 + j6(rng));
      if ((d <= 1 || d == 4) && u(rng) < (d == 4 ? 0.2 : 0.1)) ht.add(K(1, 4), t + 60 + j6(rng));
    }
  for (double t : poisson(rng, 10e3, T)) ht.add(K(1, 3), t);
  for (double t : poisson(rng, 30e3, T)) {
    for (int d = 0; d <= 4; ++d)
      if (u(rng) < 0.05) ht.add(K(0, d), t + j3(rng));
    for (int s : {0, 1, 2, 4})
      if (u(rng) < 0.05) ht.add(K(1, s), t + 80 + j6(rng));
  }
  ht.sort_all();
  return ht;
}

static void test_ac_assignment_and_block() {
  HitTimes ht = synth_ac(77);
  std::string err;
  std::vector<Group> g;
  CHECK(parse_groups("A=0:0,0:1;B=0:2-3;C=0:4", ht.keys(), "", g, err));
  std::vector<uint16_t> sh;
  CHECK(parse_mc_list("1:0-5", ht.keys(), sh, err) && sh.size() == 6);
  MeasureOpts o;
  AcResult r = measure_ac(ht, g, sh, 1000, o, 0.5, 20);
  CHECK(r.sh.size() == 6);
  CHECK(r.sh[0].status == AcStatus::Assigned && r.sh[0].group == 0);
  CHECK(r.sh[1].status == AcStatus::Assigned && r.sh[1].group == 0);
  CHECK(r.sh[2].status == AcStatus::Assigned && r.sh[2].group == 1);
  CHECK(r.sh[3].status == AcStatus::Silent);
  CHECK(r.sh[4].status == AcStatus::Ambiguous && r.sh[4].candidates.size() == 2);
  CHECK(r.sh[5].status == AcStatus::NoHits && r.sh[5].hits == 0);
  // The cascade makes the non-partner cells significant too (why dominance exists).
  CHECK(r.cell[1][0].sig && r.cell[2][0].sig);
  CHECK(r.cell[0][0].frac > 10 * r.cell[1][0].frac);
  // Δt peaks (shield - detector).
  CHECK(near(r.cell[0][0].p.mode, 80, 2) && near(r.cell[1][2].p.mode, 85, 2));
  CHECK(near(r.chan[std::make_pair(K(0, 1), K(1, 1))].p.mode, 75, 2));
  CHECK(near(r.chan[std::make_pair(K(0, 0), K(1, 0))].frac, 0.3, 0.03));
  // Window: shield after the detector by ~70..95 ns -> pre 0, post ~95 + 20.
  CHECK(r.window_ok && r.pre == 0 && r.post > 105 && r.post < 125);
  CHECK(r.lo > 55 && r.hi < 105);

  // The block is a valid event_def v1.1 section.
  std::string blk = ac_block_json(r, {"synthetic"});
  CHECK(!blk.empty());
  std::string def = "{\"version\":\"1.1\",\"window_ns\":{\"pre\":1000,\"post\":1000},\"group_ns\":100,"
                    "\"triggers\":[{\"name\":\"T\",\"channels\":[[5,0]]}]," +
                    blk.substr(blk.find('{') + 1);
  EventDefParseResult pr = parse_event_def(def, std::numeric_limits<double>::infinity());
  for (const std::string& e : pr.errors) std::printf("  event_def error: %s\n", e.c_str());
  CHECK(pr.errors.empty());
  CHECK(pr.def.ac_pairs.size() == 2);
  if (pr.def.ac_pairs.size() == 2) {
    CHECK(pr.def.ac_pairs[0].name == "A" && pr.def.ac_pairs[0].detector.size() == 2 &&
          pr.def.ac_pairs[0].shield.size() == 2 && pr.def.ac_pairs[0].shield[1] == K(1, 1));
    CHECK(pr.def.ac_pairs[1].name == "B" && pr.def.ac_pairs[1].shield.size() == 1 &&
          pr.def.ac_pairs[1].shield[0] == K(1, 2));
  }
  CHECK(pr.def.ac_pre_ns == r.pre && pr.def.ac_post_ns == r.post);
  CHECK(blk.find("AMBIGUOUS, not paired: m1c4 -> A") != std::string::npos ||
        blk.find("AMBIGUOUS, not paired: m1c4 -> C") != std::string::npos);
  CHECK(blk.find("SILENT, not paired: m1c3") != std::string::npos);
  CHECK(blk.find("NO HITS, not paired: m1c5") != std::string::npos);
  CHECK(blk.find("NO assigned shield (no pair): C") != std::string::npos);

  // dominance 0 = pure significance: the cascade makes every partner ambiguous.
  AcResult r0 = measure_ac(ht, g, sh, 1000, o, 0.0, 20);
  CHECK(r0.sh[0].status == AcStatus::Ambiguous && r0.sh[2].status == AcStatus::Ambiguous);
  CHECK(r0.sh[3].status == AcStatus::Silent);
  CHECK(!r0.window_ok && ac_block_json(r0, {}).empty());

  // Window when the shield can come first: shift every shield by -150 ns.
  HitTimes early;
  for (const auto& kv : ht.t)
    for (double t : kv.second) early.add(kv.first, t - ((kv.first >> 8) == 1 ? 150.0 : 0.0));
  early.sort_all();
  AcResult re = measure_ac(early, g, sh, 1000, o, 0.5, 20);
  CHECK(re.window_ok && re.post == 0 && re.pre > 95 && re.pre < 120);
}

int main() {
  test_parse_specs();
  test_fill_vs_brute();
  test_peak_gaussian();
  test_peak_flat_not_significant();
  test_peak_sharp_and_low_stats();
  test_peak_broad_asymmetric();
  test_peak_not_flat_and_multiple();
  test_peak_at_search_edge();
  test_board_sum_fallback();
  test_steps_and_median();
  test_offsets_json_roundtrip();
  test_boards_plan_and_verify();
  test_ac_assignment_and_block();
  std::printf("\n%d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

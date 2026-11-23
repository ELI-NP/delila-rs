// test_cfd_scan.cpp — unit tests for the CFD scan logic (cfd_scan.hpp, TODO 90
// P3): digitizer JSON -> per-channel FW CFD settings, re-timing a hit, ThGEM
// grouping, time-sum width, the coordinate-descent search, the JSON patch.
//
//   g++ -std=c++17 -O0 -g -Wall -Wextra -I../delila2root -I../root_sink/thgem
//       test_cfd_scan.cpp -o /tmp/tcs && /tmp/tcs
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "cfd_scan.hpp"

using namespace cfdtune;

static int g_pass = 0;
static int g_fail = 0;

#define CHECK(cond)                                                      \
  do {                                                                   \
    if (cond) {                                                          \
      ++g_pass;                                                          \
    } else {                                                             \
      ++g_fail;                                                          \
      std::printf("FAIL %s:%d  CHECK(%s)\n", __FILE__, __LINE__, #cond); \
    }                                                                    \
  } while (0)

static bool near(double a, double b, double eps = 1e-9) { return std::fabs(a - b) <= eps; }

static bool has(const std::vector<std::string>& v, const std::string& needle) {
  for (const auto& e : v)
    if (e.find(needle) != std::string::npos) return true;
  return false;
}

// --- digitizer JSON -------------------------------------------------------

static const char* kJson = R"({
  "digitizer_id": 1, "firmware": "PSD1", "num_channels": 16,
  "channel_defaults": {"enabled": "True", "polarity": "POLARITY_NEGATIVE",
    "discriminator_mode": "DISCR_MODE_CFD", "trigger_threshold": 100,
    "cfd_delay_ns": 20, "cfd_fraction": "CFD_FRACTLIST_50", "cfd_interpolation_point": 0,
    "input_smoothing": "CFD_SMOOTH_EXP_8"},
  "channel_overrides": {
    "0": {"polarity": "POLARITY_POSITIVE", "cfd_delay_ns": 50, "cfd_fraction": "CFD_FRACTLIST_75",
          "input_smoothing": "CFD_SMOOTH_EXP_16"},
    "10": {"enabled": "False"},
    "15": {"trigger_threshold": 300, "input_smoothing": "CFD_SMOOTH_EXP_1"}
  }
})";

static void test_channel_settings_expand_defaults_and_overrides() {
  ChannelSettings cs = parse_channel_settings(kJson, 2.0);
  CHECK(cs.errors.empty());
  CHECK(cs.module == 1);
  CHECK(cs.ch.size() == 16);
  const ChannelFw& c1 = cs.ch.at(1);
  CHECK(c1.enabled && c1.cfd_mode);
  CHECK(c1.pol == Polarity::Negative);
  CHECK(c1.p.delay == 10 && c1.p.fraction_pct == 50 && c1.p.smooth == 8 && c1.p.interp == 0);
  CHECK(near(c1.threshold, 100));
  const ChannelFw& c0 = cs.ch.at(0);
  CHECK(c0.pol == Polarity::Positive);
  CHECK(c0.p.delay == 25 && c0.p.fraction_pct == 75 && c0.p.smooth == 16);
  CHECK(!cs.ch.at(10).enabled);
  CHECK(cs.ch.at(15).p.smooth == 1 && near(cs.ch.at(15).threshold, 300));
}

static void test_channel_settings_refuse_what_they_cannot_model() {
  // A delay that is not a whole number of samples, an unknown enum, LED mode.
  ChannelSettings a = parse_channel_settings(
      R"({"digitizer_id": 2, "num_channels": 2, "channel_defaults": {"cfd_delay_ns": 21,
          "cfd_fraction": "CFD_FRACTLIST_50", "input_smoothing": "CFD_SMOOTH_EXP_8",
          "polarity": "POLARITY_NEGATIVE", "discriminator_mode": "DISCR_MODE_CFD",
          "trigger_threshold": 50}})",
      2.0);
  CHECK(has(a.errors, "cfd_delay_ns 21"));
  ChannelSettings b = parse_channel_settings(
      R"({"digitizer_id": 2, "num_channels": 1, "channel_defaults": {"cfd_delay_ns": 20,
          "cfd_fraction": "CFD_FRACTLIST_33", "input_smoothing": "CFD_SMOOTH_EXP_8",
          "polarity": "POLARITY_NEGATIVE", "discriminator_mode": "DISCR_MODE_CFD",
          "trigger_threshold": 50}})",
      2.0);
  CHECK(has(b.errors, "CFD_FRACTLIST_33"));
  ChannelSettings c = parse_channel_settings(
      R"({"digitizer_id": 2, "num_channels": 1, "channel_defaults": {"cfd_delay_ns": 20,
          "cfd_fraction": "CFD_FRACTLIST_50", "input_smoothing": "CFD_SMOOTH_EXP_8",
          "polarity": "POLARITY_NEGATIVE", "discriminator_mode": "DISCR_MODE_LED",
          "trigger_threshold": 50}})",
      2.0);
  CHECK(c.errors.empty());
  CHECK(!c.ch.at(0).cfd_mode);
  CHECK(has(parse_channel_settings("{not json", 2.0).errors, "JSON"));
}

// --- re-timing one hit ------------------------------------------------------

// Negative pulse on a 14-bit baseline with a linear edge from t0 (samples).
static std::vector<short> neg_pulse(int n, double t0, double rise, double amp) {
  std::vector<short> v(n);
  for (int i = 0; i < n; ++i) {
    double x = (i - t0) / rise;
    double p = amp * (x <= 0 ? 0.0 : (x >= 1 ? 1.0 : x));
    v[i] = static_cast<short>(std::lround(8000.0 - p));
  }
  return v;
}

static ChannelFw fw(int delay, int frac, int smooth) {
  ChannelFw c;
  c.p = CfdParams{delay, frac, smooth, 0};
  c.threshold = 50;
  c.pol = Polarity::Negative;
  return c;
}

static void test_retime_keeps_the_fw_time_at_the_fw_setting_and_moves_by_the_model() {
  // Integer start and slope (20 ADC per sample): every sample is exact.
  auto adc = neg_pulse(500, 200.0, 60.0, 1200.0);
  ChannelFw c = fw(10, 50, 1);
  HitWave h;
  CHECK(make_hit(adc, c, 1000.0, 2.0, 64, h));
  CHECK(near(h.pos_fw, 220.0, 1e-9));
  // Same setting -> the FW time itself.
  double t = 0;
  CHECK(h.retime(c.p, c.threshold, 2.0, t));
  CHECK(near(t, 1000.0, 1e-9));
  // On a linear edge with D < rise the crossing is at t0 + D/(1-f): 12 samples
  // instead of 10 at 50 % moves it by 4 samples = 8 ns.
  CHECK(h.retime(CfdParams{12, 50, 1, 0}, c.threshold, 2.0, t));
  CHECK(near(t, 1008.0, 1e-9));
  // A setting with no crossing in the record is reported, not invented.
  CHECK(!h.retime(CfdParams{300, 50, 1, 0}, c.threshold, 2.0, t));
  // A pulse the FW setting itself does not time is not a usable hit.
  HitWave flat;
  CHECK(!make_hit(std::vector<short>(500, 8000), c, 1000.0, 2.0, 64, flat));
}

// --- grouping ---------------------------------------------------------------

static void test_grouping_takes_the_nearest_arm_within_the_window() {
  // One ThGEM: T at 1000 and 5000; XL has hits at 1100 and 1150 (nearest =
  // 1100) and none near 5000; XR at 900; YU/YD near both.
  std::vector<ChannelHits> ch(5);
  ch[0].t = {1000, 5000};
  ch[1].t = {1100, 1150};
  ch[2].t = {900, 4950};
  ch[3].t = {1200, 5200};
  ch[4].t = {800, 4800};
  for (int k = 0; k < 5; ++k)
    for (size_t i = 0; i < ch[k].t.size(); ++i) ch[k].hit.push_back(static_cast<int>(10 * k + i));
  auto ev = group_thgem(ch, 300.0);
  CHECK(ev.size() == 2);
  CHECK(ev[0].hit[0] == 0 && ev[0].hit[1] == 10 && ev[0].hit[2] == 20 && ev[0].hit[3] == 30 &&
        ev[0].hit[4] == 40);
  CHECK(ev[1].hit[0] == 1 && ev[1].hit[1] == -1 && ev[1].hit[2] == 21);
}

// --- width ------------------------------------------------------------------

static void test_sigma68_is_half_the_central_68_percent() {
  // 1..100: the 16th and 84th percentiles are 16.84 and 84.16 (linear).
  std::vector<double> v;
  for (int i = 1; i <= 100; ++i) v.push_back(i);
  CHECK(near(sigma68(v), (84.16 - 16.84) / 2.0, 1e-9));
  CHECK(std::isnan(sigma68({})));
}

static void test_time_sums_are_position_independent() {
  // Delay line: t_XL - t_T = a + x, t_XR - t_T = a - x for any x, so
  // t_XL + t_XR - 2 t_T = 2a for every event; a small jitter only on XL.
  std::vector<double> t = {0, 0, 0, 0, 0};
  std::vector<ThgemEvent> ev;
  std::vector<double> times;
  for (int i = 0; i < 200; ++i) {
    double x = (i % 37) * 3.1, y = (i % 23) * 2.7, j = ((i % 5) - 2) * 0.1;
    double tt = 1e4 * i;
    ThgemEvent e;
    for (int k = 0; k < 5; ++k) e.hit[k] = static_cast<int>(times.size()) + k;
    times.insert(times.end(), {tt, tt + 50 + x + j, tt + 50 - x, tt + 60 + y, tt + 60 - y});
    ev.push_back(e);
  }
  TimeSums s = time_sums(ev, times);
  CHECK(s.x.size() == 200 && s.y.size() == 200);
  // Times up to 2e6 ns: sums are exact to ~1e-9, so compare at 1e-6.
  CHECK(near(*std::min_element(s.y.begin(), s.y.end()), 120.0, 1e-6));
  CHECK(near(*std::max_element(s.y.begin(), s.y.end()), 120.0, 1e-6));
  CHECK(sigma68(s.x) > 0.05 && sigma68(s.x) < 0.25);
  CHECK(near(sigma68(s.y), 0.0, 1e-6));
  // An event with a missing arm or a NaN time is left out of that sum only.
  ev[0].hit[1] = -1;
  times[ev[1].hit[3]] = std::nan("");
  TimeSums s2 = time_sums(ev, times);
  CHECK(s2.x.size() == 199 && s2.y.size() == 199);
}

// --- search -----------------------------------------------------------------

static void test_coordinate_descent_finds_a_separable_minimum() {
  Grid g;
  g.delays = {2, 4, 6, 8, 10, 12};
  g.fractions = {25, 50, 75, 100};
  g.smooths = {1, 2, 4, 8, 16};
  // Separable cost with the minimum at T = (6, 75, 4) and arms = (10, 25, 2).
  auto cost = [](const CfdParams& t, const CfdParams& a) {
    auto sq = [](double x) { return x * x; };
    return sq(t.delay - 6) + sq((t.fraction_pct - 75) / 25.0) + sq(std::log2(t.smooth) - 2) +
           sq(a.delay - 10) + sq((a.fraction_pct - 25) / 25.0) + sq(std::log2(a.smooth) - 1);
  };
  SearchResult r = coordinate_descent(g, CfdParams{2, 50, 8, 0}, CfdParams{12, 100, 16, 0}, cost, 6);
  CHECK(r.t.delay == 6 && r.t.fraction_pct == 75 && r.t.smooth == 4);
  CHECK(r.arm.delay == 10 && r.arm.fraction_pct == 25 && r.arm.smooth == 2);
  CHECK(near(r.cost, 0.0));
  CHECK(r.rounds >= 1 && r.rounds <= 6);
  CHECK(r.evaluations > 0);
}

// --- patch ------------------------------------------------------------------

static void test_patch_json_uses_the_digitizer_names_and_ns() {
  std::vector<PatchEntry> p = {{1, 0, CfdParams{25, 75, 16, 0}}, {1, 1, CfdParams{10, 50, 1, 0}},
                               {2, 5, CfdParams{4, 25, 2, 0}}};
  std::string j = patch_json(p, 2.0);
  auto v = tdelila::json::parse(j);
  const auto* m1 = v.find("1");
  CHECK(m1 && m1->t == tdelila::json::Value::T::Obj);
  const auto* o = m1 ? m1->find("channel_overrides") : nullptr;
  const auto* c0 = o ? o->find("0") : nullptr;
  CHECK(c0 && c0->find("cfd_delay_ns") && near(c0->find("cfd_delay_ns")->num, 50));
  CHECK(c0 && c0->find("cfd_fraction") && c0->find("cfd_fraction")->str == "CFD_FRACTLIST_75");
  CHECK(c0 && c0->find("input_smoothing") && c0->find("input_smoothing")->str == "CFD_SMOOTH_EXP_16");
  const auto* m2 = v.find("2");
  CHECK(m2 && m2->find("channel_overrides") && m2->find("channel_overrides")->find("5"));
}

// Dual trace (DIG1 VPROBE_CFD on): every value is written twice and the record
// still says 2 ns/sample (pf-qaq DAQ_Test run 17, 2026-11-21).
static void test_dual_trace_records_are_recognised() {
  CHECK(pairs_duplicated({5, 5, 9, 9, -3, -3, 7, 7}));
  CHECK(!pairs_duplicated({5, 5, 9, 8, -3, -3, 7, 7}));  // one pair differs: single trace
  CHECK(!pairs_duplicated({5, 5, 9, 9, -3}));            // odd length
  CHECK(!pairs_duplicated({5, 5}));                      // too short to tell
  CHECK(!pairs_duplicated({}));
}

static void test_undouble_keeps_the_even_samples_and_fills_the_odd_ones() {
  // Linear: the mean of the two neighbours; the last odd sample repeats the last value.
  std::vector<short> lin = undouble({0, 0, 10, 10, 20, 20, 40, 40}, Undouble::Linear);
  CHECK((lin == std::vector<short>{0, 5, 10, 15, 20, 30, 40, 40}));
  // Cubic (4-point Lagrange at the midpoint) is exact for a cubic polynomial
  // away from the edges: v(k) = k^3 at the even samples, (k + 1/2)^3 * 8 = (2k+1)^3 in between
  // with the values scaled by 8 to stay integer.
  std::vector<short> dbl;
  for (int k = 0; k < 8; ++k) {
    short v = static_cast<short>(8 * k * k * k);
    dbl.push_back(v);
    dbl.push_back(v);
  }
  std::vector<short> cub = undouble(dbl, Undouble::Cubic);
  CHECK(cub.size() == dbl.size());
  for (int k = 0; k < 8; ++k) CHECK(cub[2 * k] == 8 * k * k * k);
  for (int k = 1; k <= 5; ++k) CHECK(cub[2 * k + 1] == (2 * k + 1) * (2 * k + 1) * (2 * k + 1));
  // Not a dual-trace record: returned unchanged.
  CHECK((undouble({1, 2, 3, 4}, Undouble::Linear) == std::vector<short>{1, 2, 3, 4}));
}

int main() {
  test_dual_trace_records_are_recognised();
  test_undouble_keeps_the_even_samples_and_fills_the_odd_ones();
  test_channel_settings_expand_defaults_and_overrides();
  test_channel_settings_refuse_what_they_cannot_model();
  test_retime_keeps_the_fw_time_at_the_fw_setting_and_moves_by_the_model();
  test_grouping_takes_the_nearest_arm_within_the_window();
  test_sigma68_is_half_the_central_68_percent();
  test_time_sums_are_position_independent();
  test_coordinate_descent_finds_a_separable_minimum();
  test_patch_json_uses_the_digitizer_names_and_ns();
  std::printf("%d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

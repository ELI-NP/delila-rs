// test_cfd_emu.cpp — unit tests for the software model of the V1730 DPP-PSD CFD
// (cfd_emu.hpp, TODO 90 P1). Synthetic pulses with known answers; no ROOT:
//
//   g++ -std=c++17 -O0 -g -Wall -Wextra test_cfd_emu.cpp -o /tmp/tce && /tmp/tce
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

#include "cfd_emu.hpp"

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

// A positive-going pulse sampled at integer positions: 0 before t0, a linear ramp
// of `rise` samples up to `amp`, then flat (a step with a linear edge).
static std::vector<double> ramp_step(int n, double t0, double rise, double amp) {
  std::vector<double> v(n);
  for (int i = 0; i < n; ++i) {
    double x = (i - t0) / rise;
    v[i] = amp * (x <= 0 ? 0.0 : (x >= 1 ? 1.0 : x));
  }
  return v;
}

// Raw ADC samples as the digitizer records them: baseline + polarity * pulse.
static std::vector<short> to_adc(const std::vector<double>& p, double baseline, int polarity) {
  std::vector<short> v(p.size());
  for (size_t i = 0; i < p.size(); ++i) v[i] = static_cast<short>(std::lround(baseline + polarity * p[i]));
  return v;
}

static void test_crossing_of_a_linear_edge_is_exact() {
  // Continuous answer for c(t) = f*s(t) - s(t-D) on a linear edge with D >= rise:
  // the delayed edge meets f*amp at t = t0 + D + f*rise. Linear interpolation is
  // exact when both samples lie on the same straight piece.
  for (double t0 : {100.0, 100.3, 100.77}) {
    for (int frac : {25, 50, 75}) {
      auto s = ramp_step(400, t0, 10.0, 1000.0);
      CfdParams p{12, frac, 1, 0};
      Crossing c = cfd_crossing(s, p, 50.0);
      CHECK(c.found);
      CHECK(near(c.pos, t0 + 12 + frac / 100.0 * 10.0, 1e-9));
    }
  }
}

// A linear rise of `rise` samples to `amp`, then a linear fall of `slope` per
// sample (a real pulse comes down again; a flat top would make f = 100 % touch
// zero and stay there, which is not a crossing).
static std::vector<double> ramp_peak(int n, double t0, double rise, double amp, double slope) {
  std::vector<double> v(n);
  for (int i = 0; i < n; ++i) {
    double t = i - t0;
    v[i] = t <= 0 ? 0.0 : (t <= rise ? amp * t / rise : std::max(0.0, amp - (t - rise) * slope));
  }
  return v;
}

static void test_fraction_100_on_a_falling_top() {
  // f = 100 %: c(t) = s(t) - s(t-D). On [t0+D, t0+D+rise] both pieces are
  // straight lines, and c = 0 at x = (amp - (D-rise)*slope) / (slope + amp/rise)
  // after t0 + D.
  const double t0 = 50.25, rise = 8.0, amp = 500.0, slope = 2.0;
  const int d = 20;
  auto s = ramp_peak(400, t0, rise, amp, slope);
  Crossing c = cfd_crossing(s, CfdParams{d, 100, 1, 0}, 50.0);
  double x = (amp - (d - rise) * slope) / (slope + amp / rise);
  CHECK(c.found);
  CHECK(near(c.pos, t0 + d + x, 1e-9));
}

static void test_interpolation_points_do_not_move_a_straight_line() {
  // With k-th samples before and after, the interpolation is still exact on a
  // straight piece long enough to hold both samples.
  auto s = ramp_step(400, 100.4, 40.0, 2000.0);
  for (int k = 0; k <= 3; ++k) {
    Crossing c = cfd_crossing(s, CfdParams{50, 50, 1, k}, 50.0);
    CHECK(c.found);
    CHECK(near(c.pos, 100.4 + 50 + 0.5 * 40.0, 1e-9));
  }
}

static void test_moving_average_delays_a_ramp_by_half_its_length_minus_one() {
  // A causal n-sample moving average of a long linear ramp is the same ramp
  // delayed by (n-1)/2 samples, so the crossing moves by exactly that.
  auto s = ramp_step(600, 100.0, 200.0, 4000.0);
  Crossing c1 = cfd_crossing(s, CfdParams{60, 50, 1, 0}, 50.0);
  for (int n : {2, 4, 8, 16}) {
    auto sm = moving_average(s, n);
    Crossing cn = cfd_crossing(sm, CfdParams{60, 50, 1, 0}, 50.0);
    CHECK(c1.found && cn.found);
    CHECK(near(cn.pos - c1.pos, (n - 1) / 2.0, 1e-9));
  }
  // smooth = 1 is the identity.
  auto same = moving_average(s, 1);
  CHECK(same == s);
}

static void test_no_crossing_below_the_threshold_or_without_a_pulse() {
  std::vector<double> flat(300, 0.0);
  CHECK(!cfd_crossing(flat, CfdParams{10, 50, 1, 0}, 20.0).found);
  // A pulse that never reaches the arming threshold is not timed.
  auto small = ramp_step(300, 100.0, 5.0, 10.0);
  CHECK(!cfd_crossing(small, CfdParams{10, 50, 1, 0}, 20.0).found);
  // A crossing too close to the end of the record (no k-th sample after it).
  auto late = ramp_step(130, 100.0, 5.0, 1000.0);
  CHECK(!cfd_crossing(late, CfdParams{40, 50, 1, 0}, 20.0).found);
}

static void test_prepare_removes_baseline_and_polarity() {
  // A negative pulse on a 14-bit baseline becomes the same positive pulse.
  auto pulse = ramp_step(400, 120.0, 10.0, 800.0);
  auto adc = to_adc(pulse, 8000.0, -1);
  auto s = prepare(adc, Polarity::Negative, 64);
  CHECK(s.size() == adc.size());
  CHECK(near(s[0], 0.0, 1e-9));
  CHECK(near(s[399], 800.0, 1e-9));
  Crossing a = cfd_crossing(s, CfdParams{12, 50, 1, 0}, 50.0);
  Crossing b = cfd_crossing(pulse, CfdParams{12, 50, 1, 0}, 50.0);
  CHECK(a.found && b.found);
  // ADC rounding moves the crossing by far less than a sample.
  CHECK(near(a.pos, b.pos, 0.01));
  // Positive pulses keep their sign.
  auto padc = to_adc(pulse, 1000.0, +1);
  auto ps = prepare(padc, Polarity::Positive, 64);
  CHECK(near(ps[399], 800.0, 1e-9));
}

static void test_time_ns_and_fine_fraction() {
  // pos 117.3 samples at 2 ns = 234.6 ns; the FW's 10-bit fine time is the
  // fraction of a sample after the "before" sample, floor(0.3 * 1024) = 307.
  Crossing c;
  c.found = true;
  c.pos = 117.3;
  CHECK(near(c.time_ns(2.0), 234.6, 1e-9));
  CHECK(c.fine10() == 307);
  // Circular difference of two fractions, in samples, wrapped to (-0.5, 0.5].
  CHECK(near(frac_diff(0.98, 0.02), -0.04, 1e-12));
  CHECK(near(frac_diff(0.02, 0.98), 0.04, 1e-12));
  CHECK(near(frac_diff(0.30, 0.25), 0.05, 1e-12));
}

// Noise above the threshold early in the record arms the search there; the FW
// arms near its trigger, so the model can be told where to start (`from`).
static void test_search_can_start_after_early_noise() {
  auto pulse = ramp_step(400, 200.0, 10.0, 800.0);
  for (int i = 20; i < 30; ++i) pulse[static_cast<size_t>(i)] += (i < 25 ? 300.0 : -300.0);  // a noise burst
  Crossing early = cfd_crossing(pulse, CfdParams{12, 50, 1, 0}, 100.0);
  Crossing late = cfd_crossing(pulse, CfdParams{12, 50, 1, 0}, 100.0, 150);
  Crossing clean = cfd_crossing(ramp_step(400, 200.0, 10.0, 800.0), CfdParams{12, 50, 1, 0}, 100.0);
  CHECK(early.found && early.pos < 40.0);
  CHECK(late.found && clean.found);
  CHECK(near(late.pos, clean.pos, 1e-12));
  CHECK(!cfd_crossing(pulse, CfdParams{12, 50, 1, 0}, 100.0, 399).found);  // nothing after the end
}

int main() {
  test_search_can_start_after_early_noise();
  test_crossing_of_a_linear_edge_is_exact();
  test_fraction_100_on_a_falling_top();
  test_interpolation_points_do_not_move_a_straight_line();
  test_moving_average_delays_a_ramp_by_half_its_length_minus_one();
  test_no_crossing_below_the_threshold_or_without_a_pulse();
  test_prepare_removes_baseline_and_polarity();
  test_time_ns_and_fine_fraction();
  std::printf("%d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

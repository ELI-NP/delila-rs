// test_thgem_reco.cpp — unit tests for the ThGEM reconstruction on generic
// events (thgem_reco.hpp: thgem_from_event, TODO 71 Phase B). Split out of
// ../common/test_eb_core.cpp when tools/root_sink/ was divided into common/
// standard/ thgem/ (2026-10-30).
//
//   g++ -std=c++17 -O0 -g test_thgem_reco.cpp -o /tmp/tthg && /tmp/tthg
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <vector>

#include "../common/eb_core.hpp"
#include "thgem_reco.hpp"

using namespace rootsink;
using namespace rootsink::eb;

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

static bool near(double a, double b, double eps = 1e-9) {
  double d = a - b;
  if (d < 0) d = -d;
  return d <= eps;
}

static ScalarHit hit(int ch, double t, uint16_t e = 0) {
  ScalarHit h;
  h.channel = static_cast<uint8_t>(ch);
  h.timestamp_ns = t;
  h.energy = e;
  return h;
}

static BuilderConfig side3_cfg(double window = 500.0) {
  BuilderConfig c;
  c.trig1 = 1; c.xl1 = 2; c.xr1 = 3; c.yu1 = 4; c.yd1 = 5;
  c.trig2 = 6; c.xl2 = 7; c.xr2 = 8; c.yu2 = 9; c.yd2 = 10;
  c.labr_ch = 0;
  c.window_ns = window;
  return c;
}

// A generic event by hand: reference at t_ref, hits given in ABSOLUTE time.
static GenericEvent gev(double t_ref, int ref_ch, std::vector<ScalarHit> hits,
                        const BuilderConfig& cfg) {
  std::sort(hits.begin(), hits.end(), [](const ScalarHit& a, const ScalarHit& b) {
    return a.timestamp_ns < b.timestamp_ns;
  });
  GenericEvent e;
  e.t = t_ref;
  e.ch = static_cast<uint8_t>(ref_ch);
  for (const ScalarHit& h : hits) {
    EventHit eh;
    eh.mod = h.module;
    eh.ch = h.channel;
    eh.energy = h.energy;
    eh.dt = h.timestamp_ns - t_ref;
    eh.is_trigger = h.channel == cfg.trig1 || h.channel == cfg.trig2;
    if (eh.is_trigger) ++e.n_triggers;
    e.hits.push_back(eh);
  }
  return e;
}

static void test_thgem_from_event_plane1() {
  BuilderConfig cfg = side3_cfg(500.0);
  GenericEvent e = gev(10000.0, 1,
                       {hit(1, 10000.0, 100), hit(2, 10040.0, 11), hit(3, 10010.0, 12),
                        hit(4, 10200.0, 13), hit(5, 10150.0, 14), hit(0, 9900.0, 50)},
                       cfg);
  BuiltEvent b;
  CHECK(thgem_from_event(e, cfg, b));
  CHECK(near(b.trig1_t, 10000.0) && std::isnan(b.trig2_t) && b.e_trig1 == 100);
  CHECK(near(b.x1, 10.0 - 40.0) && near(b.y1, 200.0 - 150.0));
  CHECK(b.n_arms1 == 4 && b.e_sum1 == 11u + 12u + 13u + 14u);
  CHECK(near(b.dt_xl1, 40.0) && near(b.dt_yd1, 150.0));
  CHECK(near(b.labr_t, 9900.0) && near(b.tof1, 100.0) && b.e_labr == 50);
  CHECK(std::isnan(b.tof2) && std::isnan(b.dt_trig) && b.n_arms2 == 0);
}

static void test_thgem_from_event_both_planes() {
  // Reference = ThGEM1 (priority), but ThGEM2 fired 30 ns EARLIER: the LaBr3
  // match is anchored on the earlier trigger (the retired chunk builder's rule).
  BuilderConfig cfg = side3_cfg(500.0);
  GenericEvent e = gev(5030.0, 1,
                       {hit(6, 5000.0, 7), hit(1, 5030.0, 8), hit(7, 5060.0, 1),
                        hit(8, 5080.0, 2), hit(0, 4500.0, 3), hit(0, 5530.0, 4)},
                       cfg);
  BuiltEvent b;
  CHECK(thgem_from_event(e, cfg, b));
  CHECK(near(b.trig1_t, 5030.0) && near(b.trig2_t, 5000.0));
  CHECK(near(b.dt_trig, -30.0));
  CHECK(near(b.x2, 80.0 - 60.0) && b.n_arms2 == 2 && b.n_arms1 == 0);
  // LaBr3 at 4500 is 500 from the anchor 5000 (inclusive), 5530 is 530: 4500.
  CHECK(near(b.labr_t, 4500.0) && b.e_labr == 3);
  CHECK(near(b.tof1, 530.0) && near(b.tof2, 500.0));
}

static void test_thgem_from_event_window_and_ties() {
  // The generic window (±1000) is wider than the builder's ±500: an arm 600
  // away is in the event but must not match. Two candidates at ±20: the tie
  // goes to the successor (nearest_in rule).
  BuilderConfig cfg = side3_cfg(500.0);
  GenericEvent e = gev(0.0, 1,
                       {hit(1, 0.0), hit(2, 600.0), hit(3, -20.0, 1), hit(3, 20.0, 2),
                        hit(4, -500.0, 9)},
                       cfg);
  BuiltEvent b;
  CHECK(thgem_from_event(e, cfg, b));
  CHECK(std::isnan(b.dt_xl1));
  CHECK(near(b.dt_xr1, 20.0) && b.e_xr1 == 2);
  CHECK(near(b.dt_yu1, -500.0));  // window inclusive
  CHECK(b.n_arms1 == 2);
}

static void test_thgem_from_event_partial_and_trigger_only() {
  // Completeness is an offline cut: a plane with 2 of 4 arms keeps its row
  // (x from the complete X axis, y NaN, n_arms 2); a lone trigger is a row
  // too (efficiency denominator), everything else NaN / 0.
  BuilderConfig cfg = side3_cfg(500.0);
  BuiltEvent b;
  CHECK(thgem_from_event(gev(0.0, 1, {hit(1, 0.0), hit(2, 30.0, 3), hit(3, 50.0, 4)}, cfg), cfg, b));
  CHECK(near(b.x1, 20.0) && std::isnan(b.y1) && b.n_arms1 == 2 && b.e_sum1 == 7u);
  BuiltEvent t;
  CHECK(thgem_from_event(gev(0.0, 1, {hit(1, 0.0, 42)}, cfg), cfg, t));
  CHECK(t.e_trig1 == 42 && t.n_arms1 == 0 && std::isnan(t.x1) && std::isnan(t.labr_t));
}

static void test_thgem_from_event_labr_disabled() {
  // labr_ch = -1 (no --gamma-ch): a ch0 hit is just another hit; no TOF.
  BuilderConfig cfg = side3_cfg(500.0);
  cfg.labr_ch = -1;
  BuiltEvent b;
  CHECK(thgem_from_event(gev(0.0, 1, {hit(1, 0.0), hit(0, -100.0, 9)}, cfg), cfg, b));
  CHECK(std::isnan(b.labr_t) && std::isnan(b.tof1) && b.e_labr == 0);
}

static void test_thgem_from_event_no_thgem_trigger() {
  // An event whose triggers are none of the ThGEM planes (e.g. a LaBr3
  // trigger in a wider definition) has no built row.
  BuilderConfig cfg = side3_cfg(500.0);
  GenericEvent e = gev(0.0, 0, {hit(0, 0.0), hit(2, 30.0)}, cfg);
  BuiltEvent b;
  CHECK(!thgem_from_event(e, cfg, b));
}

int main() {
  test_thgem_from_event_plane1();
  test_thgem_from_event_both_planes();
  test_thgem_from_event_window_and_ties();
  test_thgem_from_event_partial_and_trigger_only();
  test_thgem_from_event_labr_disabled();
  test_thgem_from_event_no_thgem_trigger();

  std::printf("\ntest_thgem_reco: %d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

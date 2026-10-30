// test_nwall.cpp — unit tests for the N_Wall sink's pure logic: the channel
// map (nwall_map.hpp) and the bunch clock (bunch_clock.hpp). No ROOT, no ZMQ:
//
//   g++ -std=c++17 -O0 -g test_nwall.cpp -o /tmp/tnw && /tmp/tnw
//
// License: BSD-3-Clause (same as delila-rs).

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "bunch_clock.hpp"
#include "nwall_map.hpp"

using namespace nwall;

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

static bool near(double a, double b, double eps = 1e-6) { return std::fabs(a - b) <= eps; }

static bool has_err(const MapParseResult& r, const std::string& needle) {
  for (const auto& e : r.errors)
    if (e.find(needle) != std::string::npos) return true;
  return false;
}

// ---------------------------------------------------------------------------
// Channel map
// ---------------------------------------------------------------------------
static void test_map_valid() {
  const char* text =
      "# N_Wall map\n"
      "\n"
      "0  0  beam     timing  beam   -      -\n"
      "0  1  LaBr1    LaBr    LaBr   -      -   # comment after a role\n"
      "0  8  LS01     LS      LS     -      -\n"
      "1  3  dE_S03   dE_sec  dE     sector 3\n"
      "2  14 dE_Ri14  dE_in   dE     ring   14  extra analysis columns\n"
      "4  0  dE_Ro00  dE_out  dE     ring   30\n"
      "5  15 E_S15    E_sec   E      sector 15\n"
      "2  15 pls2     pulser  pulser -      -\n"
      "3  7  spare    misc\n"   // Grafana-only: no role
      "3  8  unused   misc    -      -      -\n";
  MapParseResult r = parse_channel_map(text);
  CHECK(r.errors.empty());
  const ChannelMap& m = r.map;
  CHECK(m.names.size() == 10);
  CHECK(m.roles.size() == 8);  // the two role-less lines have none
  CHECK(m.beam_key == map_key(0, 0));
  const Role* ro = m.role(2, 14);
  CHECK(ro && ro->det == Det::DE && ro->kind == Kind::Ring && ro->index == 14);
  ro = m.role(5, 15);
  CHECK(ro && ro->det == Det::E && ro->kind == Kind::Sector && ro->index == 15);
  ro = m.role(0, 8);
  CHECK(ro && ro->det == Det::LS && ro->kind == Kind::None && ro->index == -1);
  CHECK(m.role(3, 7) == nullptr && m.role(3, 8) == nullptr && m.role(9, 9) == nullptr);
  CHECK(m.count(Det::DE) == 3 && m.count(Det::LS) == 1 && m.count(Det::Pulser) == 1);
  CHECK(m.names.at(map_key(1, 3)) == "dE_S03");
}

static void test_map_no_beam() {
  MapParseResult r = parse_channel_map("0 8 LS01 LS LS - -\n");
  CHECK(r.errors.empty() && r.map.beam_key == -1);
}

static void test_map_errors_all_reported() {
  const char* text =
      "0 0 beam\n"                           // 1: too few columns
      "x 0 a g\n"                            // 2: module not an integer
      "0 256 a g\n"                          // 3: channel out of range
      "0 1 a g LS\n"                         // 4: role incomplete
      "0 2 a g LS -\n"                       // 5: role incomplete
      "0 3 a g Ls - -\n"                     // 6: unknown det (case matters)
      "0 4 a g dE wedge 1\n"                 // 7: unknown kind
      "0 5 a g dE ring x\n"                  // 8: bad index
      "0 6 a g dE - 1\n"                     // 9: silicon without kind
      "0 7 a g E sector -\n"                 // 10: silicon without index
      "0 8 a g LS ring 1\n"                  // 11: non-silicon with kind
      "0 9 a g beam - -\n"                   // 12: first beam (fine)
      "0 10 b g beam - -\n"                  // 13: second beam
      "0 9 c g LS - -\n";                    // 14: duplicate of line 12
  MapParseResult r = parse_channel_map(text, "map.txt");
  CHECK(r.errors.size() == 13);
  CHECK(has_err(r, "map.txt:1: expected"));
  CHECK(has_err(r, "map.txt:2: module 'x'"));
  CHECK(has_err(r, "map.txt:3: channel '256'"));
  CHECK(has_err(r, "map.txt:4: a role needs three columns"));
  CHECK(has_err(r, "map.txt:5: a role needs three columns"));
  CHECK(has_err(r, "map.txt:6: det 'Ls'"));
  CHECK(has_err(r, "map.txt:7: kind 'wedge'"));
  CHECK(has_err(r, "map.txt:8: index 'x'"));
  CHECK(has_err(r, "map.txt:9: det dE needs kind"));
  CHECK(has_err(r, "map.txt:10: det E needs kind"));
  CHECK(has_err(r, "map.txt:11: det LS takes kind '-'"));
  CHECK(has_err(r, "map.txt:13: a second beam channel (the first is on line 12)"));
  CHECK(has_err(r, "map.txt:14: module 0 channel 9 is already listed on line 12"));
  CHECK(r.map.beam_key == map_key(0, 9));
}

// ---------------------------------------------------------------------------
// Bunch clock
// ---------------------------------------------------------------------------
static BunchClock::Config clock_cfg(double nominal) {
  BunchClock::Config c;
  c.period_ns = nominal;
  c.max_gap_ns = 1e6;
  c.min_gaps = 10;
  return c;
}

static void test_clock_empty() {
  BunchClock c;
  double tb = -1;
  CHECK(!c.bunch_at_or_before(1000.0, tb) && !c.nearest_bunch(1000.0, tb));
  CHECK(near(c.period_ns(), 199.980) && !c.period_measured());
}

static void test_clock_grid_from_one_beam_hit() {
  const double T = 200.0;
  BunchClock c(clock_cfg(T));
  c.add_beam(10000.0);
  double tb = 0;
  CHECK(c.bunch_at_or_before(10000.0, tb) && near(tb, 10000.0));
  CHECK(c.bunch_at_or_before(10199.9, tb) && near(tb, 10000.0));
  CHECK(c.bunch_at_or_before(10200.0, tb) && near(tb, 10200.0));
  CHECK(c.bunch_at_or_before(10650.0, tb) && near(tb, 10600.0));
  // Before the beam hit: floor goes to the earlier bunch.
  CHECK(c.bunch_at_or_before(9999.0, tb) && near(tb, 9800.0));
  CHECK(c.nearest_bunch(10650.0, tb) && near(tb, 10600.0));
  CHECK(c.nearest_bunch(10699.0, tb) && near(tb, 10600.0));
  CHECK(c.nearest_bunch(10701.0, tb) && near(tb, 10800.0));
}

// The true period differs from the nominal one: after min_gaps gaps the
// measured T takes over, and bunches far from a beam hit land on the true grid.
static void test_clock_measures_period() {
  const double T_true = 199.950, T_nominal = 199.980;
  BunchClock c(clock_cfg(T_nominal));
  const double t0 = 5000.0;
  for (int i = 0; i < 20; ++i) {
    // every 100th bunch, with a small (±0.5 ns) jitter on the beam signal
    const double jitter = (i % 3 - 1) * 0.5;
    c.add_beam(t0 + i * 100 * T_true + jitter);
  }
  CHECK(c.period_measured() && c.gaps() == 19 && c.off_grid() == 0);
  CHECK(std::fabs(c.period_ns() - T_true) < 1e-3);
  // 50 bunches after beam hit 10 (exactly between two beam hits).
  double tb = 0;
  const double t = t0 + 10 * 100 * T_true + 50 * T_true + 30.0;
  CHECK(c.bunch_at_or_before(t, tb));
  CHECK(std::fabs(tb - (t0 + 1050 * T_true)) < 1.0);
}

static void test_clock_nearest_side() {
  const double T = 200.0;
  BunchClock c(clock_cfg(T));
  c.add_beam(0.0);
  c.add_beam(20000.0 + 3.0);  // the later hit carries a +3 ns phase
  double tb = 0;
  // 19 µs: the later beam hit is nearer, so its phase wins.
  // floor((19050 - 20003) / 200) = -5 -> 20003 - 1000.
  CHECK(c.bunch_at_or_before(19050.0, tb) && near(tb, 19003.0));
  // 1 µs: the earlier hit is nearer, phase 0.
  CHECK(c.bunch_at_or_before(1050.0, tb) && near(tb, 1000.0));
}

static void test_clock_off_grid_and_beam_off() {
  const double T = 200.0;
  BunchClock::Config cfg = clock_cfg(T);
  cfg.max_gap_ns = 1e5;  // 100 µs
  BunchClock c(cfg);
  c.add_beam(0.0);
  c.add_beam(20000.0 + 90.0);  // 100.45 periods: off the grid
  CHECK(c.off_grid() == 1 && c.gaps() == 0);
  c.add_beam(1e7);             // 9.98 ms gap: beam off, ignored (not off-grid)
  CHECK(c.off_grid() == 1 && c.gaps() == 0);
  double tb = 0;
  CHECK(!c.bunch_at_or_before(5e6, tb));  // 5 ms from any beam hit: no bunch
  CHECK(c.bunch_at_or_before(1e7 + 450.0, tb) && near(tb, 1e7 + 400.0));
}

static void test_clock_late_insert_and_reset() {
  const double T = 200.0;
  BunchClock c(clock_cfg(T));
  c.add_beam(0.0);
  c.add_beam(40000.0);
  c.add_beam(20000.0 + 7.0);  // late: inserted in place, no gap measured
  CHECK(c.beam_hits() == 3 && c.gaps() == 1);
  double tb = 0;
  CHECK(c.bunch_at_or_before(20100.0, tb) && near(tb, 20007.0));
  c.reset();
  CHECK(c.beam_hits() == 0 && c.gaps() == 0 && near(c.period_ns(), T));
  CHECK(!c.bunch_at_or_before(20100.0, tb));
}

static void test_clock_cap_bound() {
  BunchClock::Config cfg = clock_cfg(200.0);
  cfg.max_beam_hits = 4;
  BunchClock c(cfg);
  for (int i = 0; i < 10; ++i) c.add_beam(i * 20000.0);
  CHECK(c.beam_hits() == 4 && c.gaps() == 9 && c.capped() == 6);
}

// nwall_sink adds a whole chunk's beam hits before it looks anything up, so
// nothing may be dropped by count while a chunk is in hand (E2E 2026-10-30: a
// fixed 8192-hit buffer lost the first part of every 200 ms chunk). Old beam
// hits go only when the caller says they are no longer needed.
static void test_clock_forget_before() {
  BunchClock c(clock_cfg(200.0));
  for (int i = 0; i < 20000; ++i) c.add_beam(i * 20000.0);  // 400 ms of 1/100 beam
  CHECK(c.beam_hits() == 20000 && c.capped() == 0);
  double tb = 0;
  CHECK(c.bunch_at_or_before(1050.0, tb) && near(tb, 1000.0));  // the start is still there
  c.forget_before(200e6);
  CHECK(c.beam_hits() == 10000);
  CHECK(c.bunch_at_or_before(200e6 + 50.0, tb) && near(tb, 200e6));
  CHECK(!c.bunch_at_or_before(1050.0, tb));  // forgotten, and > max_gap from the rest
}

int main() {
  test_map_valid();
  test_map_no_beam();
  test_map_errors_all_reported();
  test_clock_empty();
  test_clock_grid_from_one_beam_hit();
  test_clock_measures_period();
  test_clock_nearest_side();
  test_clock_off_grid_and_beam_off();
  test_clock_late_insert_and_reset();
  test_clock_cap_bound();
  test_clock_forget_before();

  std::printf("\ntest_nwall: %d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

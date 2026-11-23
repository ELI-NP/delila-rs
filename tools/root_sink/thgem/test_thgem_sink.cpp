// test_thgem_sink.cpp — unit tests for thgem_sink's pure logic: the channel
// map (thgem_map.hpp) and the ThGEMs of one event (thgem_planes.hpp). No ROOT,
// no ZMQ:
//
//   g++ -std=c++17 -O0 -g -Wall -Wextra test_thgem_sink.cpp -o /tmp/tts && /tmp/tts
//
// License: BSD-3-Clause (same as delila-rs).

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "thgem_map.hpp"
#include "thgem_planes.hpp"

using namespace thgem;
using rootsink::eb::EventHit;
using rootsink::eb::GenericEvent;

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

static bool near(double a, double b, double eps = 1e-9) { return std::fabs(a - b) <= eps; }

static bool has(const std::vector<std::string>& v, const std::string& needle) {
  for (const auto& e : v)
    if (e.find(needle) != std::string::npos) return true;
  return false;
}

// Two ThGEMs on board 1 (ch0-4, ch5-9), pulser ch15; VX2730 as on N_Wall.
static const char* kMap =
    "# 11B_208Pb test map\n"
    "0  0  beam    timing  beam   -  -\n"
    "0  1  LaBr1   LaBr    LaBr   -  -\n"
    "0  8  LS01    LS      LS     -  -   # comment after a role\n"
    "1  0  T1      ThGEM1  ThGEM  T  1\n"
    "1  1  T1_XL   ThGEM1  ThGEM  XL 1\n"
    "1  2  T1_XR   ThGEM1  ThGEM  XR 1\n"
    "1  3  T1_YU   ThGEM1  ThGEM  YU 1\n"
    "1  4  T1_YD   ThGEM1  ThGEM  YD 1   extra analysis columns\n"
    "1  5  T2      ThGEM2  ThGEM  T  2\n"
    "1  6  T2_XL   ThGEM2  ThGEM  XL 2\n"
    "1  7  T2_XR   ThGEM2  ThGEM  XR 2\n"
    "1  8  T2_YU   ThGEM2  ThGEM  YU 2\n"
    "1  9  T2_YD   ThGEM2  ThGEM  YD 2\n"
    "1  15 pls1    pulser  pulser -  -\n"
    "1  14 spare   misc\n";  // Grafana-only: no role

// ---------------------------------------------------------------------------
// Channel map
// ---------------------------------------------------------------------------
static void test_map_valid() {
  MapParseResult r = parse_channel_map(kMap);
  CHECK(r.errors.empty());
  CHECK(r.warnings.empty());
  const ChannelMap& m = r.map;
  CHECK(m.names.size() == 15);
  CHECK(m.roles.size() == 14);
  CHECK(m.beam_key == map_key(0, 0));
  CHECK(m.count(Det::ThGEM) == 10);
  CHECK(m.count(Det::LaBr) == 1);
  CHECK(m.count(Det::LS) == 1);
  CHECK(m.count(Det::Pulser) == 1);
  CHECK(m.thgems.size() == 2);
  CHECK(m.thgems.at(1).complete());
  CHECK(m.thgems.at(2).complete());
  CHECK(m.thgems.at(1).key[static_cast<int>(Kind::T)] == map_key(1, 0));
  CHECK(m.thgems.at(2).key[static_cast<int>(Kind::YD)] == map_key(1, 9));
  const Role* r7 = m.role(1, 7);
  CHECK(r7 && r7->det == Det::ThGEM && r7->kind == Kind::XR && r7->index == 2);
  CHECK(m.role(1, 14) == nullptr);           // Grafana-only line
  CHECK(m.names.at(map_key(1, 14)) == "spare");
  CHECK(m.role(2, 0) == nullptr);            // not listed at all
}

static void test_map_errors() {
  auto bad = [](const char* text) { return parse_channel_map(text, "m.txt"); };
  CHECK(has(bad("1 0 x g ThGEM T\n").errors, "a role needs three columns"));
  CHECK(has(bad("1 0 x\n").errors, "expected 'module channel name group"));
  CHECK(has(bad("1 0 x g Si - -\n").errors, "det 'Si' is not one of"));
  CHECK(has(bad("1 0 x g ThGEM Z 1\n").errors, "kind 'Z' is not one of"));
  CHECK(has(bad("1 0 x g ThGEM T 100\n").errors, "index '100' is not an integer 0..99"));
  CHECK(has(bad("1 0 x g ThGEM T -\n").errors, "det ThGEM needs kind"));
  CHECK(has(bad("1 0 x g ThGEM - 1\n").errors, "det ThGEM needs kind"));
  CHECK(has(bad("0 1 x g LaBr T -\n").errors, "takes kind '-' and index '-'"));
  CHECK(has(bad("0 1 x g LS - 3\n").errors, "takes kind '-' and index '-'"));
  CHECK(has(bad("256 0 x g - - -\n").errors, "module '256'"));
  CHECK(has(bad("1 0 x g - - -\n1 0 y g - - -\n").errors, "m.txt:2: module 1 channel 0 is already listed on line 1"));
  CHECK(has(bad("0 0 b g beam - -\n0 1 c g beam - -\n").errors, "a second beam channel (the first is on line 1)"));
  CHECK(has(bad("1 0 a g ThGEM XL 3\n2 0 b g ThGEM XL 3\n").errors, "m.txt:2: ThGEM 3 XL is already on line 1"));
  // Errors carry the line number of the bad line.
  CHECK(has(bad("# c\n\n1 0 x g ThGEM Q 1\n").errors, "m.txt:3:"));
}

static void test_map_warnings() {
  // No trigger: nothing shown. Missing arm: no XY. Spread over boards.
  const char* text =
      "1 1 a g ThGEM XL 4\n"
      "2 0 b g ThGEM T  5\n"
      "2 1 c g ThGEM XL 5\n"
      "2 2 d g ThGEM XR 5\n"
      "2 3 e g ThGEM YU 5\n"
      "3 0 f g ThGEM T  6\n"
      "3 1 g g ThGEM XL 6\n"
      "3 2 h g ThGEM XR 6\n"
      "3 3 i g ThGEM YU 6\n"
      "4 4 j g ThGEM YD 6\n";
  MapParseResult r = parse_channel_map(text);
  CHECK(r.errors.empty());
  CHECK(has(r.warnings, "ThGEM 4 has no trigger"));
  CHECK(has(r.warnings, "ThGEM 5 has no YD: its XY image is not made"));
  CHECK(has(r.warnings, "ThGEM 6 is spread over modules 3,4"));
  CHECK(r.warnings.size() == 3);
  CHECK(!r.map.thgems.at(5).complete());
  CHECK(r.map.thgems.at(6).complete());
  // Only ThGEMs with a trigger enter the pair list.
  const std::vector<int> t = triggered_thgems(r.map);
  CHECK(t.size() == 2 && t[0] == 5 && t[1] == 6);
}

static void test_names_and_pairs() {
  CHECK(xy_name(3) == "xy_T3");
  CHECK(dt_pair_name(1, 2) == "dt_T2_vs_T1");  // y = ThGEM 2, x = ThGEM 1 (October's dt2_vs_dt1)
  CHECK(channel_hist_name("tof_rf", 1, 5) == "tof_rf_m1_ch05");
  // 8 ThGEMs -> 28 pairs, i < j, sorted.
  std::string text;
  for (int n = 1; n <= 8; ++n)
    text += std::to_string(1 + (n - 1) / 3) + " " + std::to_string(5 * ((n - 1) % 3)) +
            " T" + std::to_string(n) + " g ThGEM T " + std::to_string(n) + "\n";
  MapParseResult r = parse_channel_map(text);
  CHECK(r.errors.empty());
  const auto pairs = thgem_pairs(r.map);
  CHECK(pairs.size() == 28);
  CHECK(pairs.front() == std::make_pair(1, 2));
  CHECK(pairs.back() == std::make_pair(7, 8));
  bool ordered = true;
  for (const auto& p : pairs) ordered = ordered && p.first < p.second;
  CHECK(ordered);
  CHECK(keys_of(parse_channel_map(kMap).map, Det::ThGEM).size() == 10);
}

static void test_trigger_mismatch() {
  const ChannelMap m = parse_channel_map(kMap).map;
  // In step: no warning.
  CHECK(trigger_mismatch(m, {map_key(1, 0), map_key(1, 5)}).empty());
  // ThGEM 2's trigger missing from the definition; an extra definition trigger.
  const auto w = trigger_mismatch(m, {map_key(1, 0), map_key(1, 6)});
  CHECK(w.size() == 2);
  CHECK(has(w, "ThGEM 2 trigger (module 1 channel 5) is not a trigger of the event definition"));
  CHECK(has(w, "event-definition trigger module 1 channel 6 is not a ThGEM trigger"));
  // A map without ThGEMs: every definition trigger is reported.
  CHECK(trigger_mismatch(ChannelMap(), {map_key(3, 3)}).size() == 1);
}

// ---------------------------------------------------------------------------
// Planes of one event
// ---------------------------------------------------------------------------
static EventHit hit(int mod, int ch, double dt, uint16_t e = 100) {
  EventHit h;
  h.mod = static_cast<uint8_t>(mod);
  h.ch = static_cast<uint8_t>(ch);
  h.dt = dt;
  h.energy = e;
  return h;
}

static GenericEvent event(double t, std::vector<EventHit> hits) {
  GenericEvent ev;
  ev.t = t;
  std::sort(hits.begin(), hits.end(),
            [](const EventHit& a, const EventHit& b) { return a.dt < b.dt; });
  ev.hits = std::move(hits);
  return ev;
}

static void test_planes_four_fold() {
  const ChannelMap m = parse_channel_map(kMap).map;
  // ThGEM 1 at t = 1000 (the reference); arms XL +80, XR +120, YU +150, YD +90.
  GenericEvent ev = event(1000.0, {hit(1, 0, 0, 500), hit(1, 1, 80, 11), hit(1, 2, 120, 12),
                                   hit(1, 3, 150, 13), hit(1, 4, 90, 14), hit(0, 8, 300)});
  const auto p = planes_from_event(ev, m, 1000.0);
  CHECK(p.size() == 1);
  CHECK(p[0].number == 1);
  CHECK(near(p[0].t_trig, 1000.0));
  CHECK(p[0].e_trig == 500);
  CHECK(p[0].n_arms == 4);
  CHECK(near(p[0].x, 40.0));   // XR - XL
  CHECK(near(p[0].y, 60.0));   // YU - YD
  CHECK(p[0].has_xy());
  CHECK(p[0].e[0] == 11 && p[0].e[3] == 14);
}

static void test_planes_partial_and_two() {
  const ChannelMap m = parse_channel_map(kMap).map;
  // ThGEM 1: only XL, XR -> x, no y. ThGEM 2 at +30 ns: all four arms.
  // An arm farther than the window does not count.
  GenericEvent ev =
      event(5000.0, {hit(1, 0, 0), hit(1, 1, 10), hit(1, 2, 30), hit(1, 3, 2000), hit(1, 5, 30),
                     hit(1, 6, 130), hit(1, 7, 100), hit(1, 8, 110), hit(1, 9, 140)});
  const auto p = planes_from_event(ev, m, 1000.0);
  CHECK(p.size() == 2);
  CHECK(p[0].number == 1 && p[1].number == 2);
  CHECK(p[0].n_arms == 2);
  CHECK(near(p[0].x, 20.0));
  CHECK(std::isnan(p[0].y));
  CHECK(!p[0].has_xy());
  CHECK(std::isnan(p[0].dt[2]));  // YU at +2000 is outside ±1000
  CHECK(near(p[1].t_trig, 5030.0));
  CHECK(near(p[1].x, -30.0));  // XR(100) - XL(130), arms relative to ThGEM 2's own trigger
  CHECK(near(p[1].y, -30.0));  // YU(110) - YD(140)
  const auto d = pair_dts(p, 4990.0);
  CHECK(d.size() == 1);
  CHECK(d[0].i == 1 && d[0].j == 2);
  CHECK(near(d[0].dt_i, 10.0) && near(d[0].dt_j, 40.0));
}

static void test_planes_rules() {
  const ChannelMap m = parse_channel_map(kMap).map;
  // Two triggers of ThGEM 1: the earliest is the plane's trigger. Two XL hits:
  // the nearest to that trigger wins; a tie goes to the later hit.
  GenericEvent ev = event(0.0, {hit(1, 0, 0, 1), hit(1, 0, 50, 2), hit(1, 1, -40, 21),
                                hit(1, 1, 40, 22), hit(1, 2, 70, 31), hit(1, 2, 300, 32)});
  auto p = planes_from_event(ev, m, 1000.0);
  CHECK(p.size() == 1);
  CHECK(p[0].e_trig == 1);
  CHECK(p[0].e[0] == 22);  // tie at |40|: the later hit
  CHECK(p[0].e[1] == 31);
  CHECK(near(p[0].x, 30.0));
  // Arms without their trigger make no plane; unknown channels are ignored.
  GenericEvent arms_only = event(0.0, {hit(1, 6, 10), hit(1, 7, 20), hit(3, 3, 0)});
  CHECK(planes_from_event(arms_only, m, 1000.0).empty());
  // The window is inclusive.
  GenericEvent edge = event(0.0, {hit(1, 0, 0), hit(1, 1, 1000), hit(1, 2, -1000)});
  p = planes_from_event(edge, m, 1000.0);
  CHECK(p.size() == 1 && p[0].n_arms == 2 && near(p[0].x, -2000.0));
  CHECK(planes_from_event(edge, m, 999.0)[0].n_arms == 0);
}

static void test_pair_dts_order() {
  // pair_dts keeps i < j whatever the order of the planes.
  Plane a, b, c;
  a.number = 3; a.t_trig = 103;
  b.number = 1; b.t_trig = 101;
  c.number = 2; c.t_trig = 102;
  const auto d = pair_dts({a, b, c}, 100.0);
  CHECK(d.size() == 3);
  bool ok = true;
  for (const auto& x : d) ok = ok && x.i < x.j && near(x.dt_i, x.i) && near(x.dt_j, x.j);
  CHECK(ok);
}

int main() {
  test_map_valid();
  test_map_errors();
  test_map_warnings();
  test_names_and_pairs();
  test_trigger_mismatch();
  test_planes_four_fold();
  test_planes_partial_and_two();
  test_planes_rules();
  test_pair_dts_order();
  std::printf("test_thgem_sink: %d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

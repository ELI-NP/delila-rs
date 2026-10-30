// test_hist_config.cpp — unit tests for the standard root_sink's histogram
// config (hist_config.hpp: parse + errors, value_of / pass_cut for the hit,
// coinc and pos scopes). Split out of ../common/test_sink_core.cpp when
// tools/root_sink/ was divided into common/ standard/ thgem/ (2026-10-30).
//
//   g++ -std=c++17 -O0 -g test_hist_config.cpp -o /tmp/thc && /tmp/thc
//
// License: BSD-3-Clause (same as delila-rs).

#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

#include "../common/sink_core.hpp"
#include "hist_config.hpp"

using namespace rootsink;

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
  return (d < 0 ? -d : d) <= eps;
}

static ScalarHit mod_hit(uint8_t mod, uint8_t ch, double t, uint16_t e = 0) {
  ScalarHit h;
  h.module = mod;
  h.channel = ch;
  h.timestamp_ns = t;
  h.energy = e;
  return h;
}

static ScalarHit hit(uint8_t ch, double t, uint16_t e) {
  ScalarHit h;
  h.channel = ch;
  h.timestamp_ns = t;
  h.energy = e;
  return h;
}

static bool has_err(const ParseResult& pr, const std::string& needle) {
  for (const auto& e : pr.errors)
    if (e.find(needle) != std::string::npos) return true;
  return false;
}

static const HistDef* find_def(const ParseResult& pr, const std::string& name) {
  for (const auto& d : pr.defs)
    if (d.name == name) return &d;
  return nullptr;
}

static void test_hist_channel_cut_module() {
  const char* json = R"JSON({ "histograms": [
    { "name": "a", "type": "TH1D", "x": "energy", "channel": 3,      "bins": 4, "min": 0, "max": 4 },
    { "name": "b", "type": "TH1D", "x": "energy", "channel": "10:3", "bins": 4, "min": 0, "max": 4 },
    { "name": "c", "type": "TH1D", "x": "energy", "channel": "3",    "bins": 4, "min": 0, "max": 4 }
  ]})JSON";
  ParseResult pr = parse_hist_config(json);
  CHECK(pr.errors.empty() && pr.defs.size() == 3);
  if (pr.defs.size() == 3) {
    CHECK(pr.defs[0].cut.kind == CutKind::Channel && pr.defs[0].cut.ivalue == 3 &&
          pr.defs[0].cut.imodule == -1);
    CHECK(pr.defs[1].cut.ivalue == 3 && pr.defs[1].cut.imodule == 10);
    CHECK(pr.defs[2].cut.ivalue == 3 && pr.defs[2].cut.imodule == -1);
    CHECK(pass_cut(pr.defs[0].cut, mod_hit(0, 3, 0)) && pass_cut(pr.defs[0].cut, mod_hit(10, 3, 0)));
    CHECK(pass_cut(pr.defs[1].cut, mod_hit(10, 3, 0)) && !pass_cut(pr.defs[1].cut, mod_hit(0, 3, 0)));
  }
  const char* bad = R"JSON({ "histograms": [
    { "name": "a", "type": "TH1D", "x": "energy", "channel": "x",  "bins": 4, "min": 0, "max": 4 },
    { "name": "b", "type": "TH1D", "x": "energy", "channel": 3.5,  "bins": 4, "min": 0, "max": 4 },
    { "name": "c", "type": "TH1D", "x": "energy", "channel": 300,  "bins": 4, "min": 0, "max": 4 },
    { "name": "d", "type": "TH1D", "x": "energy", "channel": -1,   "bins": 4, "min": 0, "max": 4 },
    { "name": "e", "type": "TH1D", "x": "energy", "channel": true, "bins": 4, "min": 0, "max": 4 },
    { "name": "f", "type": "TH1D", "x": "energy", "channel": "1:2:3", "bins": 4, "min": 0, "max": 4 }
  ]})JSON";
  ParseResult pb = parse_hist_config(bad);
  CHECK(pb.defs.empty());
  int n = 0;
  for (const auto& e : pb.errors)
    if (e.find("cut \"channel\"") != std::string::npos) ++n;
  CHECK(n == 6);  // every one reported
  // The aggregate form the older tests use still means "any module".
  CHECK(pass_cut(Cut{CutKind::Channel, 3, 0, 0}, mod_hit(9, 3, 0)));
}

static void test_hist_config_valid() {
  // The standard histograms.json content: 4 defs, correct axes/scopes.
  const char* json = R"JSON({
    "histograms": [
      { "name": "dt1", "type": "TH1D", "title": "dt one;#Deltat_{1};n",
        "fill": "dt1", "bins": 2000, "min": -1000, "max": 1000 },
      { "name": "dt2", "type": "TH1D",
        "fill": "dt2", "bins": 2000, "min": -1000, "max": 1000 },
      { "name": "dt2_vs_dt1", "type": "TH2D", "x": "dt1", "y": "dt2",
        "xbins": 500, "xmin": -1000, "xmax": 1000,
        "ybins": 500, "ymin": -1000, "ymax": 1000 },
      { "name": "channels", "type": "TH1D", "fill": "channel",
        "bins": 64, "min": 0, "max": 64 }
    ]
  })JSON";
  ParseResult pr = parse_hist_config(json);
  CHECK(pr.errors.empty());
  CHECK(pr.defs.size() == 4);

  const HistDef* d1 = find_def(pr, "dt1");
  CHECK(d1 != nullptr);
  CHECK(!d1->is2d);
  CHECK(d1->x == Var::Dt1);
  CHECK(d1->scope == Scope::Coinc);
  CHECK(d1->xbins == 2000);
  CHECK(near(d1->xmin, -1000.0));
  CHECK(near(d1->xmax, 1000.0));
  CHECK(d1->title == "dt one;#Deltat_{1};n");   // explicit title
  CHECK(d1->cut.kind == CutKind::None);

  const HistDef* d2 = find_def(pr, "dt2");
  CHECK(d2 != nullptr);
  CHECK(d2->title == "dt2");                     // title defaults to name

  const HistDef* d3 = find_def(pr, "dt2_vs_dt1");
  CHECK(d3 != nullptr);
  CHECK(d3->is2d);
  CHECK(d3->x == Var::Dt1);
  CHECK(d3->y == Var::Dt2);
  CHECK(d3->scope == Scope::Coinc);
  CHECK(d3->ybins == 500);

  const HistDef* d4 = find_def(pr, "channels");
  CHECK(d4 != nullptr);
  CHECK(d4->x == Var::Channel);
  CHECK(d4->scope == Scope::Hit);
}

static void test_hist_config_features() {
  // drawopt + a hit-scope energy histogram gated on channel.
  {
    const char* json = R"JSON({ "histograms": [
      { "name": "E_ch3", "type": "TH1D", "x": "energy", "channel": 3,
        "xbins": 100, "xmin": 0, "xmax": 5000, "drawopt": "hist" }
    ]})JSON";
    ParseResult pr = parse_hist_config(json);
    CHECK(pr.errors.empty());
    CHECK(pr.defs.size() == 1);
    const HistDef& d = pr.defs[0];
    CHECK(d.x == Var::Energy);
    CHECK(d.scope == Scope::Hit);
    CHECK(d.cut.kind == CutKind::Channel);
    CHECK(d.cut.ivalue == 3);
    CHECK(d.drawopt == "hist");
  }
  // coinc 2D: E_gamma vs dt1 (both coinc — allowed).
  {
    const char* json = R"JSON({ "histograms": [
      { "name": "E_vs_dt1", "type": "TH2D", "x": "dt1", "y": "gamma_energy",
        "xbins": 200, "xmin": -1000, "xmax": 1000,
        "ybins": 200, "ymin": 0, "ymax": 20000 }
    ]})JSON";
    ParseResult pr = parse_hist_config(json);
    CHECK(pr.errors.empty());
    CHECK(pr.defs.size() == 1);
    CHECK(pr.defs[0].scope == Scope::Coinc);
    CHECK(pr.defs[0].y == Var::GammaEnergy);
  }
  // dt1 gated by gamma_energy_range (the headline use case).
  {
    const char* json = R"JSON({ "histograms": [
      { "name": "dt1_gated", "type": "TH1D", "fill": "dt1",
        "bins": 400, "min": -200, "max": 200,
        "gamma_energy_range": [800, 1200] }
    ]})JSON";
    ParseResult pr = parse_hist_config(json);
    CHECK(pr.errors.empty());
    CHECK(pr.defs.size() == 1);
    CHECK(pr.defs[0].cut.kind == CutKind::GammaEnergyRange);
    CHECK(near(pr.defs[0].cut.lo, 800.0));
    CHECK(near(pr.defs[0].cut.hi, 1200.0));
  }
}

static void test_hist_config_errors() {
  // A file with one distinct error per histogram — all must be collected.
  const char* json = R"JSON({ "histograms": [
    { "name": "a", "type": "TH1D", "fill": "energy", "bins": 10, "min": 0, "max": 10 },
    { "name": "a", "type": "TH1D", "fill": "energy", "bins": 10, "min": 0, "max": 10 },
    { "name": "b", "type": "TH9D", "fill": "energy", "bins": 10, "min": 0, "max": 10 },
    { "name": "c", "type": "TH1D", "fill": "nonsense", "bins": 10, "min": 0, "max": 10 },
    { "name": "d", "type": "TH1D", "fill": "energy", "gamma_energy_range": [1, 2],
      "bins": 10, "min": 0, "max": 10 },
    { "name": "e", "type": "TH1D", "fill": "energy", "bins": 0, "min": 0, "max": 10 },
    { "name": "f", "type": "TH1D", "fill": "energy", "bins": 10, "min": 10, "max": 10 },
    { "name": "g", "type": "TH1D", "fill": "energy", "bins": 10, "min": 0, "max": 10,
      "typo_key": 5 }
  ]})JSON";
  ParseResult pr = parse_hist_config(json);
  CHECK(has_err(pr, "duplicate name"));      // a duplicated
  CHECK(has_err(pr, "unknown type"));        // TH9D
  CHECK(has_err(pr, "unknown variable"));    // nonsense
  CHECK(has_err(pr, "scope mixing"));        // energy (hit) + gamma_energy_range (coinc)
  CHECK(has_err(pr, "xbins must be > 0"));   // bins: 0
  CHECK(has_err(pr, "xmin must be < xmax")); // min==max
  CHECK(has_err(pr, "unknown key"));         // typo_key
  CHECK(pr.errors.size() >= 7);              // collected, not first-only

  // A cut-vs-x scope mix on its own (cut coinc, x hit).
  {
    const char* j = R"JSON({ "histograms": [
      { "name": "mix", "type": "TH1D", "fill": "energy", "bins": 10, "min": 0, "max": 10,
        "thgem1_energy_range": [1, 2] }
    ]})JSON";
    ParseResult p = parse_hist_config(j);
    CHECK(has_err(p, "scope mixing"));
    CHECK(p.defs.empty());
  }

  // Malformed JSON -> exactly one clear error.
  {
    ParseResult p = parse_hist_config("{ not valid json ");
    CHECK(p.errors.size() == 1);
    CHECK(has_err(p, "malformed JSON"));
    CHECK(p.defs.empty());
  }

  // ROOT-unsafe name (contains '/').
  {
    const char* j = R"JSON({ "histograms": [
      { "name": "a/b", "type": "TH1D", "fill": "energy", "bins": 10, "min": 0, "max": 10 }
    ]})JSON";
    ParseResult p = parse_hist_config(j);
    CHECK(has_err(p, "must not contain '/'"));
  }
}

static void test_value_and_cut() {
  // value_of on a hit — always true for hit vars.
  {
    ScalarHit h = hit(3, 12345.0, 777);
    h.module = 2;
    h.energy_short = 55;
    double v;
    CHECK(value_of(Var::Energy, h, v) && near(v, 777.0));
    CHECK(value_of(Var::EnergyShort, h, v) && near(v, 55.0));
    CHECK(value_of(Var::Channel, h, v) && near(v, 3.0));
    CHECK(value_of(Var::Module, h, v) && near(v, 2.0));
    CHECK(!value_of(Var::Dt1, h, v));  // coinc var on a hit -> false
  }
  // pass_cut on a hit.
  {
    ScalarHit h = hit(3, 0.0, 500);
    CHECK(pass_cut(Cut{}, h));                                 // None
    CHECK(pass_cut(Cut{CutKind::Channel, 3, 0, 0}, h));
    CHECK(!pass_cut(Cut{CutKind::Channel, 4, 0, 0}, h));
    CHECK(pass_cut(Cut{CutKind::EnergyRange, 0, 400, 600}, h));
    CHECK(!pass_cut(Cut{CutKind::EnergyRange, 0, 600, 800}, h));
    CHECK(pass_cut(Cut{CutKind::EnergyRange, 0, 500, 500}, h));  // inclusive edge
  }
  // value_of on a coinc — partner-dependent vars are false when the partner is
  // absent; gamma_energy is always valid.
  {
    CoincResult r;
    r.gamma_energy = 1500;
    r.has_dt1 = true;
    r.dt1 = 40.0;
    r.thgem1_energy = 900;
    r.has_dt2 = false;  // no ThGEM2 partner
    double v;
    CHECK(value_of(Var::GammaEnergy, r, v) && near(v, 1500.0));
    CHECK(value_of(Var::Dt1, r, v) && near(v, 40.0));
    CHECK(value_of(Var::Thgem1Energy, r, v) && near(v, 900.0));
    CHECK(!value_of(Var::Dt2, r, v));            // has_dt2 == false
    CHECK(!value_of(Var::Thgem2Energy, r, v));   // has_dt2 == false
  }
  // pass_cut on a coinc — a *_energy_range fails when the partner is missing.
  {
    CoincResult r;
    r.gamma_energy = 1000;
    r.has_dt1 = true;
    r.thgem1_energy = 300;
    r.has_dt2 = false;
    CHECK(pass_cut(Cut{}, r));                                       // None
    CHECK(pass_cut(Cut{CutKind::GammaEnergyRange, 0, 800, 1200}, r));
    CHECK(!pass_cut(Cut{CutKind::GammaEnergyRange, 0, 0, 500}, r));
    CHECK(pass_cut(Cut{CutKind::Thgem1EnergyRange, 0, 200, 400}, r));
    CHECK(!pass_cut(Cut{CutKind::Thgem2EnergyRange, 0, 0, 5000}, r));  // no partner
  }
}

static void test_hist_config_pos() {
  // Valid pos-scope defs: xy1 2D, xy2 2D, and a 1D projection.
  {
    const char* json = R"JSON({ "histograms": [
      { "name": "xy1_pos", "type": "TH2D", "x": "xy1_x", "y": "xy1_y",
        "xbins": 500, "xmin": -250, "xmax": 250,
        "ybins": 500, "ymin": -250, "ymax": 250 },
      { "name": "xy2_pos", "type": "TH2D", "x": "xy2_x", "y": "xy2_y",
        "xbins": 500, "xmin": -250, "xmax": 250,
        "ybins": 500, "ymin": -250, "ymax": 250 },
      { "name": "x1_proj", "type": "TH1D", "fill": "xy1_x",
        "bins": 500, "min": -250, "max": 250 }
    ]})JSON";
    ParseResult pr = parse_hist_config(json);
    CHECK(pr.errors.empty());
    CHECK(pr.defs.size() == 3);
    const HistDef* d1 = find_def(pr, "xy1_pos");
    CHECK(d1 != nullptr);
    CHECK(d1->is2d);
    CHECK(d1->x == Var::Xy1X);
    CHECK(d1->y == Var::Xy1Y);
    CHECK(d1->scope == Scope::Pos1);
    const HistDef* d2 = find_def(pr, "xy2_pos");
    CHECK(d2 != nullptr);
    CHECK(d2->scope == Scope::Pos2);
    const HistDef* d3 = find_def(pr, "x1_proj");
    CHECK(d3 != nullptr);
    CHECK(!d3->is2d);
    CHECK(d3->scope == Scope::Pos1);
  }
  // Scope mixing: xy1 with xy2, pos with hit, pos with a coinc cut — all rejected.
  {
    const char* json = R"JSON({ "histograms": [
      { "name": "mix12", "type": "TH2D", "x": "xy1_x", "y": "xy2_y",
        "xbins": 10, "xmin": -1, "xmax": 1, "ybins": 10, "ymin": -1, "ymax": 1 },
      { "name": "mixhit", "type": "TH2D", "x": "xy1_x", "y": "energy",
        "xbins": 10, "xmin": -1, "xmax": 1, "ybins": 10, "ymin": 0, "ymax": 100 },
      { "name": "mixcut", "type": "TH1D", "fill": "xy1_x",
        "bins": 10, "min": -1, "max": 1, "gamma_energy_range": [0, 100] }
    ]})JSON";
    ParseResult pr = parse_hist_config(json);
    CHECK(has_err(pr, "scope mixing"));
    CHECK(pr.defs.empty());
  }
}

static void test_pos_value_and_cut() {
  // x needs BOTH X arms; y needs BOTH Y arms; the two are independent.
  {
    PosResult r;
    r.has_xl = true;  r.dt_xl = 10.0;   // t_XL = trig + 10
    r.has_xr = true;  r.dt_xr = 30.0;   // t_XR = trig + 30
    r.has_yu = true;  r.dt_yu = -10.0;
    r.has_yd = true;  r.dt_yd = 5.0;
    double v;
    CHECK(value_of(Var::Xy1X, r, v) && near(v, 20.0));   // t_XR - t_XL
    CHECK(value_of(Var::Xy1Y, r, v) && near(v, -15.0));  // t_YU - t_YD
    // Xy2X/Xy2Y compute the same thing — instance routing is the fill site's job.
    CHECK(value_of(Var::Xy2X, r, v) && near(v, 20.0));
    CHECK(value_of(Var::Xy2Y, r, v) && near(v, -15.0));
  }
  // One X arm missing -> no x, but y still fills.
  {
    PosResult r;
    r.has_xl = true;  r.dt_xl = 10.0;
    r.has_xr = false;
    r.has_yu = true;  r.dt_yu = 4.0;
    r.has_yd = true;  r.dt_yd = 1.0;
    double v;
    CHECK(!value_of(Var::Xy1X, r, v));
    CHECK(value_of(Var::Xy1Y, r, v) && near(v, 3.0));
  }
  // A hit/coinc var against a PosResult is false (unreachable after validation).
  {
    PosResult r;
    double v;
    CHECK(!value_of(Var::Energy, r, v));
    CHECK(!value_of(Var::Dt1, r, v));
  }
  // pass_cut: only None passes (no pos-scope cuts exist yet).
  {
    PosResult r;
    CHECK(pass_cut(Cut{}, r));
    CHECK(!pass_cut(Cut{CutKind::Channel, 1, 0, 0}, r));
    CHECK(!pass_cut(Cut{CutKind::GammaEnergyRange, 0, 0, 100}, r));
  }
}

int main() {
  test_hist_channel_cut_module();
  test_hist_config_valid();
  test_hist_config_features();
  test_hist_config_errors();
  test_value_and_cut();
  test_hist_config_pos();
  test_pos_value_and_cut();

  std::printf("\n%d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

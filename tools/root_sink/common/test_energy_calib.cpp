// test_energy_calib.cpp — unit tests for energy_calib.hpp. No ROOT, no ZMQ:
//
//   g++ -std=c++17 -O0 -g test_energy_calib.cpp -o /tmp/tec && /tmp/tec
//
// Run from this directory: the fixtures are the ones the Rust reader is tested on
// (src/monitor/energy_calib.rs), so the two must read them the same way.
//
// License: BSD-3-Clause (same as delila-rs).

#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#include "energy_calib.hpp"

using namespace ecal;

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

static std::string fixture(const std::string& name) {
  std::ifstream in("fixtures/energy_calib/" + name, std::ios::binary);
  if (!in) {
    std::printf("FAIL cannot open fixture %s (run from tools/root_sink/common)\n", name.c_str());
    ++g_fail;
    return std::string();
  }
  std::ostringstream s;
  s << in.rdbuf();
  return s.str();
}

static bool line_is(const CalibTable& t, unsigned m, unsigned c, double slope, double offset) {
  auto it = t.find(calib_key(m, c));
  return it != t.end() && it->second.kev_per_adc == slope && it->second.offset_kev == offset;
}

static bool starts(const std::string& s, const std::string& prefix) {
  return s.rfind(prefix, 0) == 0;
}

static bool has(const std::string& s, const std::string& needle) {
  return s.find(needle) != std::string::npos;
}

static void test_good() {
  CalibParseResult r = parse_energy_calib(fixture("good.txt"), "f");
  CHECK(r.errors.empty());
  CHECK(r.table.size() == 3);
  CHECK(line_is(r.table, 0, 1, 0.2218, 3.1));
  CHECK(line_is(r.table, 5, 0, 0.3012, -12.4));
  CHECK(line_is(r.table, 5, 1, 0.25, 0.0));
  CHECK(r.table.count(calib_key(1, 1)) == 0);
}

static void test_crlf() {
  CalibParseResult r = parse_energy_calib(fixture("good_crlf.txt"), "f");
  CHECK(r.errors.empty());
  CHECK(r.table.size() == 2);
  CHECK(line_is(r.table, 0, 1, 0.5, -10.0));
}

static void test_comments_only() {
  CalibParseResult r = parse_energy_calib(fixture("comments_only.txt"), "f");
  CHECK(r.errors.empty());
  CHECK(r.table.empty());
  r = parse_energy_calib("", "f");
  CHECK(r.errors.empty());
  CHECK(r.table.empty());
}

static void test_bad_columns() {
  CalibParseResult r = parse_energy_calib(fixture("bad_columns.txt"), "f");
  CHECK(r.errors.size() == 3);
  if (r.errors.size() != 3) return;
  CHECK(starts(r.errors[0], "f:1:"));
  CHECK(starts(r.errors[1], "f:2:"));
  CHECK(starts(r.errors[2], "f:3:"));
}

static void test_bad_values() {
  CalibParseResult r = parse_energy_calib(fixture("bad_values.txt"), "f");
  CHECK(r.errors.size() == 12);
  if (r.errors.size() != 12) return;
  CHECK(has(r.errors[0], "kev_per_adc '0'"));
  CHECK(has(r.errors[1], "kev_per_adc '-0.5'"));
  CHECK(has(r.errors[5], "offset_kev 'inf'"));
  CHECK(has(r.errors[7], "module '256'"));
  CHECK(has(r.errors[10], "channel '256'"));
  CHECK(has(r.errors[11], "kev_per_adc '0x10'"));
}

static void test_duplicate() {
  CalibParseResult r = parse_energy_calib(fixture("bad_duplicate.txt"), "f");
  CHECK(r.errors.size() == 1);
  if (r.errors.size() != 1) return;
  CHECK(starts(r.errors[0], "f:4:"));
  CHECK(has(r.errors[0], "line 1"));
}

static void test_to_kev() {
  LinearCalib c{0.5, -10.0};
  CHECK(c.to_kev(100.0) == 40.0);
}

static void test_axis_limits() {
  CalibTable t;
  t[calib_key(1, 0)] = LinearCalib{0.5, -10.0};
  t[calib_key(1, 1)] = LinearCalib{0.25, 5.0};
  const std::vector<uint16_t> both = {calib_key(1, 0), calib_key(1, 1), calib_key(1, 2)};

  AxisLimits a = axis_limits_kev(t, both, 32768.0);
  CHECK(a.valid);
  CHECK(a.hi == 0.5 * 32768.0 - 10.0);
  CHECK(a.lo == -10.0);

  a = axis_limits_kev(t, both, 65536.0);
  CHECK(a.hi == 0.5 * 65536.0 - 10.0);

  // Only positive offsets: the axis still starts at 0.
  a = axis_limits_kev(t, {calib_key(1, 1)}, 32768.0);
  CHECK(a.valid);
  CHECK(a.hi == 0.25 * 32768.0 + 5.0);
  CHECK(a.lo == 0.0);

  // No channel here has coefficients.
  a = axis_limits_kev(t, {calib_key(2, 0)}, 32768.0);
  CHECK(!a.valid);
  a = axis_limits_kev(CalibTable{}, both, 32768.0);
  CHECK(!a.valid);
}

static void test_reload_policy() {
  ActiveCalib a;
  std::vector<std::string> errors;
  bool changed = false;

  // First read fails: failed, no table.
  CHECK(!reload_calib(a, nullptr, "no such file", "p", "t0", errors, changed));
  CHECK(a.failed);
  CHECK(!a.stale);
  CHECK(!a.loaded);
  CHECK(has(a.error, "no such file"));
  CHECK(calib_status_field(a, true) == "ERROR");

  // Good file A: adopted.
  std::string good = "0 1 0.5 -10\n";
  CHECK(reload_calib(a, &good, "", "p", "t1", errors, changed));
  CHECK(changed);
  CHECK(!a.failed);
  CHECK(!a.stale);
  CHECK(a.loaded_at == "t1");
  CHECK(line_is(a.table, 0, 1, 0.5, -10.0));
  CHECK(calib_status_field(a, true) == "1");

  // The same numbers again (every run start): not a change.
  std::string same = "# a new comment\n0   1   0.5   -10\n";
  CHECK(reload_calib(a, &same, "", "p", "t2", errors, changed));
  CHECK(!changed);

  // Invalid: previous table kept, stale.
  std::string bad = "0 1 -1 0\n";
  CHECK(!reload_calib(a, &bad, "", "p", "t3", errors, changed));
  CHECK(!changed);
  CHECK(a.stale);
  CHECK(!a.failed);
  CHECK(errors.size() == 1);
  CHECK(line_is(a.table, 0, 1, 0.5, -10.0));
  CHECK(a.loaded_at == "t2");
  CHECK(calib_status_field(a, true) == "1 STALE");

  // Unreadable: previous table kept.
  CHECK(!reload_calib(a, nullptr, "gone", "p", "t4", errors, changed));
  CHECK(a.stale);
  CHECK(line_is(a.table, 0, 1, 0.5, -10.0));

  // Good file B: adopted, stale cleared.
  std::string other = "0 2 2 1\n";
  CHECK(reload_calib(a, &other, "", "p", "t5", errors, changed));
  CHECK(changed);
  CHECK(!a.stale);
  CHECK(a.error.empty());
  CHECK(a.table.count(calib_key(0, 1)) == 0);
  CHECK(line_is(a.table, 0, 2, 2.0, 1.0));

  CHECK(calib_status_field(ActiveCalib{}, false) == "off");
}

int main() {
  test_good();
  test_crlf();
  test_comments_only();
  test_bad_columns();
  test_bad_values();
  test_duplicate();
  test_to_kev();
  test_axis_limits();
  test_reload_policy();
  std::printf("%d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

// test_sink_core.cpp — ROOT/ZMQ-free unit tests for the root_sink pure logic.
//
// Covers sink_core.hpp (envelope parse, batch decode, coincidence matcher with
// energies, run state, HTTP helpers), hist_config.hpp (config parse + errors,
// value_of / pass_cut for both scopes), time_offsets.hpp (TODO 70) and
// event_def.hpp (TODO 71). Builds and runs on any box:
//
//   g++ -std=c++17 -O0 -g test_sink_core.cpp -o /tmp/ts && /tmp/ts
//
// The MessagePack bytes are hand-crafted here to match the wire format documented
// at the top of sink_core.hpp (rmp-serde fixmap(1){variant:payload}).
//
// License: BSD-3-Clause (same as delila-rs).

#include <array>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <memory>
#include <sstream>
#include <string>
#include <vector>

#include "event_def.hpp"
#include "hist_config.hpp"
#include "sink_core.hpp"
#include "time_offsets.hpp"

using namespace rootsink;

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

static bool near(double a, double b, double eps = 1e-9) {
  double d = a - b;
  return (d < 0 ? -d : d) <= eps;
}

// ---------------------------------------------------------------------------
// MessagePack byte builders
// ---------------------------------------------------------------------------
struct MB {
  std::vector<uint8_t> b;
  void u8(uint8_t x) { b.push_back(x); }
  void fixmap1() { u8(0x81); }
  void fixstr(const std::string& s) {
    u8(0xA0 | static_cast<uint8_t>(s.size()));  // len <= 31 here
    for (char c : s) u8(static_cast<uint8_t>(c));
  }
  void array(uint32_t n) {
    if (n <= 15) {
      u8(0x90 | static_cast<uint8_t>(n));
    } else {
      u8(0xDC);
      u8(static_cast<uint8_t>((n >> 8) & 0xFF));
      u8(static_cast<uint8_t>(n & 0xFF));
    }
  }
  void posint(uint8_t x) { u8(x); }  // 0..127 positive fixint
  void uint16(uint16_t x) {
    u8(0xCD);
    u8(static_cast<uint8_t>((x >> 8) & 0xFF));
    u8(static_cast<uint8_t>(x & 0xFF));
  }
  void uint64(uint64_t x) {
    u8(0xCF);
    for (int i = 7; i >= 0; --i) u8(static_cast<uint8_t>((x >> (i * 8)) & 0xFF));
  }
  void f64(double x) {
    uint64_t r;
    std::memcpy(&r, &x, 8);
    u8(0xCB);
    for (int i = 7; i >= 0; --i) u8(static_cast<uint8_t>((r >> (i * 8)) & 0xFF));
  }
  // Append an EventData array [module, channel, energy, energy_short, ts, extra...]
  void event(uint8_t mod, uint8_t ch, uint16_t e, uint16_t es, double ts,
             bool with_extra = false) {
    array(with_extra ? 7 : 5);
    posint(mod);
    posint(ch);
    uint16(e);
    uint16(es);
    f64(ts);
    if (with_extra) {
      uint64(0);       // flags
      array(4);        // user_info [u64;4]
      for (int i = 0; i < 4; ++i) uint64(0);
    }
  }
};

// Build a full Data envelope carrying one batch of the given events.
static std::vector<uint8_t> make_data(uint32_t source_id,
                                      const std::vector<std::array<double, 5>>& evs,
                                      bool with_extra = false) {
  MB m;
  m.fixmap1();
  m.fixstr("Data");
  m.array(4);                          // EventDataBatch [sid, seq, ts, events]
  m.posint(static_cast<uint8_t>(source_id));
  m.uint64(0);                         // sequence_number
  m.uint64(0);                         // timestamp
  m.array(static_cast<uint32_t>(evs.size()));
  for (const auto& e : evs)
    m.event(static_cast<uint8_t>(e[0]), static_cast<uint8_t>(e[1]),
            static_cast<uint16_t>(e[2]), static_cast<uint16_t>(e[3]), e[4],
            with_extra);
  return m.b;
}

static std::vector<uint8_t> make_eos(uint32_t sid, uint32_t run) {
  MB m;
  m.fixmap1();
  m.fixstr("EndOfStream");
  m.array(2);
  m.posint(static_cast<uint8_t>(sid));
  m.posint(static_cast<uint8_t>(run));
  return m.b;
}

// ---------------------------------------------------------------------------
// Envelope tests
// ---------------------------------------------------------------------------
static void test_envelope() {
  // Data
  {
    auto d = make_data(7, {{0, 1, 1000, 200, 12345.0}});
    Envelope e = parse_envelope(d.data(), d.size());
    CHECK(e.kind == MsgKind::Data);
    CHECK(e.variant == "Data");
    CHECK(e.payload != nullptr);
    CHECK(e.payload_size > 0);
  }
  // EndOfStream carries source_id + run_number
  {
    auto d = make_eos(3, 42);
    Envelope e = parse_envelope(d.data(), d.size());
    CHECK(e.kind == MsgKind::EndOfStream);
    CHECK(e.source_id == 3);
    CHECK(e.run_number == 42);
  }
  // Heartbeat
  {
    MB m;
    m.fixmap1();
    m.fixstr("Heartbeat");
    m.posint(0);  // arbitrary payload
    Envelope e = parse_envelope(m.b.data(), m.b.size());
    CHECK(e.kind == MsgKind::Heartbeat);
  }
  // Unknown variant
  {
    MB m;
    m.fixmap1();
    m.fixstr("Foo");
    m.posint(0);
    Envelope e = parse_envelope(m.b.data(), m.b.size());
    CHECK(e.kind == MsgKind::Unknown);
    CHECK(e.variant == "Foo");
  }
  // Malformed: empty buffer -> "<malformed>"
  {
    Envelope e = parse_envelope(nullptr, 0);
    CHECK(e.kind == MsgKind::Unknown);
    CHECK(e.variant == "<malformed>");
  }
  // Not a fixmap(1): a fixmap(2) header -> "<not-fixmap1>"
  {
    uint8_t buf[1] = {0x82};
    Envelope e = parse_envelope(buf, 1);
    CHECK(e.kind == MsgKind::Unknown);
    CHECK(e.variant == "<not-fixmap1>");
  }
}

// ---------------------------------------------------------------------------
// Batch decode tests
// ---------------------------------------------------------------------------
static void test_batch_decode() {
  // Field order preserved for one event.
  {
    auto d = make_data(9, {{2, 5, 4095, 321, 987654.5}});
    Envelope e = parse_envelope(d.data(), d.size());
    CHECK(e.kind == MsgKind::Data);
    uint32_t sid = 0;
    std::vector<ScalarHit> out;
    long n = decode_batch_into(e.payload, e.payload_size, sid, out);
    CHECK(n == 1);
    CHECK(sid == 9);
    CHECK(out.size() == 1);
    CHECK(out[0].module == 2);
    CHECK(out[0].channel == 5);
    CHECK(out[0].energy == 4095);
    CHECK(out[0].energy_short == 321);
    CHECK(near(out[0].timestamp_ns, 987654.5));
  }
  // Trailing fields (flags, user_info) are skipped; decode still succeeds.
  {
    auto d = make_data(1, {{3, 7, 100, 50, 42.0}}, /*with_extra=*/true);
    Envelope e = parse_envelope(d.data(), d.size());
    uint32_t sid = 0;
    std::vector<ScalarHit> out;
    long n = decode_batch_into(e.payload, e.payload_size, sid, out);
    CHECK(n == 1);
    CHECK(out.size() == 1);
    CHECK(out[0].module == 3);
    CHECK(out[0].channel == 7);
    CHECK(out[0].energy == 100);
    CHECK(near(out[0].timestamp_ns, 42.0));
  }
  // Multiple events + source id propagation.
  {
    auto d = make_data(4, {{0, 0, 10, 1, 1.0}, {0, 1, 20, 2, 2.0}, {0, 2, 30, 3, 3.0}});
    Envelope e = parse_envelope(d.data(), d.size());
    uint32_t sid = 0;
    std::vector<ScalarHit> out;
    long n = decode_batch_into(e.payload, e.payload_size, sid, out);
    CHECK(n == 3);
    CHECK(sid == 4);
    CHECK(out.size() == 3);
    CHECK(out[2].energy == 30);
  }
  // Malformed batch -> -1 (array header claims 4 fields but bytes are truncated).
  {
    uint8_t buf[1] = {0x94};  // array(4) header, nothing follows
    uint32_t sid = 0;
    std::vector<ScalarHit> out;
    long n = decode_batch_into(buf, 1, sid, out);
    CHECK(n == -1);
  }
}

// ---------------------------------------------------------------------------
// keep_module (--module): one board's hits only
// ---------------------------------------------------------------------------
static ScalarHit mod_hit(uint8_t mod, uint8_t ch, double t, uint16_t e = 0) {
  ScalarHit h;
  h.module = mod;
  h.channel = ch;
  h.timestamp_ns = t;
  h.energy = e;
  return h;
}

static void test_keep_module() {
  const std::vector<ScalarHit> v = {mod_hit(0, 1, 10.0), mod_hit(1, 1, 11.0),
                                    mod_hit(0, 6, 12.0), mod_hit(2, 0, 13.0)};
  auto all = v;
  keep_module(all, -1);
  CHECK(all.size() == 4);  // -1 = every module (the default, side3 behaviour)
  auto m0 = v;
  keep_module(m0, 0);
  CHECK(m0.size() == 2 && m0[0].module == 0 && m0[1].channel == 6);
  CHECK(near(m0[0].timestamp_ns, 10.0) && near(m0[1].timestamp_ns, 12.0));  // order kept
  auto m3 = v;
  keep_module(m3, 3);
  CHECK(m3.empty());
}

static void test_foreign_module_hits_do_not_reach_the_matcher() {
  // The matchers key on CHANNEL alone. A second board breaks them two ways:
  // (a) its channel 1 is taken for ThGEM1; (b) its independent clock drives
  // the watermark, so a gamma ripens before its real partner arrives.
  CoincidenceMatcher::Config cfg;
  cfg.gamma_ch = 0;
  cfg.thgem1_ch = 1;
  cfg.window_ns = 100.0;
  cfg.margin_ns = 1000.0;
  auto run = [&](int module) {
    CoincidenceMatcher m(cfg);
    std::vector<CoincResult> out;
    auto keep = [&](const CoincResult& r) { out.push_back(r); };
    std::vector<ScalarHit> batch1 = {mod_hit(0, 0, 1000.0, 7),      // gamma
                                     mod_hit(1, 1, 1005.0, 99),     // board 1 ch1
                                     mod_hit(1, 5, 1.0e9, 0)};      // board 1, clock far ahead
    keep_module(batch1, module);
    for (const ScalarHit& h : batch1) m.push(h, keep);
    std::vector<ScalarHit> batch2 = {mod_hit(0, 1, 1060.0, 42)};  // the real ThGEM1 hit
    keep_module(batch2, module);
    for (const ScalarHit& h : batch2) m.push(h, keep);
    m.flush(keep);
    return out;
  };
  auto unfiltered = run(-1);
  CHECK(unfiltered.size() == 1);
  CHECK(unfiltered[0].has_dt1 && unfiltered[0].thgem1_energy == 99);  // wrong partner
  auto filtered = run(0);
  CHECK(filtered.size() == 1);
  CHECK(filtered[0].has_dt1 && filtered[0].thgem1_energy == 42 && near(filtered[0].dt1, 60.0));
}

// ---------------------------------------------------------------------------
// Time offsets (time_offsets.hpp, TODO 70): timeSettings.json, aligned = raw - offset
// ---------------------------------------------------------------------------
static bool any_contains(const std::vector<std::string>& v, const char* needle) {
  for (const std::string& s : v)
    if (s.find(needle) != std::string::npos) return true;
  return false;
}

static OffsetTable table_of(std::initializer_list<std::pair<uint16_t, double>> kv) {
  OffsetTable t;
  t.format = "tree";
  for (const auto& p : kv) t.abs_ns[p.first] = p.second;
  return t;
}

static void test_toff_sign_convention() {
  // Positive offset = the channel arrives LATE, so it is subtracted (the Rust EB
  // convention, src/event_builder/hit.rs: aligned_ts = raw_ts - offset_ns).
  OffsetTable t = table_of({{mc_key(0, 1), 0.0}, {mc_key(0, 2), 30.0}, {mc_key(0, 3), -5.0}});
  std::vector<ScalarHit> h = {mod_hit(0, 1, 1000.0), mod_hit(0, 2, 1030.0), mod_hit(0, 3, 995.0)};
  std::set<uint16_t> reported;
  int missing = 0;
  apply_time_offsets(h, t, reported, [&](uint16_t) { ++missing; });
  CHECK(missing == 0);
  CHECK(near(h[0].timestamp_ns, 1000.0) && near(h[1].timestamp_ns, 1000.0) &&
        near(h[2].timestamp_ns, 1000.0));
  CHECK(mc_key(1, 2) == 0x0102 && mc_name(mc_key(3, 15)) == "m3c15");
}

static void test_toff_tree_resolves() {
  // The Rust single_root_resolves fixture, listed child-first on purpose:
  // resolution must not depend on entry order.
  const char* j = R"({"version":"1.0","entries":[
      {"module":5,"channel":0,"ref":[0,0],"offset_ns":12.3},
      {"module":0,"channel":0,"ref":[9,0],"offset_ns":46.75},
      {"module":9,"channel":1,"ref":[9,0],"offset_ns":0.05},
      {"module":9,"channel":0,"ref":null,"offset_ns":0.0}]})";
  OffsetParseResult r = parse_time_offsets(j);
  CHECK(r.errors.empty());
  CHECK(r.table.format == "tree");
  CHECK(r.table.abs_ns.size() == 4);
  CHECK(near(r.table.abs_ns[mc_key(5, 0)], 59.05, 1e-9));
  CHECK(near(r.table.abs_ns[mc_key(0, 0)], 46.75));
  CHECK(near(r.table.abs_ns[mc_key(9, 1)], 0.05));
  CHECK(near(r.table.abs_ns[mc_key(9, 0)], 0.0));
  CHECK(r.table.roots == 1 && r.table.max_depth == 2);
  CHECK(r.table.warnings.empty());
  CHECK(near(r.table.spread_ns(), 59.05, 1e-9));
}

static void test_toff_root_offset_applied() {
  // The root's own offset counts too (Rust root_offset_is_applied).
  OffsetParseResult r = parse_time_offsets(
      R"({"version":"1.0","entries":[{"module":0,"channel":0,"ref":null,"offset_ns":100},
                                     {"module":0,"channel":1,"ref":[0,0],"offset_ns":5}]})");
  CHECK(r.errors.empty());
  CHECK(near(r.table.abs_ns[mc_key(0, 0)], 100.0) && near(r.table.abs_ns[mc_key(0, 1)], 105.0));
}

static void test_toff_tree_errors() {
  auto errs = [](const std::string& entries) {
    return parse_time_offsets(R"({"version":"1.0","entries":[)" + entries + "]}").errors;
  };
  const std::string root = R"({"module":0,"channel":0,"ref":null,"offset_ns":0})";
  CHECK(any_contains(errs(root + "," + root), "duplicate"));
  CHECK(any_contains(errs(root + R"(,{"module":0,"channel":1,"ref":[9,9],"offset_ns":1})"),
                     "missing parent"));
  CHECK(any_contains(errs(root + R"(,{"module":0,"channel":1,"ref":[0,2],"offset_ns":1},
                                    {"module":0,"channel":2,"ref":[0,1],"offset_ns":1})"),
                     "cycle"));
  CHECK(any_contains(errs(R"({"module":0,"channel":1,"ref":[0,1],"offset_ns":1})"), "cycle"));
  CHECK(!errs(R"({"module":256,"channel":0,"ref":null,"offset_ns":0})").empty());
  CHECK(!errs(R"({"module":0,"channel":1.5,"ref":null,"offset_ns":0})").empty());
  CHECK(!errs(R"({"module":0,"channel":1,"ref":[1,2,3],"offset_ns":0})").empty());
  CHECK(!errs(R"({"module":0,"channel":1,"ref":null,"offset_ns":"8.7"})").empty());
  CHECK(any_contains(errs(R"({"module":0,"channel":1,"offset_ns":0})"), "ref"));
  CHECK(any_contains(errs(R"({"module":0,"channel":1,"Ref":null,"ref":null,"offset_ns":0})"),
                     "Ref"));
  // Top level: unknown key, missing version, malformed JSON.
  CHECK(any_contains(parse_time_offsets(R"({"version":"1.0","entries":[],"entires":[]})").errors,
                     "entires"));
  CHECK(any_contains(parse_time_offsets(R"({"entries":[]})").errors, "version"));
  CHECK(parse_time_offsets("{").errors.size() == 1);
  CHECK(any_contains(parse_time_offsets("").errors, "empty"));
  CHECK(any_contains(parse_time_offsets(" \n\t").errors, "empty"));
  CHECK(!parse_time_offsets("[1,2]").errors.empty());
  // Collect-all: two bad entries -> two errors.
  CHECK(errs(R"({"module":300,"channel":0,"ref":null,"offset_ns":0},
                {"module":0,"channel":0,"ref":null,"offset_ns":"x"})").size() == 2);
  // Detection is explicit: both or neither form key is an error, never a guess.
  CHECK(any_contains(parse_time_offsets(R"({"version":"1.0","entries":[],"offsets":{}})").errors,
                     "ambiguous"));
  CHECK(!parse_time_offsets(R"({"version":"1.0"})").errors.empty());
  // _comment and $schema are accepted (repo convention; Rust serde ignores them).
  OffsetParseResult ok = parse_time_offsets(
      R"({"$schema":"x","_comment":["a","b"],"version":"1.0","entries":[)" + root + "]}");
  CHECK(ok.errors.empty());
  CHECK(!parse_time_offsets(R"({"_comment":5,"version":"1.0","entries":[]})").errors.empty());
}

static void test_toff_tree_warnings() {
  OffsetParseResult two = parse_time_offsets(
      R"({"version":"1.0","entries":[{"module":0,"channel":0,"ref":null,"offset_ns":0},
                                     {"module":5,"channel":0,"ref":null,"offset_ns":0}]})");
  CHECK(two.errors.empty() && two.table.roots == 2 && any_contains(two.table.warnings, "disconnected"));
  std::string chain = R"({"module":0,"channel":0,"ref":null,"offset_ns":0})";
  for (int c = 1; c <= 7; ++c)
    chain += R"(,{"module":0,"channel":)" + std::to_string(c) + R"(,"ref":[0,)" +
             std::to_string(c - 1) + R"(],"offset_ns":1})";
  OffsetParseResult deep = parse_time_offsets(R"({"version":"1.0","entries":[)" + chain + "]}");
  CHECK(deep.errors.empty() && deep.table.max_depth == 7 && any_contains(deep.table.warnings, "depth"));
  CHECK(near(deep.table.abs_ns[mc_key(0, 7)], 7.0));
  OffsetParseResult v2 = parse_time_offsets(R"({"version":"2.0","entries":[]})");
  CHECK(v2.errors.empty() && any_contains(v2.table.warnings, "version"));
}

static void test_toff_legacy() {
  // `event_builder time-calib` output (src/event_builder/config.rs): already absolute.
  OffsetParseResult r = parse_time_offsets(
      R"({"ref_module":0,"ref_channel":1,"offsets":{"00_02":8.7,"0_3":-8.7,"01_02":4}})");
  CHECK(r.errors.empty());
  CHECK(r.table.format == "legacy" && r.table.roots == 1 && r.table.max_depth == 0);
  CHECK(near(r.table.abs_ns[mc_key(0, 2)], 8.7) && near(r.table.abs_ns[mc_key(0, 3)], -8.7) &&
        near(r.table.abs_ns[mc_key(1, 2)], 4.0));
  // Stricter than Rust's warn-and-ignore: a skipped key would be a silent 0 offset.
  CHECK(!parse_time_offsets(R"({"ref_module":0,"ref_channel":0,"offsets":{"O_3":1}})").errors.empty());
  CHECK(!parse_time_offsets(R"({"ref_module":0,"ref_channel":0,"offsets":{"0-3":1}})").errors.empty());
  CHECK(!parse_time_offsets(R"({"ref_module":0,"ref_channel":0,"offsets":{"0_3_1":1}})").errors.empty());
  CHECK(!parse_time_offsets(R"({"ref_module":0,"ref_channel":0,"offsets":{"0_256":1}})").errors.empty());
  CHECK(any_contains(parse_time_offsets(
                         R"({"ref_module":0,"ref_channel":0,"offsets":{"00_02":1,"0_2":2}})").errors,
                     "duplicate"));
  CHECK(!parse_time_offsets(R"({"ref_module":0,"offsets":{}})").errors.empty());
}

static void test_toff_module_specific() {
  OffsetTable t = table_of({{mc_key(0, 2), 10.0}, {mc_key(1, 2), -20.0}});
  std::vector<ScalarHit> h = {mod_hit(0, 2, 1000.0), mod_hit(1, 2, 1000.0), mod_hit(2, 2, 1000.0)};
  std::set<uint16_t> reported;
  std::vector<uint16_t> missing;
  apply_time_offsets(h, t, reported, [&](uint16_t k) { missing.push_back(k); });
  CHECK(near(h[0].timestamp_ns, 990.0) && near(h[1].timestamp_ns, 1020.0));
  CHECK(near(h[2].timestamp_ns, 1000.0));  // no entry: raw, and reported
  CHECK(missing.size() == 1 && missing[0] == mc_key(2, 2));
}

static void test_toff_missing_once_per_run() {
  OffsetTable t = table_of({{mc_key(0, 1), 0.0}});
  std::set<uint16_t> reported;
  std::vector<uint16_t> missing;
  auto on_missing = [&](uint16_t k) { missing.push_back(k); };
  std::vector<ScalarHit> b1 = {mod_hit(0, 1, 1.0), mod_hit(0, 7, 2.0), mod_hit(0, 7, 3.0),
                               mod_hit(1, 7, 4.0)};
  apply_time_offsets(b1, t, reported, on_missing);
  CHECK(missing.size() == 2 && missing[0] == mc_key(0, 7) && missing[1] == mc_key(1, 7));
  CHECK(near(b1[1].timestamp_ns, 2.0));  // unmapped stays raw
  std::vector<ScalarHit> b2 = {mod_hit(0, 7, 5.0)};
  apply_time_offsets(b2, t, reported, on_missing);
  CHECK(missing.size() == 2);  // same run: not again
  reported.clear();            // RunOpen
  std::vector<ScalarHit> b3 = {mod_hit(0, 7, 6.0)};
  apply_time_offsets(b3, t, reported, on_missing);
  CHECK(missing.size() == 3);
}

static void test_toff_reload_policy() {
  const std::string A = R"({"version":"1.0","entries":[{"module":0,"channel":1,"ref":null,"offset_ns":0},
                                                        {"module":0,"channel":2,"ref":[0,1],"offset_ns":8}]})";
  const std::string B = R"({"version":"1.0","entries":[{"module":0,"channel":1,"ref":null,"offset_ns":0},
                                                        {"module":0,"channel":2,"ref":[0,1],"offset_ns":9}]})";
  const std::string bad = R"({"version":"1.0","entries":[{"module":0}]})";
  const std::string huge = R"({"version":"1.0","entries":[{"module":0,"channel":1,"ref":null,"offset_ns":0},
                                                           {"module":0,"channel":2,"ref":[0,1],"offset_ns":2e9}]})";
  ActiveOffsets a;
  std::vector<std::string> errors;
  CHECK(reload_offsets(a, &A, "", "/x/ts.json", 1e9, "t0", errors));
  CHECK(errors.empty() && a.loaded && !a.stale && near(a.table.abs_ns[mc_key(0, 2)], 8.0));
  CHECK(a.source_text == A && a.source_path == "/x/ts.json" && a.loaded_at == "t0");
  // Invalid file at a run boundary: keep the previous table AND text, go stale.
  CHECK(!reload_offsets(a, &bad, "", "/x/ts.json", 1e9, "t1", errors));
  CHECK(!errors.empty() && a.stale && near(a.table.abs_ns[mc_key(0, 2)], 8.0) &&
        a.source_text == A && a.loaded_at == "t0" && !a.stale_error.empty());
  // Unreadable file.
  CHECK(!reload_offsets(a, nullptr, "No such file or directory", "/x/ts.json", 1e9, "t2", errors));
  CHECK(a.stale && any_contains(errors, "No such file"));
  // Recovery clears stale.
  CHECK(reload_offsets(a, &B, "", "/x/ts.json", 1e9, "t3", errors));
  CHECK(!a.stale && a.stale_error.empty() && near(a.table.abs_ns[mc_key(0, 2)], 9.0));
  // Spread >= max (the Sorter's safe horizon) would push hits behind emitted cores.
  CHECK(!reload_offsets(a, &huge, "", "/x/ts.json", 1e9, "t4", errors));
  CHECK(any_contains(errors, "spread") && near(a.table.abs_ns[mc_key(0, 2)], 9.0));
  // First load failing leaves nothing loaded (the caller exits at startup).
  ActiveOffsets fresh;
  CHECK(!reload_offsets(fresh, &bad, "", "p", 1e9, "t", errors) && !fresh.loaded);
}

static void test_toff_provenance_json() {
  const std::string src = "{\"version\":\"1.0\",\"_comment\":\"tab\\t \\\"quoted\\\" back\\\\slash\",\n"
                          "\"entries\":[{\"module\":0,\"channel\":2,\"ref\":null,\"offset_ns\":8.7}]}";
  ActiveOffsets a;
  std::vector<std::string> errors;
  CHECK(reload_offsets(a, &src, "", "/p/ts.json", 1e9, "2026-10-07T00:00:00Z", errors));
  using JV = tdelila::json::Value;
  JV v = tdelila::json::Parser(offsets_provenance_json(a)).parse();
  CHECK(v.find("applied") && v.find("applied")->b);
  CHECK(v.find("format") && v.find("format")->str == "tree");
  CHECK(v.find("status") && v.find("status")->str == "ok");
  CHECK(v.find("source_text") && v.find("source_text")->str == src);  // round-trips exactly
  const JV* res = v.find("resolved_abs_ns");
  CHECK(res && res->t == JV::T::Arr && res->arr.size() == 1);
  CHECK(res && res->arr[0].find("offset_ns") && res->arr[0].find("offset_ns")->num == 8.7);
  const std::string bad = "{";
  reload_offsets(a, &bad, "", "/p/ts.json", 1e9, "later", errors);
  JV s = tdelila::json::Parser(offsets_provenance_json(a)).parse();
  CHECK(s.find("status") && s.find("status")->str.find("stale") != std::string::npos);
  CHECK(s.find("reload_error") && !s.find("reload_error")->str.empty());
  JV off = tdelila::json::Parser(offsets_disabled_json()).parse();
  CHECK(off.find("applied") && !off.find("applied")->b);
  CHECK(format_offsets_line(a.table) == "m0c2=+8.700");
}

static void test_toff_run56_centres_xy() {
  // pf-qaq run 56 (2026-10-06): trigger -> arm delays (mid of 1/99 % edges) after
  // the move changed the cables. ThGEM1 xy1 = "1,2,3,5,4" (YU = ch5 up),
  // ThGEM2 xy2 = "6,7,8,9,10" (its Y wires taken reversed to match ThGEM1).
  // The offsets split each axis' raw centre symmetrically (delayline_offsets.C):
  // XL = -cx/2, XR = +cx/2, YU = +cy/2, YD = -cy/2; triggers stay 0.
  const char* j = R"({"version":"1.0","entries":[
      {"module":0,"channel":1,"ref":null,"offset_ns":0},
      {"module":0,"channel":0,"ref":[0,1],"offset_ns":0},
      {"module":0,"channel":6,"ref":[0,1],"offset_ns":0},
      {"module":0,"channel":2,"ref":[0,1],"offset_ns":8.7},
      {"module":0,"channel":3,"ref":[0,1],"offset_ns":-8.7},
      {"module":0,"channel":4,"ref":[0,1],"offset_ns":13.35},
      {"module":0,"channel":5,"ref":[0,1],"offset_ns":-13.35},
      {"module":0,"channel":7,"ref":[0,6],"offset_ns":1.5},
      {"module":0,"channel":8,"ref":[0,6],"offset_ns":-1.5},
      {"module":0,"channel":9,"ref":[0,6],"offset_ns":-5.9},
      {"module":0,"channel":10,"ref":[0,6],"offset_ns":5.9}]})";
  OffsetParseResult r = parse_time_offsets(j);
  CHECK(r.errors.empty() && r.table.warnings.empty() && r.table.max_depth == 2);
  auto event = [](double T) {
    return std::vector<ScalarHit>{
        mod_hit(0, 1, T),         mod_hit(0, 2, T + 140.6),       mod_hit(0, 3, T + 123.2),
        mod_hit(0, 4, T + 145.7), mod_hit(0, 5, T + 119.0),       mod_hit(0, 6, T + 5.0),
        mod_hit(0, 7, T + 5.0 + 129.2), mod_hit(0, 8, T + 5.0 + 126.2),
        mod_hit(0, 9, T + 5.0 + 119.7), mod_hit(0, 10, T + 5.0 + 131.5)};
  };
  auto image = [&](bool aligned, double& x1, double& y1, double& x2, double& y2) {
    PositionMatcher::Config c1;
    c1.trig_ch = 1; c1.xl_ch = 2; c1.xr_ch = 3; c1.yu_ch = 5; c1.yd_ch = 4;
    c1.window_ns = 1000.0; c1.margin_ns = 1000.0;
    PositionMatcher::Config c2 = c1;
    c2.trig_ch = 6; c2.xl_ch = 7; c2.xr_ch = 8; c2.yu_ch = 9; c2.yd_ch = 10;
    PositionMatcher m1(c1), m2(c2);
    std::vector<PosResult> o1, o2;
    std::vector<ScalarHit> h = event(10000.0);
    std::set<uint16_t> reported;
    if (aligned) apply_time_offsets(h, r.table, reported, [](uint16_t) {});
    for (const ScalarHit& x : h) {
      m1.push(x, [&](const PosResult& p) { o1.push_back(p); });
      m2.push(x, [&](const PosResult& p) { o2.push_back(p); });
    }
    m1.flush([&](const PosResult& p) { o1.push_back(p); });
    m2.flush([&](const PosResult& p) { o2.push_back(p); });
    if (o1.size() != 1 || o2.size() != 1) return false;
    x1 = o1[0].dt_xr - o1[0].dt_xl;
    y1 = o1[0].dt_yu - o1[0].dt_yd;
    x2 = o2[0].dt_xr - o2[0].dt_xl;
    y2 = o2[0].dt_yu - o2[0].dt_yd;
    return true;
  };
  double x1, y1, x2, y2;
  CHECK(image(false, x1, y1, x2, y2));
  CHECK(near(x1, -17.4, 1e-6) && near(y1, -26.7, 1e-6) && near(x2, -3.0, 1e-6) &&
        near(y2, -11.8, 1e-6));
  CHECK(image(true, x1, y1, x2, y2));
  CHECK(near(x1, 0.0) && near(y1, 0.0) && near(x2, 0.0) && near(y2, 0.0));
}

// ---------------------------------------------------------------------------
// Event definition (event_def.hpp, TODO 71 Phase A): triggers, group, window
// ---------------------------------------------------------------------------
static const double kEdefHorizon = 50e6;  // root_sink's default safe horizon (50 ms)

// The TODO 71 §4 example = config/event_def_thgem.json without its _comment.
static const std::string kEdefThGEM =
    R"({"version": "1.0",
        "window_ns": {"pre": 1000, "post": 1000},
        "group_ns": 1000,
        "triggers": [
          {"name": "ThGEM1", "priority": 1, "channels": [[0, 1]]},
          {"name": "ThGEM2", "priority": 2, "channels": [[0, 6]]}]})";

// A file from its parts; `extra` is spliced in as further top-level members.
static std::string edef_json(const std::string& window, const std::string& group,
                             const std::string& triggers, const std::string& extra = "") {
  return R"({"version":"1.0","window_ns":)" + window + R"(,"group_ns":)" + group +
         R"(,"triggers":)" + triggers + extra + "}";
}

static std::vector<std::string> edef_errs(const std::string& j, double horizon = kEdefHorizon) {
  return parse_event_def(j, horizon).errors;
}

// One-trigger file with the given trigger body (inside the braces).
static std::vector<std::string> edef_trig_errs(const std::string& body) {
  return edef_errs(edef_json(R"({"pre":1000,"post":1000})", "1000", "[{" + body + "}]"));
}

// `rel` relative to the repo root, located from this source file (not the cwd
// of the test binary's caller: __FILE__ is the path the compiler was given).
static std::string repo_path(const char* rel) {
  std::string here = __FILE__;
  std::size_t slash = here.find_last_of('/');
  return (slash == std::string::npos ? std::string() : here.substr(0, slash + 1)) + "../../" + rel;
}

static bool read_text(const std::string& path, std::string& out) {
  std::ifstream f(path, std::ios::binary);
  if (!f) return false;
  std::ostringstream ss;
  ss << f.rdbuf();
  out = ss.str();
  return true;
}

static void check_thgem_def(const EventDefParseResult& r) {
  CHECK(r.errors.empty() && r.warnings.empty());
  const EventDef& d = r.def;
  CHECK(near(d.pre_ns, 1000.0) && near(d.post_ns, 1000.0) && near(d.group_ns, 1000.0));
  CHECK(d.priorities_given);
  CHECK(d.triggers.size() == 2);
  if (d.triggers.size() != 2) return;
  CHECK(d.triggers[0].name == "ThGEM1" && d.triggers[0].priority == 1);
  CHECK(d.triggers[1].name == "ThGEM2" && d.triggers[1].priority == 2);
  CHECK(d.triggers[0].channels.size() == 1 && d.triggers[0].channels[0] == mc_key(0, 1));
  CHECK(d.triggers[1].channels.size() == 1 && d.triggers[1].channels[0] == mc_key(0, 6));
  CHECK(d.trig_of.size() == 65536);  // index_channels() ran
  CHECK(d.trigger_index(0, 1) == 0 && d.trigger_index(0, 6) == 1);
  CHECK(d.trigger_index(0, 2) == -1 && d.trigger_index(0, 0) == -1);  // arm, LaBr3
  CHECK(d.trigger_index(1, 1) == -1);                                 // other module
  CHECK(near(d.lookback_ns(), 4000.0));                               // G + 2 pre + post
}

static void test_edef_thgem_example() {
  check_thgem_def(parse_event_def(kEdefThGEM, kEdefHorizon));
  // A hand-built def that never ran index_channels(): no trigger at all.
  EventDef raw;
  raw.triggers.push_back(TriggerDef{"T", 0, {mc_key(0, 1)}});
  CHECK(raw.trigger_index(0, 1) == -1);
  raw.index_channels();
  CHECK(raw.trigger_index(0, 1) == 0);
}

static void test_edef_thgem_file() {
  std::string text;
  CHECK(read_text(repo_path("config/event_def_thgem.json"), text));
  EventDefParseResult r = parse_event_def(text, kEdefHorizon);
  check_thgem_def(r);
  CHECK(format_event_def_line(r.def) ==
        "window [-1000, +1000] ns, group 1000 ns, 2 trigger(s): ThGEM1(p1: m0c1) ThGEM2(p2: m0c6)");
}

static void test_edef_no_priority() {
  EventDefParseResult r = parse_event_def(
      edef_json(R"({"pre":500,"post":2000})", "250",
                R"([{"name":"Si","channels":[[0,0],[0,1]]},{"name":"Ge","channels":[[1,4]]}])"),
      kEdefHorizon);
  CHECK(r.errors.empty() && !r.def.priorities_given);
  CHECK(r.def.triggers.size() == 2 && r.def.triggers[0].priority == 0 &&
        r.def.triggers[1].priority == 0);
  CHECK(r.def.trigger_index(0, 0) == 0 && r.def.trigger_index(0, 1) == 0 &&
        r.def.trigger_index(1, 4) == 1 && r.def.trigger_index(0, 4) == -1);
  CHECK(r.def.triggers[0].channels.size() == 2 && r.def.triggers[0].channels[1] == mc_key(0, 1));
  CHECK(near(r.def.lookback_ns(), 250.0 + 1000.0 + 2000.0));
  CHECK(format_event_def_line(r.def) ==
        "window [-500, +2000] ns, group 250 ns, 2 trigger(s), reference = first of group: "
        "Si(m0c0,m0c1) Ge(m1c4)");
  // Priorities given: multi-channel form, negative and tied priorities are fine.
  EventDefParseResult p = parse_event_def(
      edef_json(R"({"pre":1000,"post":1000})", "100",
                R"([{"name":"Si","priority":-1,"channels":[[0,0],[0,1]]},
                    {"name":"Ge","priority":5,"channels":[[1,4]]},
                    {"name":"PMT","priority":5,"channels":[[2,0]]}])"),
      kEdefHorizon);
  CHECK(p.errors.empty() && p.def.priorities_given);
  CHECK(p.def.triggers.size() == 3 && p.def.triggers[0].priority == -1 &&
        p.def.triggers[2].priority == 5);
  CHECK(format_event_def_line(p.def) ==
        "window [-1000, +1000] ns, group 100 ns, 3 trigger(s): Si(p-1: m0c0,m0c1) Ge(p5: m1c4) "
        "PMT(p5: m2c0)");
  // Consecutive channels collapse to ranges (>= 3 in a run, per module).
  {
    std::string chs;
    for (int m = 1; m <= 3; ++m)
      for (int c = 0; c <= 14; ++c)
        chs += std::string(chs.empty() ? "" : ",") + "[" + std::to_string(m) + "," +
               std::to_string(c) + "]";
    EventDefParseResult rr = parse_event_def(
        edef_json(R"({"pre":2000,"post":2000})", "2000",
                  "[{\"name\":\"Ring\",\"channels\":[" + chs + "]},"
                  "{\"name\":\"X\",\"channels\":[[0,1],[0,2],[0,3],[0,5],[0,6],[4,7]]}]"),
        kEdefHorizon);
    CHECK(rr.errors.empty());
    CHECK(format_event_def_line(rr.def).find("Ring(m1c0-14,m2c0-14,m3c0-14)") != std::string::npos);
    CHECK(format_event_def_line(rr.def).find("X(m0c1-3,m0c5,m0c6,m4c7)") != std::string::npos);
  }
}

static void test_edef_priority_rules() {
  const std::string w = R"({"pre":1000,"post":1000})";
  // All-or-none: a missing priority would silently become "best" or "worst".
  std::vector<std::string> e = edef_errs(edef_json(
      w, "100",
      R"([{"name":"A","priority":1,"channels":[[0,1]]},{"name":"B","channels":[[0,2]]}])"));
  CHECK(e.size() == 1 && any_contains(e, "all triggers or for none") && any_contains(e, "'B'"));
  // Not an integer / out of int range / not a number.
  CHECK(any_contains(edef_trig_errs(R"("name":"A","priority":1.5,"channels":[[0,1]])"),
                     "'priority' must be an integer"));
  CHECK(any_contains(edef_trig_errs(R"("name":"A","priority":3e9,"channels":[[0,1]])"),
                     "'priority' must be an integer"));
  CHECK(any_contains(edef_trig_errs(R"("name":"A","priority":"1","channels":[[0,1]])"),
                     "'priority' must be an integer"));
  CHECK(edef_trig_errs(R"("name":"A","priority":null,"channels":[[0,1]])").size() == 1);
}

static void test_edef_group_vs_window() {
  const std::string trig = R"([{"name":"T","channels":[[0,1]]}])";
  auto errs = [&](const char* pre, const char* post, const char* g) {
    return edef_errs(edef_json(std::string(R"({"pre":)") + pre + R"(,"post":)" + post + "}", g,
                               trig));
  };
  // G > pre and G > post, symmetric and asymmetric both ways: the reference can
  // be the group's last trigger (others up to G before it) or its first.
  std::vector<std::string> e = errs("1000", "1000", "1001");
  CHECK(e.size() == 1 && any_contains(e, "group_ns 1001 > min(") &&
        any_contains(e, "wherever the reference sits"));
  CHECK(any_contains(errs("500", "1000", "800"), "group_ns"));
  CHECK(any_contains(errs("1000", "500", "800"), "group_ns"));
  CHECK(any_contains(errs("1000", "500", "800"), "window_ns.post 500"));
  // G == min(pre, post) is the limit, still fine; so is G = 0 and a zero window.
  CHECK(errs("500", "1000", "500").empty());
  CHECK(errs("1000", "500", "500").empty());
  CHECK(errs("1000", "1000", "1000").empty());
  CHECK(errs("1000", "1000", "0").empty());
  CHECK(errs("0", "0", "0").empty());
  EventDefParseResult asym = parse_event_def(
      edef_json(R"({"pre":300,"post":700})", "300", trig), kEdefHorizon);
  CHECK(asym.errors.empty() && near(asym.def.pre_ns, 300.0) && near(asym.def.post_ns, 700.0));
}

static void test_edef_window_errors() {
  const std::string trig = R"([{"name":"T","channels":[[0,1]]}])";
  auto win = [&](const std::string& w, const std::string& g = "0") {
    return edef_errs(edef_json(w, g, trig));
  };
  // Exactly one error each: an invalid window does not cascade into a G error.
  CHECK(win(R"({"pre":-1,"post":1000})", "1000").size() == 1);
  CHECK(any_contains(win(R"({"pre":-1,"post":1000})"),
                     "'window_ns.pre' must be a finite number >= 0"));
  CHECK(any_contains(win(R"({"pre":1000,"post":"1000"})"), "window_ns.post"));
  CHECK(any_contains(win(R"({"pre":1000,"post":null})"), "window_ns.post"));
  CHECK(any_contains(win(R"({"pre":1000})"), "missing 'window_ns.post'"));
  CHECK(win(R"({})").size() == 2);  // both missing
  CHECK(any_contains(win("1000"), "'window_ns' must be an object"));
  CHECK(!win(R"({"pre":1e999,"post":1000})").empty());  // not finite: rejected by the parser
  CHECK(any_contains(win(R"({"pre":10,"post":10,"pos":10})"), "unknown key 'pos'"));
  CHECK(win(R"({"_comment":"symmetric","pre":10,"post":10})").empty());
  CHECK(!win(R"({"_comment":7,"pre":10,"post":10})").empty());
  CHECK(any_contains(win(R"({"pre":10,"post":10})", "-5"), "'group_ns' must be a finite number"));
  CHECK(any_contains(win(R"({"pre":10,"post":10})", "\"5\""), "'group_ns'"));
  // Missing members.
  CHECK(any_contains(edef_errs(R"({"version":"1.0","group_ns":0,"triggers":)" + trig + "}"),
                     "'window_ns' must be an object"));
  CHECK(any_contains(edef_errs(R"({"version":"1.0","window_ns":{"pre":1,"post":1},"triggers":)" +
                               trig + "}"),
                     "missing 'group_ns'"));
}

static void test_edef_channel_errors() {
  const std::string w = R"({"pre":1000,"post":1000})";
  // The same channel twice inside one trigger, and across two triggers.
  std::vector<std::string> in = edef_trig_errs(R"("name":"ThGEM1","channels":[[0,1],[0,2],[0,1]])");
  CHECK(in.size() == 1 && any_contains(in, "m0c1 listed twice in trigger 'ThGEM1'"));
  std::vector<std::string> across = edef_errs(edef_json(
      w, "100",
      R"([{"name":"ThGEM1","channels":[[0,1]]},{"name":"ThGEM2","channels":[[0,6],[0,1]]}])"));
  CHECK(across.size() == 1 &&
        any_contains(across, "m0c1 listed in both trigger 'ThGEM1' and trigger 'ThGEM2'"));
  // Same channel number on another module is a different channel.
  CHECK(edef_errs(edef_json(w, "100",
                            R"([{"name":"A","channels":[[0,1]]},{"name":"B","channels":[[1,1]]}])"))
            .empty());
  // Bad [module, channel] pairs.
  const char* bad[] = {"[[300,0]]", "[[0,-1]]", "[[0,1.5]]", "[[0]]", "[[0,1,2]]",
                       "[\"0_1\"]", "[[\"0\",1]]", "[[0,256]]"};
  for (const char* b : bad)
    CHECK(any_contains(edef_trig_errs(std::string(R"("name":"T","channels":)") + b),
                       "channels[0] must be [module, channel]"));
  CHECK(edef_trig_errs(R"("name":"T","channels":[[255,255]])").empty());
  CHECK(any_contains(edef_trig_errs(R"("name":"T","channels":[])"), "'channels' is empty"));
  CHECK(any_contains(edef_trig_errs(R"("name":"T")"), "'channels' must be a non-empty array"));
  CHECK(any_contains(edef_trig_errs(R"("name":"T","channels":[0,1])"), "channels[1]"));
}

static void test_edef_trigger_errors() {
  const std::string w = R"({"pre":1000,"post":1000})";
  std::vector<std::string> dup = edef_errs(edef_json(
      w, "100", R"([{"name":"A","channels":[[0,1]]},{"name":"A","channels":[[0,2]]}])"));
  CHECK(dup.size() == 1 && any_contains(dup, "triggers[0] and triggers[1]: duplicate name 'A'"));
  CHECK(any_contains(edef_errs(edef_json(w, "100", "[]")), "'triggers' is empty"));
  CHECK(any_contains(edef_errs(edef_json(w, "100", "{}")), "'triggers' must be a non-empty array"));
  CHECK(any_contains(edef_errs(R"({"version":"1.0","window_ns":{"pre":1,"post":1},"group_ns":0})"),
                     "'triggers' must be"));
  CHECK(any_contains(edef_errs(edef_json(w, "100", "[5]")), "triggers[0]: not an object"));
  CHECK(any_contains(edef_trig_errs(R"("channels":[[0,1]])"), "'name' must be a non-empty string"));
  CHECK(any_contains(edef_trig_errs(R"("name":"","channels":[[0,1]])"), "'name'"));
  CHECK(any_contains(edef_trig_errs(R"("name":3,"channels":[[0,1]])"), "'name'"));
  // Tag-based selection needs the channel table (Phase C): a specific message.
  std::vector<std::string> tags = edef_trig_errs(R"("name":"Si","tags":["SiE"])");
  CHECK(any_contains(tags, "triggers[0]: 'tags' needs the channel table (TODO 71 Phase C); use "
                           "'channels'"));
  CHECK(any_contains(edef_trig_errs(R"("name":"T","channels":[[0,1]],"chanels":[[0,2]])"),
                     "unknown key 'chanels'"));
  CHECK(edef_trig_errs(R"("name":"T","_comment":"front strips","channels":[[0,1]])").empty());
  // A trigger with a bad name still takes part in the duplicate-channel check.
  CHECK(any_contains(edef_errs(edef_json(
                         w, "100", R"([{"name":"A","channels":[[0,1]]},{"channels":[[0,1]]}])")),
                     "m0c1 listed in both trigger 'A' and triggers[1]"));
  // TriggerIdx is 8-bit: 256 triggers are fine, 257 are not.
  auto many = [&](int n) {
    std::string t = "[";
    for (int i = 0; i < n; ++i)
      t += std::string(i ? "," : "") + R"({"name":"t)" + std::to_string(i) + R"(","channels":[[)" +
           std::to_string(i / 256) + "," + std::to_string(i % 256) + "]]}";
    return parse_event_def(edef_json(w, "100", t + "]"), kEdefHorizon);
  };
  EventDefParseResult m256 = many(256);
  CHECK(m256.errors.empty() && m256.def.trigger_index(0, 255) == 255);
  CHECK(any_contains(many(257).errors, "at most 256"));
}

static void test_edef_top_level() {
  const std::string w = R"({"pre":1000,"post":1000})";
  const std::string trig = R"([{"name":"T","channels":[[0,1]]}])";
  CHECK(any_contains(edef_errs(edef_json(w, "100", trig, R"(,"trigers":[])")),
                     "unknown top-level key 'trigers'"));
  CHECK(edef_errs(edef_json(w, "100", trig, R"(,"$schema":"x","_comment":["a","b"])")).empty());
  CHECK(!edef_errs(edef_json(w, "100", trig, R"(,"_comment":5)")).empty());
  EventDefParseResult v2 = parse_event_def(
      R"({"version":"2.0","window_ns":{"pre":1,"post":1},"group_ns":0,"triggers":)" + trig + "}",
      kEdefHorizon);
  CHECK(v2.errors.empty() && v2.warnings.size() == 1 && any_contains(v2.warnings, "\"2.0\""));
  CHECK(any_contains(edef_errs(R"({"window_ns":{"pre":1,"post":1},"group_ns":0,"triggers":)" +
                               trig + "}"),
                     "'version'"));
  CHECK(any_contains(edef_errs(""), "empty file"));
  CHECK(any_contains(edef_errs(" \n\t"), "empty file"));
  CHECK(edef_errs("{").size() == 1 && any_contains(edef_errs("{"), "malformed JSON"));
  CHECK(any_contains(edef_errs("[1,2]"), "top level must be a JSON object"));
}

static void test_edef_lookback_and_collect_all() {
  // lookback = G + 2 pre + post = 4000 ns for the ThGEM file; the bound is strict.
  std::vector<std::string> e = edef_errs(kEdefThGEM, 4000.0);
  CHECK(e.size() == 1 && any_contains(e, "4000 ns >= Sorter safe horizon 4000 ns"));
  CHECK(edef_errs(kEdefThGEM, 4000.5).empty());
  // A typo'd window of seconds is caught by the default horizon.
  CHECK(any_contains(edef_errs(edef_json(R"({"pre":1e8,"post":1e8})", "0",
                                         R"([{"name":"T","channels":[[0,1]]}])")),
                     "safe horizon"));
  // Every problem at once (and no lookback complaint on top of real errors).
  std::vector<std::string> all = edef_errs(
      R"({"version":"1.0","bogus":1,"window_ns":{"pre":-5,"post":1e8},"group_ns":0,
          "triggers":[{"name":"A","channels":[[0,1]]},
                      {"name":"A","tags":["x"],"channels":[[0,2],[400,0]]}]})");
  CHECK(all.size() == 5);
  CHECK(any_contains(all, "'bogus'") && any_contains(all, "window_ns.pre") &&
        any_contains(all, "duplicate name 'A'") && any_contains(all, "'tags'") &&
        any_contains(all, "channels[1]"));
  CHECK(!any_contains(all, "safe horizon"));
}

static void test_edef_reload_policy() {
  const std::string A = kEdefThGEM;
  const std::string B = edef_json(R"({"pre":500,"post":500})", "500",
                                  R"([{"name":"Only","channels":[[0,3]]}])");
  const std::string bad = edef_json(R"({"pre":500,"post":500})", "600",
                                    R"([{"name":"Only","channels":[[0,3]]}])");
  ActiveEventDef a;
  std::vector<std::string> errors;
  CHECK(reload_event_def(a, &A, "", "/x/ed.json", kEdefHorizon, "t0", errors));
  CHECK(errors.empty() && a.loaded && !a.stale && a.def && a.def->triggers.size() == 2);
  CHECK(a.source_text == A && a.source_path == "/x/ed.json" && a.loaded_at == "t0");
  std::shared_ptr<const EventDef> frozen = a.def;  // what a running Sorter holds
  // Invalid file at a run boundary: keep the previous def (same object) AND text.
  CHECK(!reload_event_def(a, &bad, "", "/x/ed.json", kEdefHorizon, "t1", errors));
  CHECK(!errors.empty() && a.stale && a.def == frozen && a.source_text == A &&
        a.loaded_at == "t0" && a.stale_error.find("group_ns") != std::string::npos);
  // Unreadable file.
  CHECK(!reload_event_def(a, nullptr, "No such file or directory", "/x/ed.json", kEdefHorizon,
                          "t2", errors));
  CHECK(a.stale && a.def == frozen && any_contains(errors, "cannot read /x/ed.json") &&
        any_contains(errors, "No such file"));
  // Lookback over the horizon is a failure too.
  CHECK(!reload_event_def(a, &A, "", "/x/ed.json", 100.0, "t3", errors));
  CHECK(any_contains(errors, "safe horizon") && a.def == frozen);
  // Recovery: a NEW object, stale cleared; the frozen one is untouched.
  CHECK(reload_event_def(a, &B, "", "/y/ed.json", kEdefHorizon, "t4", errors));
  CHECK(!a.stale && a.stale_error.empty() && a.def != frozen && a.source_path == "/y/ed.json");
  // Warnings survive the reload path (no silent failure): kept with the def in
  // force and recorded in the provenance; a clean reload clears them.
  {
    ActiveEventDef w;
    std::string v2 = A;
    v2.replace(v2.find("\"1.0\""), 5, "\"2.0\"");
    CHECK(reload_event_def(w, &v2, "", "/x/ed.json", kEdefHorizon, "t5", errors));
    CHECK(w.warnings.size() == 1 && w.warnings[0].find("2.0") != std::string::npos);
    CHECK(event_def_provenance_json(w).find("\"warnings\":[\"version") != std::string::npos);
    CHECK(reload_event_def(w, &A, "", "/x/ed.json", kEdefHorizon, "t6", errors));
    CHECK(w.warnings.empty());
  }
  CHECK(a.def->triggers.size() == 1 && a.def->trigger_index(0, 3) == 0);
  CHECK(frozen->triggers.size() == 2 && frozen->trigger_index(0, 1) == 0);
  // Reloading the very same text still yields a fresh object (frozen per run).
  std::shared_ptr<const EventDef> prev = a.def;
  CHECK(reload_event_def(a, &B, "", "/y/ed.json", kEdefHorizon, "t5", errors));
  CHECK(a.def != prev);
  // First load failing leaves nothing loaded (the caller exits at startup).
  ActiveEventDef fresh;
  CHECK(!reload_event_def(fresh, &bad, "", "p", kEdefHorizon, "t", errors) && !fresh.loaded &&
        !fresh.def && fresh.stale);
}

static void test_edef_provenance_json() {
  const std::string src =
      "{\"version\":\"1.0\",\"_comment\":\"tab\\t \\\"quoted\\\" back\\\\slash\",\n"
      "\"window_ns\":{\"pre\":1000.1,\"post\":900},\"group_ns\":0.3,\n"
      "\"triggers\":[{\"name\":\"ThGEM1\",\"priority\":1,\"channels\":[[0,1]]},\n"
      "              {\"name\":\"Si \\\"E\\\"\",\"priority\":2,\"channels\":[[2,7],[2,8]]}]}";
  ActiveEventDef a;
  std::vector<std::string> errors;
  CHECK(reload_event_def(a, &src, "", "/p/ed.json", kEdefHorizon, "2026-10-07T00:00:00Z", errors));
  using JV = tdelila::json::Value;
  JV v = tdelila::json::Parser(event_def_provenance_json(a)).parse();
  CHECK(v.find("applied") && v.find("applied")->b);
  CHECK(v.find("convention") && v.find("convention")->str.find("ALIGNED") != std::string::npos);
  CHECK(v.find("status") && v.find("status")->str == "ok" && !v.find("reload_error"));
  CHECK(v.find("source_path") && v.find("source_path")->str == "/p/ed.json");
  CHECK(v.find("loaded_at") && v.find("loaded_at")->str == "2026-10-07T00:00:00Z");
  CHECK(v.find("source_text") && v.find("source_text")->str == src);  // round-trips exactly
  const JV* w = v.find("window_ns");
  CHECK(w && w->find("pre") && w->find("pre")->num == 1000.1);  // %.17g: exact
  CHECK(w && w->find("post") && w->find("post")->num == 900.0);
  CHECK(v.find("group_ns") && v.find("group_ns")->num == 0.3);
  CHECK(v.find("priorities_given") && v.find("priorities_given")->b);
  const JV* t = v.find("triggers");
  CHECK(t && t->t == JV::T::Arr && t->arr.size() == 2);
  if (t && t->arr.size() == 2) {
    const JV& t1 = t->arr[1];
    CHECK(t1.find("index") && t1.find("index")->num == 1.0);
    CHECK(t1.find("name") && t1.find("name")->str == "Si \"E\"");
    CHECK(t1.find("priority") && t1.find("priority")->num == 2.0);
    const JV* ch = t1.find("channels");
    CHECK(ch && ch->arr.size() == 2 && ch->arr[1].arr.size() == 2 && ch->arr[1].arr[0].num == 2.0 &&
          ch->arr[1].arr[1].num == 8.0);
    CHECK(t->arr[0].find("index") && t->arr[0].find("index")->num == 0.0);
  }
  // Stale: the previous definition is still what the provenance describes.
  const std::string bad = "{";
  reload_event_def(a, &bad, "", "/p/ed.json", kEdefHorizon, "later", errors);
  JV s = tdelila::json::Parser(event_def_provenance_json(a)).parse();
  CHECK(s.find("status") &&
        s.find("status")->str == "stale: run-start reload failed, previous definition kept");
  CHECK(s.find("reload_error") && s.find("reload_error")->str.find("malformed JSON") == 0);
  CHECK(s.find("source_text") && s.find("source_text")->str == src);
  CHECK(s.find("loaded_at") && s.find("loaded_at")->str == "2026-10-07T00:00:00Z");
  // Without priorities: flag false, every priority 0.
  const std::string np = edef_json(R"({"pre":10,"post":10})", "0",
                                   R"([{"name":"T","channels":[[0,1]]}])");
  ActiveEventDef b;
  CHECK(reload_event_def(b, &np, "", "q", kEdefHorizon, "t", errors));
  JV n = tdelila::json::Parser(event_def_provenance_json(b)).parse();
  CHECK(n.find("priorities_given") && n.find("priorities_given")->t == JV::T::Bool &&
        !n.find("priorities_given")->b);
  // Never loaded: still valid JSON, says so.
  JV z = tdelila::json::Parser(event_def_provenance_json(ActiveEventDef{})).parse();
  CHECK(z.find("applied") && !z.find("applied")->b);
}

// ---------------------------------------------------------------------------
// Coincidence matcher tests (with energies)
// ---------------------------------------------------------------------------
static ScalarHit hit(uint8_t ch, double t, uint16_t e) {
  ScalarHit h;
  h.channel = ch;
  h.timestamp_ns = t;
  h.energy = e;
  return h;
}

static void test_matcher() {
  CoincidenceMatcher::Config cfg;
  cfg.gamma_ch = 0;
  cfg.thgem1_ch = 1;
  cfg.thgem2_ch = 2;
  cfg.window_ns = 100.0;
  cfg.margin_ns = 1000.0;

  // Closest-partner selection + energy propagation.
  {
    CoincidenceMatcher m(cfg);
    std::vector<CoincResult> res;
    auto sink = [&](const CoincResult& r) { res.push_back(r); };
    m.push(hit(0, 1000.0, 500), sink);  // gamma, energy 500
    m.push(hit(1, 1050.0, 111), sink);  // thgem1 d=+50
    m.push(hit(1, 1080.0, 222), sink);  // thgem1 d=+80 (farther)
    m.push(hit(2, 1030.0, 999), sink);  // thgem2 d=+30
    m.flush(sink);
    CHECK(res.size() == 1);
    CHECK(res[0].gamma_energy == 500);
    CHECK(res[0].has_dt1);
    CHECK(near(res[0].dt1, 50.0));
    CHECK(res[0].thgem1_energy == 111);  // the nearer partner's energy
    CHECK(res[0].has_dt2);
    CHECK(near(res[0].dt2, 30.0));
    CHECK(res[0].thgem2_energy == 999);
  }
  // Window edge: |dt| == window passes.
  {
    CoincidenceMatcher m(cfg);
    std::vector<CoincResult> res;
    auto sink = [&](const CoincResult& r) { res.push_back(r); };
    m.push(hit(0, 1000.0, 1), sink);
    m.push(hit(1, 1100.0, 7), sink);  // d=+100 == window
    m.flush(sink);
    CHECK(res.size() == 1);
    CHECK(res[0].has_dt1);
    CHECK(near(res[0].dt1, 100.0));
    CHECK(res[0].thgem1_energy == 7);
  }
  // Out of window: no partner.
  {
    CoincidenceMatcher m(cfg);
    std::vector<CoincResult> res;
    auto sink = [&](const CoincResult& r) { res.push_back(r); };
    m.push(hit(0, 1000.0, 1), sink);
    m.push(hit(1, 1101.0, 7), sink);  // d=+101 > window
    m.flush(sink);
    CHECK(res.size() == 1);
    CHECK(!res[0].has_dt1);
    CHECK(!res[0].has_dt2);
    CHECK(res[0].gamma_energy == 1);
  }
  // Watermark ripening: a far-future gamma pushes the earlier one over the mark,
  // and arrival disorder (partner buffered before the gamma) is tolerated.
  {
    CoincidenceMatcher m(cfg);
    std::vector<CoincResult> res;
    auto sink = [&](const CoincResult& r) { res.push_back(r); };
    m.push(hit(1, 1050.0, 2), sink);  // thgem1 arrives first
    m.push(hit(0, 1000.0, 1), sink);  // gamma arrives later, earlier timestamp
    CHECK(res.empty());               // not yet ripe (watermark below gamma_t)
    m.push(hit(0, 5000.0, 9), sink);  // advance watermark to 4000 -> ripen gamma@1000
    CHECK(res.size() == 1);
    CHECK(near(res[0].gamma_t, 1000.0));
    CHECK(res[0].has_dt1);
    CHECK(near(res[0].dt1, 50.0));
    CHECK(res[0].thgem1_energy == 2);
    m.flush(sink);                    // ripen the remaining gamma@5000 (no partner)
    CHECK(res.size() == 2);
    CHECK(!res[1].has_dt1);
    CHECK(res[1].gamma_energy == 9);
  }
  // reset clears state: buffered hits are dropped, flush yields nothing.
  {
    CoincidenceMatcher m(cfg);
    std::vector<CoincResult> res;
    auto sink = [&](const CoincResult& r) { res.push_back(r); };
    m.push(hit(0, 1000.0, 1), sink);
    m.push(hit(1, 1010.0, 2), sink);
    m.reset();
    m.flush(sink);
    CHECK(res.empty());
  }
}

// ---------------------------------------------------------------------------
// Position matcher tests (delay-line XY)
// ---------------------------------------------------------------------------
static void test_pos_matcher() {
  PositionMatcher::Config cfg;
  cfg.trig_ch = 1;
  cfg.xl_ch = 2;
  cfg.xr_ch = 3;
  cfg.yu_ch = 4;
  cfg.yd_ch = 5;
  cfg.window_ns = 100.0;
  cfg.margin_ns = 1000.0;

  // Full match: all four arms found, signed dt per arm, trigger energy kept.
  {
    PositionMatcher m(cfg);
    std::vector<PosResult> res;
    auto sink = [&](const PosResult& r) { res.push_back(r); };
    m.push(hit(1, 1000.0, 321), sink);  // trigger
    m.push(hit(2, 1010.0, 0), sink);    // XL d=+10
    m.push(hit(3, 1030.0, 0), sink);    // XR d=+30
    m.push(hit(4, 990.0, 0), sink);     // YU d=-10
    m.push(hit(5, 1005.0, 0), sink);    // YD d=+5
    m.flush(sink);
    CHECK(res.size() == 1);
    CHECK(near(res[0].trig_t, 1000.0));
    CHECK(res[0].trig_energy == 321);
    CHECK(res[0].has_xl && near(res[0].dt_xl, 10.0));
    CHECK(res[0].has_xr && near(res[0].dt_xr, 30.0));
    CHECK(res[0].has_yu && near(res[0].dt_yu, -10.0));
    CHECK(res[0].has_yd && near(res[0].dt_yd, 5.0));
  }
  // Nearest-arm selection: of two XL hits the closer wins.
  {
    PositionMatcher m(cfg);
    std::vector<PosResult> res;
    auto sink = [&](const PosResult& r) { res.push_back(r); };
    m.push(hit(1, 1000.0, 1), sink);
    m.push(hit(2, 1050.0, 0), sink);  // XL d=+50
    m.push(hit(2, 1010.0, 0), sink);  // XL d=+10 (closer)
    m.flush(sink);
    CHECK(res.size() == 1);
    CHECK(res[0].has_xl && near(res[0].dt_xl, 10.0));
  }
  // Missing arm: no XR hit -> has_xr false, the others still match.
  {
    PositionMatcher m(cfg);
    std::vector<PosResult> res;
    auto sink = [&](const PosResult& r) { res.push_back(r); };
    m.push(hit(1, 1000.0, 1), sink);
    m.push(hit(2, 1010.0, 0), sink);
    m.push(hit(4, 1020.0, 0), sink);
    m.push(hit(5, 1030.0, 0), sink);
    m.flush(sink);
    CHECK(res.size() == 1);
    CHECK(res[0].has_xl);
    CHECK(!res[0].has_xr);
    CHECK(res[0].has_yu && res[0].has_yd);
  }
  // Window edge: |d| == window passes, |d| == window+1 does not.
  {
    PositionMatcher m(cfg);
    std::vector<PosResult> res;
    auto sink = [&](const PosResult& r) { res.push_back(r); };
    m.push(hit(1, 1000.0, 1), sink);
    m.push(hit(2, 1100.0, 0), sink);  // d=+100 == window
    m.push(hit(3, 1101.0, 0), sink);  // d=+101 > window
    m.flush(sink);
    CHECK(res.size() == 1);
    CHECK(res[0].has_xl && near(res[0].dt_xl, 100.0));
    CHECK(!res[0].has_xr);
  }
  // Arrival disorder + watermark: an arm buffered before its trigger matches;
  // nothing ripens until the watermark passes the trigger timestamp. Unmonitored
  // channels must also advance the watermark (max_ts before the channel test).
  {
    PositionMatcher m(cfg);
    std::vector<PosResult> res;
    auto sink = [&](const PosResult& r) { res.push_back(r); };
    m.push(hit(2, 1050.0, 0), sink);   // XL arrives first
    m.push(hit(1, 1000.0, 55), sink);  // trigger arrives later, earlier timestamp
    CHECK(res.empty());                // watermark still below trig_t
    m.push(hit(0, 5000.0, 0), sink);   // unmonitored ch0 advances watermark to 4000
    CHECK(res.size() == 1);
    CHECK(near(res[0].trig_t, 1000.0));
    CHECK(res[0].trig_energy == 55);
    CHECK(res[0].has_xl && near(res[0].dt_xl, 50.0));
  }
  // reset clears buffered state: flush after reset yields nothing.
  {
    PositionMatcher m(cfg);
    std::vector<PosResult> res;
    auto sink = [&](const PosResult& r) { res.push_back(r); };
    m.push(hit(1, 1000.0, 1), sink);
    m.push(hit(2, 1010.0, 0), sink);
    m.reset();
    m.flush(sink);
    CHECK(res.empty());
  }
}

// ---------------------------------------------------------------------------
// parse_ch_list tests (backs --xy1-ch/--xy2-ch)
// ---------------------------------------------------------------------------
static void test_parse_ch_list() {
  std::vector<int> out;
  CHECK(parse_ch_list("1,2,3,4,5", 5, out));
  CHECK(out.size() == 5 && out[0] == 1 && out[4] == 5);
  CHECK(parse_ch_list("0,255,10,20,30", 5, out));   // bounds are inclusive
  CHECK(!parse_ch_list("1,2,3,4", 5, out));         // too few
  CHECK(!parse_ch_list("1,2,3,4,5,6", 5, out));     // too many
  CHECK(!parse_ch_list("1,2,x,4,5", 5, out));       // junk token
  CHECK(!parse_ch_list("1,2,-3,4,5", 5, out));      // negative
  CHECK(!parse_ch_list("1,2,300,4,5", 5, out));     // > 255
  CHECK(!parse_ch_list("1,1,3,4,5", 5, out));       // duplicate (trig would
                                                    // silently shadow the arm)
  CHECK(!parse_ch_list("", 5, out));                // empty
  CHECK(!parse_ch_list("1,2,3,4,", 5, out));        // trailing comma
}

// ---------------------------------------------------------------------------
// RunState tests
// ---------------------------------------------------------------------------
static void test_run_state() {
  // Single source: one Data opens, one EOS finalizes.
  {
    RunState rs;
    int opens = 0, finals = 0, stales = 0;
    uint32_t final_run = 0;
    bool opened = rs.on_data(1, [&] { ++opens; });
    CHECK(opened);
    CHECK(rs.is_writing());
    rs.on_data(1, [&] { ++opens; });  // same run, no re-open
    CHECK(opens == 1);
    bool closed = rs.on_eos(
        1, 55, [&](uint32_t rn) { ++finals; final_run = rn; },
        [&](uint32_t, uint32_t) { ++stales; });
    CHECK(closed);
    CHECK(finals == 1);
    CHECK(final_run == 55);
    CHECK(!rs.is_writing());
  }
  // Multi-source: run closes only after EVERY seen source has sent EOS.
  {
    RunState rs;
    int finals = 0;
    rs.on_data(1, [] {});
    rs.on_data(2, [] {});
    bool c1 = rs.on_eos(
        1, 7, [&](uint32_t) { ++finals; }, [](uint32_t, uint32_t) {});
    CHECK(!c1);
    CHECK(rs.is_writing());
    CHECK(finals == 0);
    bool c2 = rs.on_eos(
        2, 7, [&](uint32_t) { ++finals; }, [](uint32_t, uint32_t) {});
    CHECK(c2);
    CHECK(finals == 1);
    CHECK(!rs.is_writing());
  }
  // Stale EOS while idle: stale callback, no finalize.
  {
    RunState rs;
    int finals = 0, stales = 0;
    bool c = rs.on_eos(
        1, 9, [&](uint32_t) { ++finals; }, [&](uint32_t, uint32_t) { ++stales; });
    CHECK(!c);
    CHECK(finals == 0);
    CHECK(stales == 1);
  }
}

// ---------------------------------------------------------------------------
// HTTP helper tests
// ---------------------------------------------------------------------------
static void test_http_helpers() {
  // Split header/body at the first blank line.
  {
    std::string raw =
        "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}";
    std::string hdr, body;
    bool ok = split_http_response(raw, hdr, body);
    CHECK(ok);
    CHECK(hdr == "HTTP/1.0 200 OK\r\nContent-Type: application/json");
    CHECK(body == "{\"a\":1}");
  }
  // No boundary -> false, whole input as header.
  {
    std::string hdr, body;
    bool ok = split_http_response("garbage no header end", hdr, body);
    CHECK(!ok);
    CHECK(body.empty());
  }
  // Valid status JSON -> experiment_name.
  {
    std::string body =
        "{\"experiment_name\":\"psd1_test\",\"system_state\":\"Running\"}";
    CHECK(extract_experiment_name(body) == "psd1_test");
  }
  // Missing key -> "".
  {
    CHECK(extract_experiment_name("{\"system_state\":\"Idle\"}").empty());
  }
  // Garbage -> "".
  {
    CHECK(extract_experiment_name("not json at all").empty());
  }
}

// ---------------------------------------------------------------------------
// hist_config tests
// ---------------------------------------------------------------------------
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

// ---------------------------------------------------------------------------
int main() {
  test_envelope();
  test_batch_decode();
  test_matcher();
  test_keep_module();
  test_foreign_module_hits_do_not_reach_the_matcher();
  test_pos_matcher();
  test_parse_ch_list();
  test_run_state();
  test_http_helpers();
  test_hist_config_valid();
  test_hist_config_features();
  test_hist_config_errors();
  test_value_and_cut();
  test_hist_config_pos();
  test_pos_value_and_cut();
  test_toff_sign_convention();
  test_toff_tree_resolves();
  test_toff_root_offset_applied();
  test_toff_tree_errors();
  test_toff_tree_warnings();
  test_toff_legacy();
  test_toff_module_specific();
  test_toff_missing_once_per_run();
  test_toff_reload_policy();
  test_toff_provenance_json();
  test_toff_run56_centres_xy();
  test_edef_thgem_example();
  test_edef_thgem_file();
  test_edef_no_priority();
  test_edef_priority_rules();
  test_edef_group_vs_window();
  test_edef_window_errors();
  test_edef_channel_errors();
  test_edef_trigger_errors();
  test_edef_top_level();
  test_edef_lookback_and_collect_all();
  test_edef_reload_policy();
  test_edef_provenance_json();

  std::printf("\n%d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

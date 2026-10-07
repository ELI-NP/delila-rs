// test_sink_core.cpp — ROOT/ZMQ-free unit tests for the root_sink pure logic.
//
// Covers sink_core.hpp (envelope parse, batch decode, coincidence matcher with
// energies, run state, HTTP helpers) and hist_config.hpp (config parse + errors,
// value_of / pass_cut for both scopes). Builds and runs on any box:
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
#include <string>
#include <vector>

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

  std::printf("\n%d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

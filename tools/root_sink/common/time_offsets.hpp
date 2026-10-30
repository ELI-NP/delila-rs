// time_offsets.hpp — per-(module, channel) time offsets for root_sink (TODO 70).
//
// What it is: a constant propagation delay per digitizer channel (cable, delay
// line, electronics) that every consumer must remove so they all work on ONE
// time axis. root_sink applies it once, in the Receiver, right after --module
// filtering and before the Sorter, the display tee, the built-event builder and
// the hits tree:  aligned_ts = raw_ts - offset_ns  (positive = the channel
// arrives LATE). That is exactly how the Rust event builder applies the same
// file at its input stage (SPEC TODO/event-builder/SPECIFICATION.md §4.3.1:
// "apply once at input, frozen per run"; src/event_builder/hit.rs).
//
// What it is NOT:
//   * not a decoder-layer correction — the .delila keeps the raw digitizer time
//     (docs/plans/rollover_tracker_design.md, "layer responsibility" rule);
//   * not an analysis constant such as the mm calibration or the -70 ns TOF
//     display offset, which stay in macros (eb_core.hpp). Note that moving a
//     TRIGGER or LaBr3 channel's offset shifts TOF, so such macro constants
//     must then be re-derived.
//
// File format = the Rust EB's timeSettings.json (src/event_builder/
// time_offsets.rs, schemas/timeSettings.schema.json), tree form:
//   {"version":"1.0","entries":[{"module":0,"channel":1,"ref":null,"offset_ns":0},
//                               {"module":0,"channel":2,"ref":[0,1],"offset_ns":8.7}]}
// absolute offset = sum of offset_ns along the path to the root, the root's own
// value included. The legacy `event_builder time-calib` output
//   {"ref_module":0,"ref_channel":0,"offsets":{"00_02":8.7}}
// (src/event_builder/config.rs) is accepted too; its values are already absolute.
// The form is chosen by which key is present — never by "try one, fall back to
// the other", which would hide the real error of a broken tree file.
//
// Pure C++17 + TDelila.hpp's JSON parser: no ROOT, no ZMQ, unit-tested in
// test_sink_core.cpp (and test_eb_core.cpp for the Sorter contract).
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef ROOTSINK_TIME_OFFSETS_HPP
#define ROOTSINK_TIME_OFFSETS_HPP

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <map>
#include <set>
#include <string>
#include <vector>

#include "sink_core.hpp"  // ScalarHit, tdelila::json

namespace rootsink {

// (module << 8) | channel — the same key as Rust's make_channel_key (hit.rs).
inline uint16_t mc_key(uint8_t module, uint8_t channel) {
  return static_cast<uint16_t>((static_cast<unsigned>(module) << 8) | channel);
}

inline std::string mc_name(uint16_t key) {
  char buf[24];
  std::snprintf(buf, sizeof buf, "m%uc%u", static_cast<unsigned>(key >> 8),
                static_cast<unsigned>(key & 0xFF));
  return buf;
}

// Resolved table: ABSOLUTE offset per (module, channel).
struct OffsetTable {
  std::map<uint16_t, double> abs_ns;
  std::string format;       // "tree" | "legacy"
  int roots = 0;            // tree: entries with ref == null; legacy: 1
  int max_depth = 0;        // tree: longest path to a root; legacy: 0
  std::vector<std::string> warnings;

  // max - min of the absolute offsets: aligning adds up to this much arrival
  // disorder to the stream, so it must stay below the Sorter's safe horizon.
  double spread_ns() const {
    if (abs_ns.size() < 2) return 0.0;
    double lo = abs_ns.begin()->second, hi = lo;
    for (const auto& kv : abs_ns) {
      if (kv.second < lo) lo = kv.second;
      if (kv.second > hi) hi = kv.second;
    }
    return hi - lo;
  }
};

struct OffsetParseResult {
  OffsetTable table;
  std::vector<std::string> errors;  // non-empty => table must not be used
};

namespace toff_detail {

using JV = tdelila::json::Value;

inline bool get_u8(const JV* v, int& out) {
  if (!v || v->t != JV::T::Num || !std::isfinite(v->num)) return false;
  if (v->num != std::floor(v->num) || v->num < 0 || v->num > 255) return false;
  out = static_cast<int>(v->num);
  return true;
}

inline bool comment_ok(const JV& v) {
  if (v.t == JV::T::Str) return true;
  if (v.t != JV::T::Arr) return false;
  for (const JV& e : v.arr)
    if (e.t != JV::T::Str) return false;
  return true;
}

// Check the top-level keys against an allow-list; _comment / $schema are
// accepted everywhere (repo convention, ignored by Rust serde).
inline void check_top_keys(const JV& root, std::initializer_list<const char*> allowed,
                           std::vector<std::string>& errors) {
  for (const auto& kv : root.obj) {
    const std::string& k = kv.first;
    if (k == "$schema") continue;
    if (k == "_comment") {
      if (!comment_ok(kv.second))
        errors.push_back("'_comment' must be a string or an array of strings");
      continue;
    }
    bool ok = false;
    for (const char* a : allowed)
      if (k == a) ok = true;
    if (!ok) errors.push_back("unknown top-level key '" + k + "'");
  }
}

inline void parse_tree(const JV& root, OffsetParseResult& r) {
  std::vector<std::string>& errors = r.errors;
  OffsetTable& t = r.table;
  t.format = "tree";
  check_top_keys(root, {"version", "entries"}, errors);
  const JV* ver = root.find("version");
  if (!ver || ver->t != JV::T::Str)
    errors.push_back("missing or non-string 'version' (expected \"1.0\")");
  else if (ver->str != "1.0")
    t.warnings.push_back("version \"" + ver->str + "\" (this reader knows \"1.0\")");
  const JV* entries = root.find("entries");
  if (!entries || entries->t != JV::T::Arr) {
    errors.push_back("'entries' must be an array");
    return;
  }

  struct Node {
    int parent = -1;  // key of the parent, -1 = root
    double offset = 0.0;
    std::size_t idx = 0;
  };
  std::map<int, Node> nodes;
  for (std::size_t i = 0; i < entries->arr.size(); ++i) {
    const JV& e = entries->arr[i];
    const std::string where = "entries[" + std::to_string(i) + "]";
    if (e.t != JV::T::Obj) {
      errors.push_back(where + ": not an object");
      continue;
    }
    std::size_t before = errors.size();
    for (const auto& kv : e.obj)
      if (kv.first != "module" && kv.first != "channel" && kv.first != "ref" &&
          kv.first != "offset_ns")
        errors.push_back(where + ": unknown key '" + kv.first + "'");
    int m = 0, c = 0;
    if (!get_u8(e.find("module"), m)) errors.push_back(where + ": 'module' must be an integer 0..255");
    if (!get_u8(e.find("channel"), c)) errors.push_back(where + ": 'channel' must be an integer 0..255");
    const JV* ref = e.find("ref");
    int parent = -1;
    if (!ref) {
      errors.push_back(where + ": missing 'ref' (null for a root, [module, channel] otherwise)");
    } else if (ref->t != JV::T::Null) {
      int pm = 0, pc = 0;
      if (ref->t != JV::T::Arr || ref->arr.size() != 2 || !get_u8(&ref->arr[0], pm) ||
          !get_u8(&ref->arr[1], pc))
        errors.push_back(where + ": 'ref' must be null or [module, channel] with integers 0..255");
      else
        parent = mc_key(static_cast<uint8_t>(pm), static_cast<uint8_t>(pc));
    }
    const JV* off = e.find("offset_ns");
    if (!off || off->t != JV::T::Num || !std::isfinite(off->num))
      errors.push_back(where + ": 'offset_ns' must be a finite number");
    if (errors.size() != before) continue;  // this entry is unusable
    int key = mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(c));
    auto it = nodes.find(key);
    if (it != nodes.end()) {
      errors.push_back("entries[" + std::to_string(it->second.idx) + "] and " + where +
                       ": duplicate " + mc_name(static_cast<uint16_t>(key)));
      continue;
    }
    nodes[key] = Node{parent, off->num, i};
  }
  if (!errors.empty()) return;

  // Dangling refs.
  for (const auto& kv : nodes)
    if (kv.second.parent >= 0 && nodes.find(kv.second.parent) == nodes.end())
      errors.push_back("entries[" + std::to_string(kv.second.idx) + "] (" +
                       mc_name(static_cast<uint16_t>(kv.first)) + "): ref " +
                       mc_name(static_cast<uint16_t>(kv.second.parent)) +
                       " is not an entry (missing parent)");
  if (!errors.empty()) return;

  // Resolve: walk each key up to its root, summing offsets (root included).
  std::set<int> roots, cycle_reported;
  for (const auto& kv : nodes) {
    std::set<int> seen;
    std::vector<int> path;
    double sum = 0.0;
    int k = kv.first;
    bool cyc = false;
    while (true) {
      if (!seen.insert(k).second) {
        cyc = true;
        break;
      }
      path.push_back(k);
      const Node& n = nodes[k];
      sum += n.offset;
      if (n.parent < 0) break;
      k = n.parent;
    }
    if (cyc) {
      // Report each cycle once, keyed by its smallest member.
      int lo = k;
      for (std::size_t i = 0; i < path.size(); ++i)
        if (path[i] == k) {
          for (std::size_t j = i; j < path.size(); ++j)
            if (path[j] < lo) lo = path[j];
          break;
        }
      if (cycle_reported.insert(lo).second) {
        std::string s = "cycle in 'ref' chain";
        if (nodes[k].parent == k) s += " (self-reference)";
        s += ": ";
        bool in = false;
        for (int p : path) {
          if (p == k) in = true;
          if (in) s += mc_name(static_cast<uint16_t>(p)) + " -> ";
        }
        s += mc_name(static_cast<uint16_t>(k));
        errors.push_back(s);
      }
      continue;
    }
    roots.insert(path.back());
    int depth = static_cast<int>(path.size()) - 1;
    if (depth > t.max_depth) t.max_depth = depth;
    t.abs_ns[static_cast<uint16_t>(kv.first)] = sum;
  }
  if (!errors.empty()) {
    t.abs_ns.clear();
    return;
  }
  t.roots = static_cast<int>(roots.size());
  if (t.roots > 1) {
    std::string s = std::to_string(t.roots) + " roots (disconnected timing domains:";
    for (int k : roots) s += " " + mc_name(static_cast<uint16_t>(k));
    t.warnings.push_back(s + ")");
  }
  if (t.max_depth > 5)
    t.warnings.push_back("tree depth " + std::to_string(t.max_depth) +
                         " > 5 (offset errors accumulate along long ref chains)");
}

// "M_C" with 1..3 decimal digits each side, both 0..255.
inline bool parse_legacy_key(const std::string& s, int& m, int& c) {
  std::size_t u = s.find('_');
  if (u == std::string::npos || s.find('_', u + 1) != std::string::npos) return false;
  auto num = [](const std::string& p, int& out) {
    if (p.empty() || p.size() > 3) return false;
    for (char ch : p)
      if (ch < '0' || ch > '9') return false;
    out = std::stoi(p);
    return out <= 255;
  };
  return num(s.substr(0, u), m) && num(s.substr(u + 1), c);
}

inline void parse_legacy(const JV& root, OffsetParseResult& r) {
  std::vector<std::string>& errors = r.errors;
  OffsetTable& t = r.table;
  t.format = "legacy";
  t.roots = 1;
  t.max_depth = 0;
  check_top_keys(root, {"ref_module", "ref_channel", "offsets"}, errors);
  int rm = 0, rc = 0;
  if (!get_u8(root.find("ref_module"), rm)) errors.push_back("'ref_module' must be an integer 0..255");
  if (!get_u8(root.find("ref_channel"), rc)) errors.push_back("'ref_channel' must be an integer 0..255");
  const JV* offs = root.find("offsets");
  if (!offs || offs->t != JV::T::Obj) {
    errors.push_back("'offsets' must be an object of \"M_C\": offset_ns");
    return;
  }
  std::map<int, std::string> seen;
  for (const auto& kv : offs->obj) {
    int m = 0, c = 0;
    if (!parse_legacy_key(kv.first, m, c)) {
      errors.push_back("offsets key '" + kv.first + "' is not \"M_C\" (module_channel, 0..255)");
      continue;
    }
    if (kv.second.t != JV::T::Num || !std::isfinite(kv.second.num)) {
      errors.push_back("offsets['" + kv.first + "'] must be a finite number");
      continue;
    }
    int key = mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(c));
    auto it = seen.find(key);
    if (it != seen.end()) {
      errors.push_back("offsets keys '" + it->second + "' and '" + kv.first + "': duplicate " +
                       mc_name(static_cast<uint16_t>(key)));
      continue;
    }
    seen[key] = kv.first;
    t.abs_ns[static_cast<uint16_t>(key)] = kv.second.num;
  }
  if (!errors.empty()) t.abs_ns.clear();
}

}  // namespace toff_detail

// Parse + validate + resolve. Collects EVERY error (an operator fixing a file
// should see all problems at once); on any error the table is empty and unusable.
inline OffsetParseResult parse_time_offsets(const std::string& json_text) {
  OffsetParseResult r;
  if (json_text.find_first_not_of(" \t\r\n") == std::string::npos) {
    r.errors.push_back("empty file (expected a timeSettings.json object)");
    return r;
  }
  toff_detail::JV root;
  try {
    root = tdelila::json::Parser(json_text).parse();
  } catch (const std::exception& e) {
    r.errors.push_back(std::string("malformed JSON (") + e.what() + ")");
    return r;
  }
  if (root.t != toff_detail::JV::T::Obj) {
    r.errors.push_back("top level must be a JSON object");
    return r;
  }
  bool tree = root.find("entries") != nullptr;
  bool legacy = root.find("offsets") != nullptr;
  if (tree && legacy) {
    r.errors.push_back("ambiguous: both 'entries' (timeSettings tree form) and 'offsets' "
                       "(legacy time-calib form) present");
  } else if (tree) {
    toff_detail::parse_tree(root, r);
  } else if (legacy) {
    toff_detail::parse_legacy(root, r);
  } else {
    r.errors.push_back("neither 'entries' (timeSettings tree form) nor 'offsets' "
                       "(legacy time-calib form) present");
  }
  return r;
}

// Hot path: one map lookup per hit (a handful of entries). A hit whose channel
// has no entry keeps its raw time (Rust get_or_zero); `on_missing(key)` fires
// once per key per run — the caller clears `reported` at RunOpen.
template <class MissingF>
inline void apply_time_offsets(std::vector<ScalarHit>& hits, const OffsetTable& t,
                               std::set<uint16_t>& reported, MissingF on_missing) {
  for (ScalarHit& h : hits) {
    uint16_t k = mc_key(h.module, h.channel);
    auto it = t.abs_ns.find(k);
    if (it != t.abs_ns.end())
      h.timestamp_ns -= it->second;
    else if (reported.insert(k).second)
      on_missing(k);
  }
}

// The table in force, owned by the Receiver.
struct ActiveOffsets {
  OffsetTable table;
  std::string source_text;  // verbatim file actually applied
  std::string source_path;
  std::string loaded_at;    // UTC ISO time of the successful load
  bool loaded = false;      // a valid table has been adopted at least once
  bool stale = false;       // the latest reload failed; previous table kept
  std::string stale_error;
};

// Run-boundary reload policy. `text == nullptr` means the file could not be read
// (`read_error` says why). On success the new table is adopted and any stale
// flag cleared (returns true). On ANY failure — unreadable, invalid, or a spread
// >= `max_spread_ns` (the Sorter's safe horizon: aligning would push hits behind
// already-emitted chunk cores, i.e. drop them as late) — the previous table and
// text are KEPT, `stale` is set, and the errors are returned (returns false).
// Never falls back to "no offsets" silently.
inline bool reload_offsets(ActiveOffsets& a, const std::string* text,
                           const std::string& read_error, const std::string& path,
                           double max_spread_ns, const std::string& now_utc,
                           std::vector<std::string>& errors) {
  errors.clear();
  OffsetParseResult r;
  if (!text) {
    errors.push_back("cannot read " + path + ": " + read_error);
  } else {
    r = parse_time_offsets(*text);
    errors = r.errors;
    if (errors.empty() && r.table.spread_ns() >= max_spread_ns) {
      char buf[160];
      std::snprintf(buf, sizeof buf,
                    "offset spread %.3f ns >= safe horizon %.3f ns: aligned hits would "
                    "fall behind emitted chunks (late)",
                    r.table.spread_ns(), max_spread_ns);
      errors.push_back(buf);
    }
  }
  if (!errors.empty()) {
    a.stale = true;
    a.stale_error.clear();
    for (std::size_t i = 0; i < errors.size(); ++i)
      a.stale_error += (i ? "; " : "") + errors[i];
    return false;
  }
  a.table = std::move(r.table);
  a.source_text = *text;
  a.source_path = path;
  a.loaded_at = now_utc;
  a.loaded = true;
  a.stale = false;
  a.stale_error.clear();
  return true;
}

inline std::string json_escape(const std::string& s) {
  std::string out;
  out.reserve(s.size() + 8);
  for (char ch : s) {
    switch (ch) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      case '\b': out += "\\b"; break;
      case '\f': out += "\\f"; break;
      default:
        if (static_cast<unsigned char>(ch) < 0x20) {
          char buf[8];
          std::snprintf(buf, sizeof buf, "\\u%04x", static_cast<unsigned>(ch));
          out += buf;
        } else {
          out += ch;
        }
    }
  }
  return out;
}

// "m0c1=+0.000 m0c2=+8.700 ..." for the per-run log line.
inline std::string format_offsets_line(const OffsetTable& t) {
  std::string s;
  for (const auto& kv : t.abs_ns) {
    char buf[48];
    std::snprintf(buf, sizeof buf, "%s=%+.3f", mc_name(kv.first).c_str(), kv.second);
    if (!s.empty()) s += ' ';
    s += buf;
  }
  return s;
}

// Provenance stored with every run (TTree UserInfo): what was applied, from
// where, and the verbatim source. Offsets use %.17g so they round-trip exactly.
inline std::string offsets_provenance_json(const ActiveOffsets& a) {
  std::string s = "{\"applied\":true,";
  s += "\"convention\":\"aligned_ts = raw_ts - offset_ns (positive = channel arrives late); "
       "hits-tree timestamp_ns and all built-tree times are ALIGNED; the .delila is raw\",";
  s += "\"source_path\":\"" + json_escape(a.source_path) + "\",";
  s += "\"format\":\"" + json_escape(a.table.format) + "\",";
  s += "\"loaded_at\":\"" + json_escape(a.loaded_at) + "\",";
  s += std::string("\"status\":\"") +
       (a.stale ? "stale: run-start reload failed, previous table kept" : "ok") + "\",";
  if (a.stale) s += "\"reload_error\":\"" + json_escape(a.stale_error) + "\",";
  s += "\"roots\":" + std::to_string(a.table.roots) + ",";
  s += "\"max_depth\":" + std::to_string(a.table.max_depth) + ",";
  s += "\"warnings\":[";
  for (std::size_t i = 0; i < a.table.warnings.size(); ++i)
    s += (i ? ",\"" : "\"") + json_escape(a.table.warnings[i]) + "\"";
  s += "],\"resolved_abs_ns\":[";
  bool first = true;
  for (const auto& kv : a.table.abs_ns) {
    char buf[96];
    std::snprintf(buf, sizeof buf, "{\"module\":%u,\"channel\":%u,\"offset_ns\":%.17g}",
                  static_cast<unsigned>(kv.first >> 8), static_cast<unsigned>(kv.first & 0xFF),
                  kv.second);
    if (!first) s += ',';
    s += buf;
    first = false;
  }
  s += "],\"source_text\":\"" + json_escape(a.source_text) + "\"}";
  return s;
}

inline std::string offsets_disabled_json() {
  return "{\"applied\":false,\"convention\":\"no time offsets: timestamp_ns is the RAW "
         "digitizer time, as in the .delila\"}";
}

}  // namespace rootsink

#endif  // ROOTSINK_TIME_OFFSETS_HPP

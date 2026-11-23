// thgem_map.hpp — the channel map as thgem_sink reads it (TODO 88 §3.2).
//
// The same file format as N_Wall's (nwall_map.hpp): ONE map per experiment,
// shared by Grafana, the sink and the offline analysis, e.g.
// config/channel_map_11B_208Pb.txt, whitespace separated, '#' comments:
//
//   module  channel  name  group  det  kind  index  [more columns...]
//
// Columns 1-4 are the Grafana map (tools/grafana/gen_dashboards.py, TODO 74
// §3.2), which ignores columns 5+. thgem_sink reads columns 5-7, the ROLE of
// the channel, and ignores columns 8+ (left to the analysis):
//
//   det    beam | LaBr | LS | pulser | ThGEM | -
//   kind   for ThGEM the electrode: T (trigger) | XL | XR | YU | YD; else -
//   index  for ThGEM its number (0..99, the display names it T<n>); else -
//
// A line with only the four Grafana columns has no role. Roles are read from
// the map, never assumed from channel numbers.
//
// Errors (the map is then not used) are collected with their line numbers:
// unknown det / kind, missing index, a channel listed twice, a second beam
// channel, the same ThGEM electrode twice. Warnings (the map is used) are
// about ThGEMs that cannot show everything: no trigger (nothing of it is
// shown), a missing arm (no XY), electrodes on more than one board (x and y
// then contain the board-to-board offsets). Pure C++17, no ROOT: unit-tested
// in test_thgem_map.cpp.
//
// Separate from nwall_map.hpp on purpose (one program per experiment, TODO 79
// §5.1): nwall_sink is not touched.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef THGEM_MAP_HPP
#define THGEM_MAP_HPP

#include <cstdint>
#include <cstdio>
#include <map>
#include <set>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

namespace thgem {

enum class Det : uint8_t { None, Beam, LaBr, LS, Pulser, ThGEM };

// The five electrodes of one ThGEM, in this order everywhere (arrays index by
// it): the trigger, then the delay-line ends. x = t_XR - t_XL, y = t_YU - t_YD.
enum class Kind : uint8_t { T = 0, XL = 1, XR = 2, YU = 3, YD = 4, None = 5 };
constexpr int kElectrodes = 5;

inline const char* det_name(Det d) {
  switch (d) {
    case Det::Beam: return "beam";
    case Det::LaBr: return "LaBr";
    case Det::LS: return "LS";
    case Det::Pulser: return "pulser";
    case Det::ThGEM: return "ThGEM";
    case Det::None: break;
  }
  return "-";
}

inline const char* kind_name(Kind k) {
  static const char* names[] = {"T", "XL", "XR", "YU", "YD", "-"};
  return names[static_cast<int>(k)];
}

struct Role {
  Det det = Det::None;
  Kind kind = Kind::None;
  int index = -1;  // ThGEM number, else -1
};

inline uint16_t map_key(unsigned module, unsigned channel) {
  return static_cast<uint16_t>((module << 8) | channel);
}

// The channels of one ThGEM: key[k] = map_key of electrode k, -1 = not in the map.
struct Thgem {
  int number = -1;
  int key[kElectrodes] = {-1, -1, -1, -1, -1};

  bool has(Kind k) const { return key[static_cast<int>(k)] >= 0; }
  bool has_trigger() const { return has(Kind::T); }
  bool complete() const {
    for (int k = 0; k < kElectrodes; ++k)
      if (key[k] < 0) return false;
    return true;
  }
};

struct ChannelMap {
  std::map<uint16_t, Role> roles;         // only channels that have a role
  std::map<uint16_t, std::string> names;  // every listed channel (column 3)
  std::map<int, Thgem> thgems;            // by ThGEM number
  int beam_key = -1;                      // map_key of the beam channel, -1 = none

  const Role* role(unsigned module, unsigned channel) const {
    auto it = roles.find(map_key(module, channel));
    return it == roles.end() ? nullptr : &it->second;
  }
  std::size_t count(Det d) const {
    std::size_t n = 0;
    for (const auto& kv : roles)
      if (kv.second.det == d) ++n;
    return n;
  }
};

struct MapParseResult {
  ChannelMap map;
  std::vector<std::string> errors;    // the map must not be used
  std::vector<std::string> warnings;  // the map is used; something will not be shown
};

namespace map_detail {

inline bool parse_uint(const std::string& s, int max, int& out) {
  if (s.empty() || s.size() > 6) return false;
  int v = 0;
  for (char c : s) {
    if (c < '0' || c > '9') return false;
    v = v * 10 + (c - '0');
  }
  if (v > max) return false;
  out = v;
  return true;
}

inline bool parse_det(const std::string& s, Det& d) {
  static const std::pair<const char*, Det> table[] = {
      {"beam", Det::Beam},     {"LaBr", Det::LaBr},   {"LS", Det::LS},
      {"pulser", Det::Pulser}, {"ThGEM", Det::ThGEM}, {"-", Det::None}};
  for (const auto& t : table)
    if (s == t.first) {
      d = t.second;
      return true;
    }
  return false;
}

inline bool parse_kind(const std::string& s, Kind& k) {
  static const std::pair<const char*, Kind> table[] = {{"T", Kind::T},   {"XL", Kind::XL},
                                                       {"XR", Kind::XR}, {"YU", Kind::YU},
                                                       {"YD", Kind::YD}, {"-", Kind::None}};
  for (const auto& t : table)
    if (s == t.first) {
      k = t.second;
      return true;
    }
  return false;
}

inline std::string mc(uint16_t key) {
  return "module " + std::to_string(key >> 8) + " channel " + std::to_string(key & 0xFF);
}

// Warnings for ThGEMs that cannot show everything (the map is still used).
inline void check_thgems(MapParseResult& r) {
  for (const auto& kv : r.map.thgems) {
    const Thgem& g = kv.second;
    const std::string who = "ThGEM " + std::to_string(g.number);
    if (!g.has_trigger()) {
      r.warnings.push_back(who + " has no trigger (kind T): nothing of it is shown");
      continue;
    }
    std::string missing;
    for (int k = 1; k < kElectrodes; ++k)
      if (g.key[k] < 0) missing += (missing.empty() ? "" : " ") + std::string(kind_name(Kind(k)));
    if (!missing.empty())
      r.warnings.push_back(who + " has no " + missing + ": its XY image is not made");
    std::set<int> boards;
    for (int k = 0; k < kElectrodes; ++k)
      if (g.key[k] >= 0) boards.insert(g.key[k] >> 8);
    if (boards.size() > 1) {
      std::string list;
      for (int b : boards) list += (list.empty() ? "" : ",") + std::to_string(b);
      r.warnings.push_back(who + " is spread over modules " + list +
                           ": its x and y then include the board-to-board time offsets "
                           "(keep one ThGEM on one board, TODO 88 §2)");
    }
  }
}

}  // namespace map_detail

// Parse the whole map; `source` names the file in the messages.
inline MapParseResult parse_channel_map(const std::string& text,
                                        const std::string& source = "<map>") {
  using namespace map_detail;
  MapParseResult r;
  std::istringstream in(text);
  std::string raw;
  std::map<uint16_t, int> first_line;
  std::map<std::pair<int, int>, int> electrode_line;  // (ThGEM number, kind) -> line
  int beam_line = 0;
  int lineno = 0;
  while (std::getline(in, raw)) {
    ++lineno;
    const std::string where = source + ":" + std::to_string(lineno);
    std::string line = raw.substr(0, raw.find('#'));
    std::istringstream cols(line);
    std::vector<std::string> c;
    for (std::string tok; cols >> tok;) c.push_back(tok);
    if (c.empty()) continue;
    if (c.size() < 4) {
      r.errors.push_back(where + ": expected 'module channel name group [det kind index]', got " +
                         std::to_string(c.size()) + " column(s)");
      continue;
    }
    int module = 0, channel = 0;
    bool bad = false;
    if (!parse_uint(c[0], 255, module)) {
      r.errors.push_back(where + ": module '" + c[0] + "' is not an integer 0..255");
      bad = true;
    }
    if (!parse_uint(c[1], 255, channel)) {
      r.errors.push_back(where + ": channel '" + c[1] + "' is not an integer 0..255");
      bad = true;
    }
    if (bad) continue;
    const uint16_t key = map_key(static_cast<unsigned>(module), static_cast<unsigned>(channel));
    auto dup = first_line.find(key);
    if (dup != first_line.end()) {
      r.errors.push_back(where + ": module " + c[0] + " channel " + c[1] +
                         " is already listed on line " + std::to_string(dup->second));
      continue;
    }
    first_line[key] = lineno;
    r.map.names[key] = c[2];
    if (c.size() == 4) continue;  // Grafana-only line: no role
    if (c.size() < 7) {
      r.errors.push_back(where + ": a role needs three columns 'det kind index' (use '-' for "
                         "none), got " + std::to_string(c.size() - 4));
      continue;
    }
    Role role;
    if (!parse_det(c[4], role.det)) {
      r.errors.push_back(where + ": det '" + c[4] + "' is not one of beam LaBr LS pulser ThGEM -");
      bad = true;
    }
    if (!parse_kind(c[5], role.kind)) {
      r.errors.push_back(where + ": kind '" + c[5] + "' is not one of T XL XR YU YD -");
      bad = true;
    }
    if (c[6] != "-" && !parse_uint(c[6], 99, role.index)) {
      r.errors.push_back(where + ": index '" + c[6] + "' is not an integer 0..99 or '-'");
      bad = true;
    }
    if (bad) continue;
    const bool is_thgem = role.det == Det::ThGEM;
    if (is_thgem && (role.kind == Kind::None || role.index < 0)) {
      r.errors.push_back(where + ": det ThGEM needs kind T, XL, XR, YU or YD and its number");
      continue;
    }
    if (!is_thgem && (role.kind != Kind::None || role.index >= 0)) {
      r.errors.push_back(where + ": det " + c[4] + " takes kind '-' and index '-'");
      continue;
    }
    if (role.det == Det::None) continue;
    if (role.det == Det::Beam) {
      if (r.map.beam_key >= 0) {
        r.errors.push_back(where + ": a second beam channel (the first is on line " +
                           std::to_string(beam_line) + ") — the bunch clock uses exactly one");
        continue;
      }
      r.map.beam_key = key;
      beam_line = lineno;
    }
    if (is_thgem) {
      const auto ek = std::make_pair(role.index, static_cast<int>(role.kind));
      auto prev = electrode_line.find(ek);
      if (prev != electrode_line.end()) {
        r.errors.push_back(where + ": ThGEM " + c[6] + " " + c[5] + " is already on line " +
                           std::to_string(prev->second));
        continue;
      }
      electrode_line[ek] = lineno;
      Thgem& g = r.map.thgems[role.index];
      g.number = role.index;
      g.key[static_cast<int>(role.kind)] = key;
    }
    r.map.roles[key] = role;
  }
  check_thgems(r);
  return r;
}

// Display histogram names. They go by ThGEM number and by board / channel,
// never by the map's channel names: histograms are kept across runs while the
// map is re-read at every run start, so a name must keep meaning one thing.
inline std::string xy_name(int number) {
  char buf[32];
  std::snprintf(buf, sizeof buf, "xy_T%d", number);
  return buf;
}

// The pair histogram of ThGEMs i < j: dt_j on y against dt_i on x, the
// 8-detector form of October's dt2_vs_dt1 (TODO 65).
inline std::string dt_pair_name(int i, int j) {
  char buf[48];
  std::snprintf(buf, sizeof buf, "dt_T%d_vs_T%d", j, i);
  return buf;
}

inline std::string channel_hist_name(const char* stem, unsigned module, unsigned channel) {
  char buf[128];
  std::snprintf(buf, sizeof buf, "%s_m%u_ch%02u", stem, module, channel);
  return buf;
}

// ThGEM numbers that have a trigger, ascending.
inline std::vector<int> triggered_thgems(const ChannelMap& m) {
  std::vector<int> out;
  for (const auto& kv : m.thgems)
    if (kv.second.has_trigger()) out.push_back(kv.first);
  return out;
}

// Every pair (i < j) of ThGEMs with a trigger: the display makes one dt pair
// histogram per pair up front, so all are listed before any data.
inline std::vector<std::pair<int, int>> thgem_pairs(const ChannelMap& m) {
  const std::vector<int> n = triggered_thgems(m);
  std::vector<std::pair<int, int>> out;
  for (std::size_t a = 0; a < n.size(); ++a)
    for (std::size_t b = a + 1; b < n.size(); ++b) out.emplace_back(n[a], n[b]);
  return out;
}

// The map keys (module << 8 | channel) of one detector type, sorted.
inline std::vector<uint16_t> keys_of(const ChannelMap& m, Det d) {
  std::vector<uint16_t> out;
  for (const auto& kv : m.roles)
    if (kv.second.det == d) out.push_back(kv.first);
  return out;  // std::map iterates in key order
}

// The map and the event definition both name the ThGEM triggers (two files the
// user keeps in step). Warnings for every disagreement, never silent: a ThGEM
// trigger the definition does not trigger on never opens an event (its ThGEM
// only shows up next to another one), and a definition trigger that is no
// ThGEM trigger in the map opens events the /thgem display cannot place.
// `def_triggers` = every trigger channel of the definition, map_key encoding.
inline std::vector<std::string> trigger_mismatch(const ChannelMap& m,
                                                 const std::set<uint16_t>& def_triggers) {
  std::vector<std::string> out;
  std::set<uint16_t> map_triggers;
  for (const auto& kv : m.thgems) {
    if (!kv.second.has_trigger()) continue;
    const uint16_t k = static_cast<uint16_t>(kv.second.key[static_cast<int>(Kind::T)]);
    map_triggers.insert(k);
    if (!def_triggers.count(k))
      out.push_back("ThGEM " + std::to_string(kv.first) + " trigger (" + map_detail::mc(k) +
                    ") is not a trigger of the event definition: it never opens an event");
  }
  for (uint16_t k : def_triggers)
    if (!map_triggers.count(k))
      out.push_back("event-definition trigger " + map_detail::mc(k) +
                    " is not a ThGEM trigger in the channel map");
  return out;
}

}  // namespace thgem

#endif  // THGEM_MAP_HPP

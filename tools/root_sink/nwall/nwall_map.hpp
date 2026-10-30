// nwall_map.hpp — the N_Wall channel map as nwall_sink reads it (TODO 79 §2).
//
// ONE map file per experiment is shared by Grafana, nwall_sink and the offline
// analysis: config/channel_map_N_Wall.txt, whitespace separated, '#' comments.
//
//   module  channel  name  group  det  kind  index  [more columns...]
//
// Columns 1-4 are the Grafana map (tools/grafana/gen_dashboards.py, TODO 74
// §3.2); that generator ignores columns 5+. nwall_sink reads columns 5-7, the
// detector ROLE of the channel, and ignores columns 8+ (left to the analysis):
//
//   det    beam | LaBr | LS | dE | E | pulser | -
//   kind   ring | sector | -        (dE and E need ring or sector, the rest -)
//   index  ring / sector number (>= 0) for dE and E, '-' for the rest
//
// A line with only the four Grafana columns has no role (the sink shows the
// channel in the per-board histograms only). Roles are read from the map, never
// assumed from channel numbers: the VX2730 assignment (beam 0, LaBr3 1-7, LS
// 8-31, 2026-10-30) may still change.
//
// Every error is collected with its line number (unknown det / kind, missing
// index, duplicate channel, more than one beam channel, ...). Pure C++17, no
// ROOT: unit-tested in test_nwall.cpp.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef NWALL_MAP_HPP
#define NWALL_MAP_HPP

#include <cstdint>
#include <map>
#include <sstream>
#include <string>
#include <vector>

namespace nwall {

enum class Det : uint8_t { None, Beam, LaBr, LS, DE, E, Pulser };
enum class Kind : uint8_t { None, Ring, Sector };

inline const char* det_name(Det d) {
  switch (d) {
    case Det::Beam: return "beam";
    case Det::LaBr: return "LaBr";
    case Det::LS: return "LS";
    case Det::DE: return "dE";
    case Det::E: return "E";
    case Det::Pulser: return "pulser";
    case Det::None: break;
  }
  return "-";
}

struct Role {
  Det det = Det::None;
  Kind kind = Kind::None;
  int index = -1;  // ring / sector number for dE and E, else -1
};

inline uint16_t map_key(unsigned module, unsigned channel) {
  return static_cast<uint16_t>((module << 8) | channel);
}

struct ChannelMap {
  std::map<uint16_t, Role> roles;         // only channels that have a role
  std::map<uint16_t, std::string> names;  // every listed channel (column 3)
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
  std::vector<std::string> errors;
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
      {"beam", Det::Beam}, {"LaBr", Det::LaBr}, {"LS", Det::LS},     {"dE", Det::DE},
      {"E", Det::E},       {"pulser", Det::Pulser}, {"-", Det::None}};
  for (const auto& t : table)
    if (s == t.first) {
      d = t.second;
      return true;
    }
  return false;
}

inline bool parse_kind(const std::string& s, Kind& k) {
  if (s == "ring") k = Kind::Ring;
  else if (s == "sector") k = Kind::Sector;
  else if (s == "-") k = Kind::None;
  else return false;
  return true;
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
      r.errors.push_back(where + ": det '" + c[4] + "' is not one of beam LaBr LS dE E pulser -");
      bad = true;
    }
    if (!parse_kind(c[5], role.kind)) {
      r.errors.push_back(where + ": kind '" + c[5] + "' is not one of ring sector -");
      bad = true;
    }
    if (c[6] != "-" && !parse_uint(c[6], 9999, role.index)) {
      r.errors.push_back(where + ": index '" + c[6] + "' is not a non-negative integer or '-'");
      bad = true;
    }
    if (bad) continue;
    const bool silicon = role.det == Det::DE || role.det == Det::E;
    if (silicon && (role.kind == Kind::None || role.index < 0)) {
      r.errors.push_back(where + ": det " + c[4] + " needs kind ring or sector and an index");
      continue;
    }
    if (!silicon && (role.kind != Kind::None || role.index >= 0)) {
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
    r.map.roles[key] = role;
  }
  return r;
}

}  // namespace nwall

#endif  // NWALL_MAP_HPP

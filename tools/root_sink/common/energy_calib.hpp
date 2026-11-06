// energy_calib.hpp — energy calibration for the DISPLAY (TODO 83):
//
//   E_keV = kev_per_adc * ADC + offset_kev
//
// The coefficients are worked out outside delila-rs and arrive as a table of four
// columns, one line per channel, '#' starting a comment:
//
//   module channel kev_per_adc offset_kev
//
// Only the live histograms use this: .delila and .root keep raw ADC whatever the
// file says, so a bad file never stops anything. The previous table stays in force
// (STALE), and a channel not listed simply has no keV histogram.
//
// No ROOT. src/monitor/energy_calib.rs reads the same format; both are tested on
// the files in fixtures/energy_calib/.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef DELILA_ENERGY_CALIB_HPP
#define DELILA_ENERGY_CALIB_HPP

#include <cerrno>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <map>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

namespace ecal {

struct LinearCalib {
  double kev_per_adc = 1.0;
  double offset_kev = 0.0;
  double to_kev(double adc) const { return kev_per_adc * adc + offset_kev; }
  bool operator==(const LinearCalib& o) const {
    return kev_per_adc == o.kev_per_adc && offset_kev == o.offset_kev;
  }
};

inline uint16_t calib_key(unsigned module, unsigned channel) {
  return static_cast<uint16_t>((module << 8) | channel);
}

// A channel not listed has no coefficients.
using CalibTable = std::map<uint16_t, LinearCalib>;

struct CalibParseResult {
  CalibTable table;
  std::vector<std::string> errors;
};

namespace detail {

// Digits only, 0..255.
inline bool parse_id(const std::string& s, int& out) {
  if (s.empty() || s.size() > 6) return false;
  int v = 0;
  for (char c : s) {
    if (c < '0' || c > '9') return false;
    v = v * 10 + (c - '0');
  }
  if (v > 255) return false;
  out = v;
  return true;
}

// The whole token must be a number. strtod also takes hex ("0x10"), which the
// Rust reader does not, so hex is refused here to keep the two the same.
inline bool parse_number(const std::string& s, double& out) {
  if (s.empty() || s.find_first_of("xX") != std::string::npos) return false;
  errno = 0;
  char* end = nullptr;
  const double v = std::strtod(s.c_str(), &end);
  if (end != s.c_str() + s.size()) return false;
  out = v;
  return true;
}

}  // namespace detail

// Parse the whole table; `source` names the file in the messages. Any bad line
// rejects the file (the caller keeps its previous table), and every problem is
// reported at once with its line number.
inline CalibParseResult parse_energy_calib(const std::string& text,
                                           const std::string& source = "<calib>") {
  CalibParseResult r;
  std::istringstream in(text);
  std::string raw;
  std::map<uint16_t, int> first_line;
  int lineno = 0;
  while (std::getline(in, raw)) {
    ++lineno;
    const std::string where = source + ":" + std::to_string(lineno);
    std::istringstream cols(raw.substr(0, raw.find('#')));
    std::vector<std::string> c;
    for (std::string tok; cols >> tok;) c.push_back(tok);
    if (c.empty()) continue;
    if (c.size() != 4) {
      r.errors.push_back(where + ": expected 'module channel kev_per_adc offset_kev', got " +
                         std::to_string(c.size()) + " column(s)");
      continue;
    }
    int module = 0, channel = 0;
    double slope = 0.0, offset = 0.0;
    bool bad = false;
    if (!detail::parse_id(c[0], module)) {
      r.errors.push_back(where + ": module '" + c[0] + "' is not an integer 0..255");
      bad = true;
    }
    if (!detail::parse_id(c[1], channel)) {
      r.errors.push_back(where + ": channel '" + c[1] + "' is not an integer 0..255");
      bad = true;
    }
    if (!detail::parse_number(c[2], slope) || !std::isfinite(slope) || slope <= 0.0) {
      r.errors.push_back(where + ": kev_per_adc '" + c[2] + "' must be a finite number > 0");
      bad = true;
    }
    if (!detail::parse_number(c[3], offset) || !std::isfinite(offset)) {
      r.errors.push_back(where + ": offset_kev '" + c[3] + "' must be a finite number");
      bad = true;
    }
    if (bad) continue;
    const uint16_t key = calib_key(static_cast<unsigned>(module), static_cast<unsigned>(channel));
    auto dup = first_line.find(key);
    if (dup != first_line.end()) {
      r.errors.push_back(where + ": module " + c[0] + " channel " + c[1] +
                         " is already listed on line " + std::to_string(dup->second));
      continue;
    }
    first_line[key] = lineno;
    r.table[key] = LinearCalib{slope, offset};
  }
  return r;
}

// The keV range of a histogram whose ADC axis is [0, adc_max): the top is the
// largest mapped limit among `channels`, the bottom 0 or the lowest offset.
struct AxisLimits {
  bool valid = false;  // no channel here has coefficients
  double lo = 0.0;
  double hi = 0.0;
  bool operator==(const AxisLimits& o) const {
    return valid == o.valid && lo == o.lo && hi == o.hi;
  }
};

inline AxisLimits axis_limits_kev(const CalibTable& table, const std::vector<uint16_t>& channels,
                                  double adc_max) {
  AxisLimits a;
  for (uint16_t key : channels) {
    auto it = table.find(key);
    if (it == table.end()) continue;
    const double top = it->second.to_kev(adc_max);
    const double low = it->second.offset_kev < 0.0 ? it->second.offset_kev : 0.0;
    if (!a.valid || top > a.hi) a.hi = top;
    if (!a.valid || low < a.lo) a.lo = low;
    a.valid = true;
  }
  return a;
}

// The table in force, owned by the display.
struct ActiveCalib {
  CalibTable table;
  std::string source_path;
  std::string loaded_at;  // UTC ISO time of the latest successful read
  bool loaded = false;    // a valid table has been adopted at least once
  bool stale = false;     // the latest read failed; previous table kept
  bool failed = false;    // no read has succeeded yet
  std::string error;
};

// Same policy as reload_offsets() in time_offsets.hpp: on any failure the previous
// table is KEPT. Unlike time offsets, nothing recorded depends on this, so a failure
// at start-up does not stop the program. `text == nullptr` means the file could not
// be read. Returns whether the read succeeded; `changed` says whether the
// coefficients differ from what was in force.
inline bool reload_calib(ActiveCalib& a, const std::string* text, const std::string& read_error,
                         const std::string& path, const std::string& now_utc,
                         std::vector<std::string>& errors, bool& changed) {
  errors.clear();
  changed = false;
  CalibParseResult r;
  if (!text) {
    errors.push_back("cannot read " + path + ": " + read_error);
  } else {
    r = parse_energy_calib(*text, path);
    errors = r.errors;
  }
  if (!errors.empty()) {
    a.source_path = path;
    a.failed = !a.loaded;
    a.stale = a.loaded;
    a.error.clear();
    for (std::size_t i = 0; i < errors.size(); ++i) a.error += (i ? "; " : "") + errors[i];
    return false;
  }
  changed = !(r.table == a.table);
  a.table = std::move(r.table);
  a.source_path = path;
  a.loaded_at = now_utc;
  a.loaded = true;
  a.stale = false;
  a.failed = false;
  a.error.clear();
  return true;
}

// The status-line field: "off" with no file, then the channel count, with the state.
inline std::string calib_status_field(const ActiveCalib& a, bool configured) {
  if (!configured) return "off";
  if (a.failed) return "ERROR";
  std::string s = std::to_string(a.table.size());
  if (a.stale) s += " STALE";
  return s;
}

}  // namespace ecal

#endif  // DELILA_ENERGY_CALIB_HPP

// p2_delila.cxx — TODO 90 P2 on real ThGEM data, straight from .delila files:
// does the CFD model (cfd_emu.hpp) put the crossing where the V1730 FW did?
// For every ThGEM electrode channel of the map: the model crossing at the
// channel's FW setting (digitizer JSON) vs the FW timestamp's fraction of a
// sample. A dual-trace record (every value written twice, 4 ns real sampling,
// pf-qaq run 17) is compared three ways: as recorded, and rebuilt to 2 ns by
// linear and cubic interpolation (cfd_scan.hpp undouble). The scan can only be
// trusted on a channel kind whose RMS here is small against the time-sum widths.
//
//   g++ -std=c++17 -O2 -I../delila2root -I../root_sink/thgem p2_delila.cxx -o p2_delila
//   ./p2_delila --map MAP --digitizer JSON [--digitizer JSON ...]
//       [--max 3000] [--baseline 64] [--from SAMPLES] FILE.delila [...]
//
// No ROOT. License: BSD-3-Clause (same as delila-rs).
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

#include "TDelila.hpp"
#include "cfd_scan.hpp"
#include "thgem_map.hpp"

using namespace cfdtune;

namespace {

bool read_file(const std::string& path, std::string& out) {
  std::ifstream f(path);
  if (!f) return false;
  std::stringstream ss;
  ss << f.rdbuf();
  out = ss.str();
  return true;
}

struct Stat {
  std::vector<double> d;    // model frac - FW frac, samples, wrapped
  std::vector<double> pos;  // model crossing in the record, samples
  long missing = 0;
  void line(const char* mode) const {
    if (d.empty()) {
      std::printf("      %-9s no crossings (missing %ld)\n", mode, missing);
      return;
    }
    std::vector<double> v = d;
    std::sort(v.begin(), v.end());
    const double med = v[v.size() / 2];
    double rms = 0;
    long close = 0;
    for (double x : v) {
      rms += (x - med) * (x - med);
      if (std::fabs(x - med) < 0.05) ++close;
    }
    rms = std::sqrt(rms / static_cast<double>(v.size()));
    std::vector<double> dev;
    for (double x : v) dev.push_back(x - med);
    std::vector<double> p = pos;
    std::sort(p.begin(), p.end());
    auto q = [&](double f) { return p[static_cast<size_t>(f * static_cast<double>(p.size() - 1))]; };
    std::printf("      %-9s n %5zu  median %+.3f  RMS %.3f smp (%.3f ns)  sigma68 %.3f ns  |d-med|<0.05: %5.1f %%  missing %ld"
                "  crossing at %.0f/%.0f/%.0f (10/50/90 %%)\n",
                mode, v.size(), med, rms, 2.0 * rms, 2.0 * sigma68(dev), 100.0 * close / v.size(), missing, q(0.1), q(0.5),
                q(0.9));
  }
};

}  // namespace

int main(int argc, char** argv) {
  std::string map_file;
  std::vector<std::string> dig_files, data_files;
  long max_per_ch = 3000;
  int n_baseline = 64;
  long from = 0;  // samples: start of the model's search (skip noise before the FW trigger)
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    auto val = [&]() -> std::string {
      if (i + 1 >= argc) {
        std::fprintf(stderr, "%s needs a value\n", a.c_str());
        std::exit(2);
      }
      return argv[++i];
    };
    if (a == "--map") map_file = val();
    else if (a == "--digitizer") dig_files.push_back(val());
    else if (a == "--max") max_per_ch = std::atol(val().c_str());
    else if (a == "--baseline") n_baseline = std::atoi(val().c_str());
    else if (a == "--from") from = std::atol(val().c_str());
    else if (!a.empty() && a[0] == '-') {
      std::fprintf(stderr, "unknown option %s\n", a.c_str());
      return 2;
    } else data_files.push_back(a);
  }
  if (map_file.empty() || dig_files.empty() || data_files.empty()) {
    std::fprintf(stderr, "usage: %s --map MAP --digitizer JSON [...] [--max N] [--baseline N] [--from SAMPLES] FILE.delila [...]\n", argv[0]);
    return 2;
  }

  std::string text;
  if (!read_file(map_file, text)) {
    std::fprintf(stderr, "cannot read %s\n", map_file.c_str());
    return 1;
  }
  auto mr = thgem::parse_channel_map(text, map_file);
  for (const auto& e : mr.errors) std::fprintf(stderr, "map ERROR %s\n", e.c_str());
  if (!mr.errors.empty()) return 1;

  std::map<int, ChannelSettings> fw;
  for (const auto& f : dig_files) {
    if (!read_file(f, text)) {
      std::fprintf(stderr, "cannot read %s\n", f.c_str());
      return 1;
    }
    ChannelSettings cs = parse_channel_settings(text, 2.0);
    fw[cs.module] = cs;
  }

  // The ThGEM electrode channels we can model: key -> label, FW setting.
  const char* kName[5] = {"T", "XL", "XR", "YU", "YD"};
  struct Chan {
    std::string label;
    ChannelFw fw;
    Stat st[3];
    long seen = 0, dual = 0;
  };
  std::map<int, Chan> chans;
  for (const auto& [n, g] : mr.map.thgems)
    for (int k = 0; k < 5; ++k) {
      if (g.key[k] < 0) continue;
      int m = g.key[k] >> 8, c = g.key[k] & 0xff;
      auto it = fw.find(m);
      if (it == fw.end()) continue;
      const ChannelFw& cf = it->second.ch.at(static_cast<size_t>(c));
      if (!cf.enabled || !cf.cfd_mode) continue;
      Chan ch;
      ch.label = "D" + std::to_string(n) + "-" + kName[k] + " (m" + std::to_string(m) + " c" + std::to_string(c) + ")";
      ch.fw = cf;
      chans[g.key[k]] = ch;
    }
  if (chans.empty()) {
    std::fprintf(stderr, "no ThGEM channel with a --digitizer in CFD mode\n");
    return 1;
  }

  long full = 0;
  for (const auto& file : data_files) {
    tdelila::TDelila d(file);
    if (!d.good()) {
      std::fprintf(stderr, "%s: %s\n", file.c_str(), d.error().c_str());
      return 1;
    }
    tdelila::Event ev;
    while (d.next(ev)) {
      auto it = chans.find((ev.module() << 8) | ev.channel());
      if (it == chans.end() || !ev.has_waveform()) continue;
      Chan& ch = it->second;
      if (ch.seen >= max_per_ch) continue;
      if (++ch.seen == max_per_ch) ++full;
      const auto& wf = ev.waveform();
      const double nsps = wf.ns_per_sample() > 0 ? wf.ns_per_sample() : 2.0;
      const double fw_frac = ev.timestamp_ns() / nsps - std::floor(ev.timestamp_ns() / nsps);
      std::vector<short> raw = wf.analog_probe(1);
      if (pairs_duplicated(raw)) ++ch.dual;
      const std::vector<short> recs[3] = {raw, undouble(raw, Undouble::Linear), undouble(raw, Undouble::Cubic)};
      for (int mode = 0; mode < 3; ++mode) {
        auto s = moving_average(prepare(recs[mode], ch.fw.pol, n_baseline), ch.fw.p.smooth);
        Crossing c = cfd_crossing(s, ch.fw.p, ch.fw.threshold, from);
        if (!c.found) {
          ++ch.st[mode].missing;
          continue;
        }
        ch.st[mode].d.push_back(frac_diff(c.pos - std::floor(c.pos), fw_frac));
        ch.st[mode].pos.push_back(c.pos);
      }
    }
    if (full == static_cast<long>(chans.size())) break;
  }

  const char* kMode[3] = {"recorded", "linear", "cubic"};
  for (const auto& [key, ch] : chans) {
    std::printf("%s: %ld waveforms (%ld dual trace); FW delay %d smp, %d %%, smooth %d, thr %.0f, %s\n", ch.label.c_str(),
                ch.seen, ch.dual, ch.fw.p.delay, ch.fw.p.fraction_pct, ch.fw.p.smooth, ch.fw.threshold,
                ch.fw.pol == Polarity::Positive ? "positive" : "negative");
    if (ch.seen == 0) continue;
    for (int mode = 0; mode < 3; ++mode) ch.st[mode].line(kMode[mode]);
  }
  return 0;
}

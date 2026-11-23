// coinc_arms.cxx — how big are the ThGEM arm pulses that belong to a real
// trigger? (TODO 90, threshold choice). For every trigger hit with Qlong >=
// --t-energy, the arm hit of the same ThGEM nearest in time within
// [-100, +500] ns of it: its pulse height (smoothed as its FW setting, baseline
// = first --baseline samples; dual trace rebuilt linearly) and its Qlong. An arm
// hit found this way is the arm's half of a real event whatever its charge, so
// the quantiles say what a higher arm threshold or a charge cut would lose.
// Only arm hits that triggered at the run's threshold exist in the data.
//
//   g++ -std=c++17 -O2 -I../delila2root -I../root_sink/thgem coinc_arms.cxx -o coinc_arms
//   ./coinc_arms --map MAP --digitizer JSON [...] [--t-energy 4500] [--baseline 32] FILE.delila [...]
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

struct Hit {
  double t;
  double energy;
  double height;
};

double q(std::vector<double> v, double f) {
  if (v.empty()) return NAN;
  std::sort(v.begin(), v.end());
  return v[static_cast<size_t>(f * static_cast<double>(v.size() - 1))];
}

double frac_ge(const std::vector<double>& v, double x) {
  if (v.empty()) return NAN;
  long n = 0;
  for (double y : v)
    if (y >= x) ++n;
  return static_cast<double>(n) / static_cast<double>(v.size());
}

}  // namespace

int main(int argc, char** argv) {
  std::string map_file;
  std::vector<std::string> dig_files, data_files;
  double t_energy = 4500;
  int n_baseline = 32;
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    auto val = [&]() -> std::string {
      if (i + 1 >= argc) std::exit(2);
      return argv[++i];
    };
    if (a == "--map") map_file = val();
    else if (a == "--digitizer") dig_files.push_back(val());
    else if (a == "--t-energy") t_energy = std::atof(val().c_str());
    else if (a == "--baseline") n_baseline = std::atoi(val().c_str());
    else if (!a.empty() && a[0] == '-') {
      std::fprintf(stderr, "unknown option %s\n", a.c_str());
      return 2;
    } else data_files.push_back(a);
  }
  if (map_file.empty() || dig_files.empty() || data_files.empty()) {
    std::fprintf(stderr, "usage: %s --map MAP --digitizer JSON [...] [--t-energy E] [--baseline N] FILE.delila [...]\n", argv[0]);
    return 2;
  }
  std::string text;
  if (!read_file(map_file, text)) return 1;
  auto mr = thgem::parse_channel_map(text, map_file);
  if (!mr.errors.empty()) return 1;
  std::map<int, ChannelSettings> fw;
  for (const auto& f : dig_files) {
    if (!read_file(f, text)) return 1;
    ChannelSettings cs = parse_channel_settings(text, 2.0);
    fw[cs.module] = cs;
  }
  // key -> (ThGEM number, electrode index), FW setting
  struct Role {
    int thgem, k;
    ChannelFw fw;
  };
  std::map<int, Role> roles;
  for (const auto& [n, g] : mr.map.thgems)
    for (int k = 0; k < 5; ++k)
      if (g.key[k] >= 0 && fw.count(g.key[k] >> 8)) roles[g.key[k]] = Role{n, k, fw[g.key[k] >> 8].ch.at(g.key[k] & 0xff)};

  std::map<int, std::vector<Hit>> hits[5];  // [electrode][thgem] -> hits
  for (const auto& file : data_files) {
    tdelila::TDelila d(file);
    if (!d.good()) return 1;
    tdelila::Event ev;
    while (d.next(ev)) {
      auto it = roles.find((ev.module() << 8) | ev.channel());
      if (it == roles.end()) continue;
      const Role& r = it->second;
      Hit h{ev.timestamp_ns(), static_cast<double>(ev.energy()), NAN};
      if (r.k == 0 && h.energy < t_energy) continue;  // triggers: real ones only
      if (ev.has_waveform()) {
        auto s = moving_average(prepare(undouble(ev.waveform().analog_probe(1), Undouble::Linear), r.fw.pol, n_baseline),
                                r.fw.p.smooth);
        double peak = -1e9;
        for (size_t i = static_cast<size_t>(n_baseline); i < s.size(); ++i) peak = std::max(peak, s[i]);
        h.height = peak;
      }
      hits[r.k][r.thgem].push_back(h);
    }
  }
  const char* kName[5] = {"T", "XL", "XR", "YU", "YD"};
  for (auto& [n, trig] : hits[0]) {
    std::sort(trig.begin(), trig.end(), [](const Hit& a, const Hit& b) { return a.t < b.t; });
    std::printf("D%d: %zu real triggers (Qlong >= %.0f)\n", n, trig.size(), t_energy);
    for (int k = 1; k < 5; ++k) {
      auto& arm = hits[k][n];
      std::sort(arm.begin(), arm.end(), [](const Hit& a, const Hit& b) { return a.t < b.t; });
      std::vector<double> he, en, dt;
      for (const Hit& t0 : trig) {
        auto lo = std::lower_bound(arm.begin(), arm.end(), t0.t - 100.0, [](const Hit& a, double v) { return a.t < v; });
        const Hit* best = nullptr;
        for (auto it = lo; it != arm.end() && it->t <= t0.t + 500.0; ++it)
          if (!best || std::fabs(it->t - t0.t - 100.0) < std::fabs(best->t - t0.t - 100.0)) best = &*it;
        if (!best) continue;
        en.push_back(best->energy);
        dt.push_back(best->t - t0.t);
        if (!std::isnan(best->height)) he.push_back(best->height);
      }
      std::printf("  %-2s found for %4zu (%5.1f %%)  dt 10/50/90 %4.0f %4.0f %4.0f ns | height 1/5/10/50 %% %5.0f %5.0f %5.0f %5.0f"
                  " | >=200 %.2f >=300 %.2f >=400 %.2f | Qlong 1/5/50 %% %5.0f %5.0f %5.0f, >=4500 %.2f\n",
                  kName[k], en.size(), trig.empty() ? 0.0 : 100.0 * en.size() / trig.size(), q(dt, .1), q(dt, .5), q(dt, .9),
                  q(he, .01), q(he, .05), q(he, .1), q(he, .5), frac_ge(he, 200), frac_ge(he, 300), frac_ge(he, 400), q(en, .01),
                  q(en, .05), q(en, .5), frac_ge(en, 4500));
    }
  }
  return 0;
}

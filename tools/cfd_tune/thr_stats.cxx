// thr_stats.cxx — ThGEM threshold check without waveforms (TODO 90): per
// ThGEM, how many real triggers (T with Qlong >= --t-energy) have each arm, both
// X arms, both Y arms and all four within [-100, +500] ns, and how often each arm
// fires on noise (Qlong < --noise-energy). Run it on short runs taken at
// different arm thresholds: the threshold to keep is the lowest one whose noise
// rate is still acceptable, if a lower one finds more arms.
//
//   g++ -std=c++17 -O2 -I../delila2root -I../root_sink/thgem thr_stats.cxx -o thr_stats
//   ./thr_stats --map MAP [--t-energy 4500] [--noise-energy 4500] [--arm-min 0] FILE.delila [...]
//
// --arm-min E ignores arm hits with Qlong < E when looking for the arms of a
// real T: on a run taken without the Reader's adc_min it shows what adc_min = E
// would cost.
//
// No ROOT, no digitizer JSON. License: BSD-3-Clause (same as delila-rs).
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
#include "thgem_map.hpp"

namespace {

bool read_file(const std::string& path, std::string& out) {
  std::ifstream f(path);
  if (!f) return false;
  std::stringstream ss;
  ss << f.rdbuf();
  out = ss.str();
  return true;
}

// Any time in sorted `v` within [t + lo, t + hi]?
bool any_in(const std::vector<double>& v, double t, double lo, double hi) {
  auto it = std::lower_bound(v.begin(), v.end(), t + lo);
  return it != v.end() && *it <= t + hi;
}

}  // namespace

int main(int argc, char** argv) {
  std::string map_file;
  std::vector<std::string> files;
  double t_energy = 4500, noise_energy = 4500, arm_min = 0;
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    auto val = [&]() -> std::string {
      if (i + 1 >= argc) std::exit(2);
      return argv[++i];
    };
    if (a == "--map") map_file = val();
    else if (a == "--t-energy") t_energy = std::atof(val().c_str());
    else if (a == "--noise-energy") noise_energy = std::atof(val().c_str());
    else if (a == "--arm-min") arm_min = std::atof(val().c_str());
    else if (!a.empty() && a[0] == '-') {
      std::fprintf(stderr, "unknown option %s\n", a.c_str());
      return 2;
    } else files.push_back(a);
  }
  if (map_file.empty() || files.empty()) {
    std::fprintf(stderr, "usage: %s --map MAP [--t-energy E] [--noise-energy E] [--arm-min E] FILE.delila [...]\n", argv[0]);
    return 2;
  }
  std::string text;
  if (!read_file(map_file, text)) return 1;
  auto mr = thgem::parse_channel_map(text, map_file);
  if (!mr.errors.empty()) return 1;
  std::map<int, std::pair<int, int>> role;  // key -> (thgem, electrode)
  for (const auto& [n, g] : mr.map.thgems)
    for (int k = 0; k < 5; ++k)
      if (g.key[k] >= 0) role[g.key[k]] = {n, k};

  std::map<int, std::vector<double>> t_real, arm_all[5];
  std::map<int, long> n_all[5], n_noise[5];
  double t0 = 0, t1 = 0;
  bool have = false;
  for (const auto& f : files) {
    tdelila::TDelila d(f);
    if (!d.good()) {
      std::fprintf(stderr, "%s: %s\n", f.c_str(), d.error().c_str());
      return 1;
    }
    tdelila::Event ev;
    while (d.next(ev)) {
      const double t = ev.timestamp_ns();
      if (ev.module() == 0) {
        if (!have) t0 = t1 = t, have = true;
        t0 = std::min(t0, t), t1 = std::max(t1, t);
      }
      auto it = role.find((ev.module() << 8) | ev.channel());
      if (it == role.end()) continue;
      const auto [n, k] = it->second;
      ++n_all[k][n];
      if (ev.energy() < noise_energy) ++n_noise[k][n];
      if (k == 0) {
        if (ev.energy() >= t_energy) t_real[n].push_back(t);
      } else if (ev.energy() >= arm_min) {
        arm_all[k][n].push_back(t);
      }
    }
  }
  const double secs = (t1 - t0) * 1e-9;
  std::printf("%.0f s of data; real T = Qlong >= %.0f; noise = Qlong < %.0f; arm (Qlong >= %.0f) searched in [-100, +500] ns of a real T\n",
              secs, t_energy, noise_energy, arm_min);
  std::printf("%-4s %8s %8s | %-31s | %-31s | %6s %6s %6s\n", "", "real T/s", "T noise/s", "arm found % (XL XR YU YD)",
              "arm noise /s (XL XR YU YD)", "X ok%", "Y ok%", "4 ok%");
  for (auto& [n, tr] : t_real) {
    for (int k = 1; k < 5; ++k) std::sort(arm_all[k][n].begin(), arm_all[k][n].end());
    long found[5] = {0, 0, 0, 0, 0}, x_ok = 0, y_ok = 0, all_ok = 0;
    for (double t : tr) {
      bool f[5];
      for (int k = 1; k < 5; ++k) {
        f[k] = any_in(arm_all[k][n], t, -100.0, 500.0);
        found[k] += f[k];
      }
      x_ok += f[1] && f[2];
      y_ok += f[3] && f[4];
      all_ok += f[1] && f[2] && f[3] && f[4];
    }
    const double nt = std::max<double>(1.0, static_cast<double>(tr.size()));
    std::printf("D%-3d %8.2f %9.1f |", n, tr.size() / secs, n_noise[0][n] / secs);
    for (int k = 1; k < 5; ++k) std::printf(" %6.1f", 100.0 * found[k] / nt);
    std::printf("    |");
    for (int k = 1; k < 5; ++k) std::printf(" %6.1f", n_noise[k][n] / secs);
    std::printf("    | %6.1f %6.1f %6.1f\n", 100.0 * x_ok / nt, 100.0 * y_ok / nt, 100.0 * all_ok / nt);
  }
  return 0;
}

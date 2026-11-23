// amp_check.cxx — trigger threshold vs noise on the ThGEM channels (TODO 90):
// per electrode channel, the pulse height of every recorded hit (smoothed as the
// channel's FW setting, baseline = mean of the first --baseline samples) split
// by the FW charge: Qlong >= --real-energy = a real pulse, below = noise (the
// ringing / pickup triggers of pf-qaq run 17 sit at the charge pedestal). For a
// few candidate thresholds: the fraction of noise hits still above it and of
// real pulses kept. Only hits that triggered at the run's threshold are seen,
// so this can rank higher thresholds, not lower ones. Dual-trace records are
// rebuilt to 2 ns (linear) first.
//
//   g++ -std=c++17 -O2 -I../delila2root -I../root_sink/thgem amp_check.cxx -o amp_check
//   ./amp_check --map MAP --digitizer JSON [...] [--real-energy 4500] [--baseline 32]
//       [--max-noise 20000] FILE.delila [...]
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

double quantile(std::vector<double>& v, double q) {
  if (v.empty()) return NAN;
  std::sort(v.begin(), v.end());
  return v[static_cast<size_t>(q * static_cast<double>(v.size() - 1))];
}

double frac_at_or_above(const std::vector<double>& v, double thr) {
  if (v.empty()) return NAN;
  long n = 0;
  for (double x : v)
    if (x >= thr) ++n;
  return static_cast<double>(n) / static_cast<double>(v.size());
}

}  // namespace

int main(int argc, char** argv) {
  std::string map_file;
  std::vector<std::string> dig_files, data_files;
  double real_energy = 4500;
  int n_baseline = 32;
  long max_noise = 20000;
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
    else if (a == "--real-energy") real_energy = std::atof(val().c_str());
    else if (a == "--baseline") n_baseline = std::atoi(val().c_str());
    else if (a == "--max-noise") max_noise = std::atol(val().c_str());
    else if (!a.empty() && a[0] == '-') {
      std::fprintf(stderr, "unknown option %s\n", a.c_str());
      return 2;
    } else data_files.push_back(a);
  }
  if (map_file.empty() || dig_files.empty() || data_files.empty()) {
    std::fprintf(stderr, "usage: %s --map MAP --digitizer JSON [...] [--real-energy E] [--baseline N] [--max-noise N] FILE.delila [...]\n", argv[0]);
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
  const char* kName[5] = {"T", "XL", "XR", "YU", "YD"};
  struct Chan {
    std::string label;
    ChannelFw fw;
    std::vector<double> noise, real;
  };
  std::map<int, Chan> chans;
  for (const auto& [n, g] : mr.map.thgems)
    for (int k = 0; k < 5; ++k) {
      if (g.key[k] < 0) continue;
      int m = g.key[k] >> 8, c = g.key[k] & 0xff;
      auto it = fw.find(m);
      if (it == fw.end()) continue;
      Chan ch;
      ch.label = "D" + std::to_string(n) + "-" + kName[k];
      ch.fw = it->second.ch.at(c);
      chans[g.key[k]] = ch;
    }

  double t_first = 0, t_last = 0;
  bool have_t = false;
  for (const auto& file : data_files) {
    tdelila::TDelila d(file);
    if (!d.good()) {
      std::fprintf(stderr, "%s: %s\n", file.c_str(), d.error().c_str());
      return 1;
    }
    tdelila::Event ev;
    while (d.next(ev)) {
      if (ev.module() == 0) {  // run time from the master board
        if (!have_t) t_first = ev.timestamp_ns(), have_t = true;
        t_first = std::min(t_first, ev.timestamp_ns());
        t_last = std::max(t_last, ev.timestamp_ns());
      }
      auto it = chans.find((ev.module() << 8) | ev.channel());
      if (it == chans.end() || !ev.has_waveform()) continue;
      Chan& ch = it->second;
      const bool is_real = ev.energy() >= real_energy;
      if (!is_real && static_cast<long>(ch.noise.size()) >= max_noise) continue;
      auto s = moving_average(prepare(undouble(ev.waveform().analog_probe(1), Undouble::Linear), ch.fw.pol, n_baseline),
                              ch.fw.p.smooth);
      double peak = -1e9;
      for (size_t i = static_cast<size_t>(n_baseline); i < s.size(); ++i) peak = std::max(peak, s[i]);
      (is_real ? ch.real : ch.noise).push_back(peak);
    }
  }
  const double secs = (t_last - t_first) * 1e-9;
  std::printf("%.1f s of data (module 0 timestamps); real = Qlong >= %.0f; height = max of the smoothed, baseline-subtracted input\n",
              secs, real_energy);
  const double cand[] = {150, 200, 250, 300, 400, 500};
  std::printf("%-7s %5s %8s | %-22s | %-22s |", "chan", "thr", "real", "noise height 50/99/99.9%", "real height 1/5/50%");
  for (double c : cand) std::printf(" thr%-4.0f", c);
  std::printf("   (noise left / real kept)\n");
  for (auto& [key, ch] : chans) {
    std::vector<double> nz = ch.noise, rl = ch.real;
    std::printf("%-7s %5.0f %8zu | %6.0f %6.0f %6.0f      | %6.0f %6.0f %6.0f      |", ch.label.c_str(), ch.fw.threshold,
                ch.real.size(), quantile(nz, 0.5), quantile(nz, 0.99), quantile(nz, 0.999), quantile(rl, 0.01),
                quantile(rl, 0.05), quantile(rl, 0.5));
    for (double c : cand) std::printf(" %.3f/%.2f", frac_at_or_above(ch.noise, c), frac_at_or_above(ch.real, c));
    std::printf("\n");
  }
  return 0;
}

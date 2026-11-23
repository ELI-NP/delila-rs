// cfd_scan.cxx — scan V1730 DPP-PSD CFD settings offline on recorded ThGEM
// waveforms and pick the ones that make the delay-line time sums narrowest
// (TODO 90 P3). The FW model (cfd_emu.hpp) was checked against the FW in P2.
//
//   g++ -O2 -std=c++17 -Wall -Wextra -I../delila2root -I../root_sink/thgem
//       cfd_scan.cxx -o cfd_scan
//   ./cfd_scan --map config/channel_map_11B_208Pb.txt
//       --digitizer config/digitizers/11B_208Pb_v1730_psd1_1.json [--digitizer ...]
//       [--window-ns 1000] [--delays-ns 2:60] [--max-events 5000] [--thgem N]
//       [--rounds 4] [--out patch.json] run0042_*.delila
//
// Input: runs with waveforms on the ThGEM channels, single trace (Input only,
// full rate), record length < 1792 samples (couple arbiter, UM4380). The
// digitizer JSONs must be the ones the run was taken with: the FW setting of
// each channel ties its FW timestamp to its waveform.
//
// Output: per ThGEM the time-sum widths (sigma68 of S_X = XL + XR - 2T and S_Y =
// YU + YD - 2T) at the FW setting and at the best setting found, the best
// trigger and arm settings, and (--out) a channel_overrides patch for the
// digitizer JSONs. Apply it through Tune Up / the Operator REST API, then check
// the widths again on a new run (P4).
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <map>
#include <set>
#include <sstream>
#include <string>
#include <vector>

#include "TDelila.hpp"
#include "cfd_scan.hpp"
#include "thgem_map.hpp"

using namespace cfdtune;

namespace {

struct Options {
  std::string map_file;
  std::vector<std::string> digitizer_files;
  std::vector<std::string> data_files;
  double window_ns = 1000;
  int delay_min_ns = 2, delay_max_ns = 60;
  long max_events = 5000;
  int only_thgem = -1;
  int rounds = 4;
  int n_baseline = 64;
  double min_kept = 0.9;  // a setting must keep this fraction of the FW-timed sums
  bool fw_only = false;   // only the time-sum widths from FW timestamps (no waveforms, no scan)
  double min_energy = 0;  // skip hits with a smaller FW charge (Qlong): noise triggers (pf-qaq run 17)
  std::string out_file;
  std::string dump_file;  // per-event electrode times (FW and best setting) as CSV, for 2D plots
  // --eval-t / --eval-arm "DELAY_NS:FRACTION:SMOOTH": no search, report this setting
  // (e.g. the best of another run: an out-of-sample check of the scan).
  bool eval = false;
  int eval_t[3] = {0, 0, 0}, eval_arm[3] = {0, 0, 0};
};

void usage(const char* a0) {
  std::printf(
      "usage: %s --map MAP --digitizer JSON [--digitizer JSON ...] [--window-ns W]\n"
      "          [--delays-ns MIN:MAX] [--max-events N] [--thgem N] [--rounds R] [--min-energy Q] [--dump EVENTS.csv]\n"
      "          [--eval-t NS:PCT:SMOOTH --eval-arm NS:PCT:SMOOTH]\n"
      "          [--baseline N] [--min-kept F] [--fw-only] [--out PATCH.json] FILE.delila [...]\n"
      "  --fw-only: time-sum widths from the FW timestamps only (any run, waveforms or not;\n"
      "             the P4 check after applying a patch). No scan.\n",
      a0);
}

bool read_file(const std::string& path, std::string& out) {
  std::ifstream f(path);
  if (!f) return false;
  std::stringstream ss;
  ss << f.rdbuf();
  out = ss.str();
  return true;
}

bool parse_args(int argc, char** argv, Options& o) {
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    auto val = [&](std::string& v) {
      if (i + 1 >= argc) return false;
      v = argv[++i];
      return true;
    };
    std::string v;
    if (a == "--map") {
      if (!val(o.map_file)) return false;
    } else if (a == "--digitizer") {
      if (!val(v)) return false;
      o.digitizer_files.push_back(v);
    } else if (a == "--window-ns") {
      if (!val(v)) return false;
      o.window_ns = std::atof(v.c_str());
    } else if (a == "--delays-ns") {
      if (!val(v) || std::sscanf(v.c_str(), "%d:%d", &o.delay_min_ns, &o.delay_max_ns) != 2) return false;
    } else if (a == "--max-events") {
      if (!val(v)) return false;
      o.max_events = std::atol(v.c_str());
    } else if (a == "--thgem") {
      if (!val(v)) return false;
      o.only_thgem = std::atoi(v.c_str());
    } else if (a == "--rounds") {
      if (!val(v)) return false;
      o.rounds = std::atoi(v.c_str());
    } else if (a == "--baseline") {
      if (!val(v)) return false;
      o.n_baseline = std::atoi(v.c_str());
    } else if (a == "--min-kept") {
      if (!val(v)) return false;
      o.min_kept = std::atof(v.c_str());
    } else if (a == "--fw-only") {
      o.fw_only = true;
    } else if (a == "--min-energy") {
      if (!val(v)) return false;
      o.min_energy = std::atof(v.c_str());
    } else if (a == "--out") {
      if (!val(o.out_file)) return false;
    } else if (a == "--dump") {
      if (!val(o.dump_file)) return false;
    } else if (a == "--eval-t" || a == "--eval-arm") {
      int* d = a == "--eval-t" ? o.eval_t : o.eval_arm;
      if (!val(v) || std::sscanf(v.c_str(), "%d:%d:%d", &d[0], &d[1], &d[2]) != 3) return false;
      o.eval = true;
    } else if (a == "-h" || a == "--help") {
      return false;
    } else if (!a.empty() && a[0] == '-') {
      std::fprintf(stderr, "unknown option %s\n", a.c_str());
      return false;
    } else {
      o.data_files.push_back(a);
    }
  }
  return !o.map_file.empty() && (o.fw_only || !o.digitizer_files.empty()) && !o.data_files.empty();
}

std::string params_text(const CfdParams& p, double ns) {
  char b[96];
  std::snprintf(b, sizeof(b), "delay %3g ns, %3d %%, smooth %2d", p.delay * ns, p.fraction_pct, p.smooth);
  return b;
}

double combine(const TimeSums& s) {
  double sx = sigma68(s.x), sy = sigma68(s.y);
  if (std::isnan(sx) && std::isnan(sy)) return std::numeric_limits<double>::infinity();
  if (std::isnan(sx)) return sy;
  if (std::isnan(sy)) return sx;
  return std::sqrt(sx * sx + sy * sy);
}

}  // namespace

int main(int argc, char** argv) {
  Options o;
  if (!parse_args(argc, argv, o)) {
    usage(argv[0]);
    return 2;
  }

  // Channel map: which channels are the electrodes of which ThGEM.
  std::string text;
  if (!read_file(o.map_file, text)) {
    std::fprintf(stderr, "cannot read %s\n", o.map_file.c_str());
    return 1;
  }
  auto mr = thgem::parse_channel_map(text, o.map_file);
  for (const auto& e : mr.errors) std::fprintf(stderr, "map ERROR %s\n", e.c_str());
  if (!mr.errors.empty()) return 1;
  for (const auto& w : mr.warnings) std::printf("map warning: %s\n", w.c_str());

  // FW CFD settings per module.
  std::map<int, ChannelSettings> fw;
  double ns_per_sample = 2.0;  // V1730; checked against the data below
  bool bad = false;
  for (const auto& f : o.digitizer_files) {
    if (!read_file(f, text)) {
      std::fprintf(stderr, "cannot read %s\n", f.c_str());
      return 1;
    }
    ChannelSettings cs = parse_channel_settings(text, ns_per_sample);
    if (cs.module < 0) {
      std::fprintf(stderr, "%s: no digitizer_id\n", f.c_str());
      return 1;
    }
    // Errors only matter for channels we use; keep them and check below.
    fw[cs.module] = cs;
  }

  struct Electrodes {
    int number;
    int key[5];
  };
  std::vector<Electrodes> thgems;
  for (const auto& [n, g] : mr.map.thgems) {
    if (o.only_thgem >= 0 && n != o.only_thgem) continue;
    if (!g.has_trigger()) continue;
    Electrodes e{n, {}};
    for (int k = 0; k < 5; ++k) e.key[k] = g.key[k];
    thgems.push_back(e);
  }
  if (thgems.empty()) {
    std::fprintf(stderr, "no ThGEM with a trigger in the map\n");
    return 1;
  }

  std::vector<PatchEntry> patch;
  const char* kName[5] = {"T", "XL", "XR", "YU", "YD"};

  // One CSV line per ThGEM event and setting: the five electrode times in ns
  // (empty = missing), for XY images and time sum vs position.
  std::ofstream dump;
  if (!o.dump_file.empty()) {
    dump.open(o.dump_file);
    dump << "thgem,set,t_T,t_XL,t_XR,t_YU,t_YD\n";
  }
  auto write_dump = [&](int number, const char* set, const std::vector<ThgemEvent>& evs, const std::vector<double>& t) {
    if (!dump.is_open()) return;
    char buf[64];
    for (const auto& e : evs) {
      dump << number << ',' << set;
      for (int k = 0; k < 5; ++k) {
        dump << ',';
        const int h = e.hit[k];
        if (h >= 0 && !std::isnan(t[static_cast<size_t>(h)])) {
          std::snprintf(buf, sizeof buf, "%.3f", t[static_cast<size_t>(h)]);
          dump << buf;
        }
      }
      dump << '\n';
    }
  };

  for (const auto& th : thgems) {
    // The electrodes' FW settings.
    ChannelFw efw[5];
    bool have[5] = {false, false, false, false, false};
    for (int k = 0; k < 5; ++k) {
      if (th.key[k] < 0) continue;
      if (o.fw_only) {
        have[k] = true;
        continue;
      }
      int m = th.key[k] >> 8, c = th.key[k] & 0xff;
      auto it = fw.find(m);
      if (it == fw.end()) {
        std::fprintf(stderr, "ThGEM %d %s: no --digitizer for module %d\n", th.number, kName[k], m);
        bad = true;
        continue;
      }
      for (const auto& err : it->second.errors)
        if (err.find("ch" + std::to_string(c) + ":") != std::string::npos) {
          std::fprintf(stderr, "ThGEM %d %s: %s\n", th.number, kName[k], err.c_str());
          bad = true;
        }
      efw[k] = it->second.ch.at(c);
      if (!efw[k].cfd_mode) {
        std::fprintf(stderr, "ThGEM %d %s (m%d c%d): not in CFD mode, cannot model\n", th.number, kName[k], m, c);
        bad = true;
      }
      have[k] = true;
    }
    if (bad) return 1;

    // Read this ThGEM's hits.
    std::vector<HitWave> hits;
    std::vector<ChannelHits> ch(5);
    long n_read = 0, n_nowf = 0, n_untimed = 0, n_low = 0, n_dual = 0;
    // Keep enough of each record around the FW crossing for the longest
    // scanned delay plus the longest smoothing (slow ThGEM pulses want long delays).
    const int crop_pre = std::max(100, o.delay_max_ns / 2 + 16 + 20);
    const int crop_post = std::max(80, o.delay_max_ns / 2 + 40);
    std::set<double> nsps_seen;
    for (const auto& file : o.data_files) {
      tdelila::TDelila d(file);
      if (!d.good()) {
        std::fprintf(stderr, "%s: %s\n", file.c_str(), d.error().c_str());
        return 1;
      }
      tdelila::Event ev;
      while (d.next(ev)) {
        int key = (ev.module() << 8) | ev.channel();
        int k = -1;
        for (int j = 0; j < 5; ++j)
          if (th.key[j] == key) k = j;
        if (k < 0) continue;
        ++n_read;
        if (ev.energy() < o.min_energy) {
          ++n_low;
          continue;
        }
        if (o.fw_only) {
          HitWave h;
          h.t_fw = ev.timestamp_ns();
          ch[static_cast<size_t>(k)].t.push_back(h.t_fw);
          ch[static_cast<size_t>(k)].hit.push_back(static_cast<int>(hits.size()));
          hits.push_back(std::move(h));
          continue;
        }
        if (!ev.has_waveform()) {
          ++n_nowf;
          continue;
        }
        const auto& wf = ev.waveform();
        nsps_seen.insert(wf.ns_per_sample());
        if (pairs_duplicated(wf.analog_probe(1))) {  // says 2 ns but samples every 4 ns
          ++n_dual;
          continue;
        }
        HitWave h;
        if (!make_hit(wf.analog_probe(1), efw[k], ev.timestamp_ns(), ns_per_sample, o.n_baseline, h, crop_pre, crop_post)) {
          ++n_untimed;
          continue;
        }
        ch[static_cast<size_t>(k)].t.push_back(h.t_fw);
        ch[static_cast<size_t>(k)].hit.push_back(static_cast<int>(hits.size()));
        hits.push_back(std::move(h));
      }
    }
    if (n_dual > 0) {
      std::fprintf(stderr,
                   "ThGEM %d: %ld dual-trace waveforms (every value written twice = 4 ns sampling although the record "
                   "says 2 ns): take the waveforms in single trace (waveform_run.py does)\n",
                   th.number, n_dual);
      return 1;
    }
    for (double v : nsps_seen)
      if (std::fabs(v - ns_per_sample) > 1e-9) {
        std::fprintf(stderr, "ThGEM %d: waveforms at %.2f ns/sample (need single trace at %.1f)\n", th.number, v,
                     ns_per_sample);
        return 1;
      }
    // Times ascending per channel (keep the hit index alongside).
    for (auto& c : ch) {
      std::vector<size_t> idx(c.t.size());
      for (size_t i = 0; i < idx.size(); ++i) idx[i] = i;
      std::sort(idx.begin(), idx.end(), [&](size_t a, size_t b) { return c.t[a] < c.t[b]; });
      ChannelHits s;
      for (size_t i : idx) {
        s.t.push_back(c.t[i]);
        s.hit.push_back(c.hit[i]);
      }
      c = s;
    }
    std::vector<ThgemEvent> ev = group_thgem(ch, o.window_ns);
    if (o.max_events > 0 && static_cast<long>(ev.size()) > o.max_events) ev.resize(static_cast<size_t>(o.max_events));

    std::printf("\n== ThGEM %d: %ld hits read, %ld below --min-energy, %ld without waveform, %ld not timed by the model "
                "at the FW setting, %zu triggers used\n",
                th.number, n_read, n_low, n_nowf, n_untimed, ev.size());
    if (ev.empty()) continue;

    std::vector<double> times(hits.size());
    for (size_t i = 0; i < hits.size(); ++i) times[i] = hits[i].t_fw;
    TimeSums s_fw = time_sums(ev, times);
    write_dump(th.number, "fw", ev, times);
    std::printf("   FW setting: sigma68 S_X %.3f ns (%zu), S_Y %.3f ns (%zu)\n", sigma68(s_fw.x), s_fw.x.size(),
                sigma68(s_fw.y), s_fw.y.size());
    if (o.fw_only) continue;
    const size_t need_x = static_cast<size_t>(o.min_kept * static_cast<double>(s_fw.x.size()));
    const size_t need_y = static_cast<size_t>(o.min_kept * static_cast<double>(s_fw.y.size()));

    // Which hit belongs to which electrode.
    std::vector<int> role(hits.size(), -1);
    for (int k = 0; k < 5; ++k)
      for (int h : ch[static_cast<size_t>(k)].hit) role[static_cast<size_t>(h)] = k;

    auto cost = [&](const CfdParams& pt, const CfdParams& pa) {
      std::vector<double> t(hits.size(), std::nan(""));
      for (size_t i = 0; i < hits.size(); ++i) {
        int k = role[i];
        if (k < 0) continue;
        double v;
        if (hits[i].retime(k == 0 ? pt : pa, efw[k].threshold, ns_per_sample, v)) t[i] = v;
      }
      TimeSums s = time_sums(ev, t);
      if (s.x.size() < need_x || s.y.size() < need_y) return std::numeric_limits<double>::infinity();
      return combine(s);
    };

    Grid g;
    for (int d = o.delay_min_ns; d <= o.delay_max_ns; d += 2) g.delays.push_back(d / 2);
    g.fractions = {25, 50, 75, 100};
    g.smooths = {1, 2, 4, 8, 16};
    CfdParams arm0 = efw[have[1] ? 1 : (have[3] ? 3 : 0)].p;
    SearchResult r;
    if (o.eval) {
      if (o.eval_t[0] % 2 != 0 || o.eval_arm[0] % 2 != 0 || o.eval_t[0] <= 0 || o.eval_arm[0] <= 0) {
        std::fprintf(stderr, "--eval-t / --eval-arm: the delay must be a positive even number of ns\n");
        return 1;
      }
      r.t = CfdParams{o.eval_t[0] / 2, o.eval_t[1], o.eval_t[2], efw[0].p.interp};
      r.arm = CfdParams{o.eval_arm[0] / 2, o.eval_arm[1], o.eval_arm[2], arm0.interp};
      r.evaluations = 1;
    } else {
      r = coordinate_descent(g, efw[0].p, arm0, cost, o.rounds);
    }

    // Report the best setting's sums.
    std::vector<double> tb(hits.size(), std::nan(""));
    for (size_t i = 0; i < hits.size(); ++i) {
      int k = role[i];
      double v;
      if (k >= 0 && hits[i].retime(k == 0 ? r.t : r.arm, efw[k].threshold, ns_per_sample, v)) tb[i] = v;
    }
    TimeSums s_best = time_sums(ev, tb);
    write_dump(th.number, "best", ev, tb);
    std::printf("   %s sigma68 S_X %.3f ns (%zu), S_Y %.3f ns (%zu)   [%d rounds, %ld settings tried]\n",
                o.eval ? "evaluated: " : "best:      ",
                sigma68(s_best.x), s_best.x.size(), sigma68(s_best.y), s_best.y.size(), r.rounds, r.evaluations);
    std::printf("   trigger: FW %s -> best %s\n", params_text(efw[0].p, ns_per_sample).c_str(),
                params_text(r.t, ns_per_sample).c_str());
    for (int k = 1; k < 5; ++k)
      if (have[k])
        std::printf("   %-2s:      FW %s -> best %s (common to the arms)\n", kName[k],
                    params_text(efw[k].p, ns_per_sample).c_str(), params_text(r.arm, ns_per_sample).c_str());
    for (int k = 0; k < 5; ++k)
      if (have[k]) patch.push_back({th.key[k] >> 8, th.key[k] & 0xff, k == 0 ? r.t : r.arm});
  }

  if (dump.is_open()) std::printf("\nwrote %s\n", o.dump_file.c_str());
  if (!o.out_file.empty()) {
    std::ofstream f(o.out_file);
    f << patch_json(patch, ns_per_sample);
    std::printf("\nwrote %s (%zu channels)\n", o.out_file.c_str(), patch.size());
  }
  return 0;
}

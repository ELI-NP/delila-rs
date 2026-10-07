// time_map.C — measure time offsets between digitizer channels and boards,
// plan the hardware start-delay change per board, verify it with the pulser,
// and map anti-Compton shields to their detectors (TODO 72, TODO 71 Phase D).
//
// Input: a hits tree (`delila`: module/b channel/b energy/s energy_short/s
// timestamp_ns/D) — delila2root output of a .delila or a root_sink --hits-tree
// file (a root_sink file written with --time-calib is ALIGNED: then every
// number below is a residual). Need not be time-sorted. `file` may be a TChain
// pattern ("run0112_*.root"). Pure logic: time_map.hpp (unit tests:
// test_time_map.cpp).
//
//   root -l -b -q -e '.L time_map.C+' -e 'time_map_offsets("run.root", "9:0", 2000, "ts.json")'
//   root -l -b -q -e '.L time_map.C+' -e 'time_map_boards("cal.root", "1:0", 2000, "", "16,10=8", "plan.json")'
//   root -l -b -q -e '.L time_map.C+' -e 'time_map_verify("cal.root", "check.root", "plan.json")'
//   root -l -b -q -e '.L time_map.C+' -e 'time_map_ac("run.root", "A1=9:0;C2=10:0", "8:*", 1000, "ac.json")'
//   root -l -b -q 'time_map.C+'              (prints this usage)
// ACLiC writes time_map_C.so/.d/.pcm next to the macro; to keep the source
// tree clean add  -e 'gSystem->SetBuildDir("/tmp/aclic", kTRUE)'  first.
//
// ---------------------------------------------------------------------------
// 1. time_map_offsets(file, ref "M:C", window_ns, out_json, options)
//    For every other channel: dt = t(ch) - t(ref) over all pairs within
//    +-window -> flat accidental level from the side bands -> background-
//    subtracted peak: mode, median, centroid, FWHM, central 90 %, signal, z.
//    Writes a timeSettings.json tree (ref = root, offset_ns = peak, aligned =
//    raw - offset) for the significant channels; every channel without a peak
//    is listed (report and _comment), never dropped silently. Also prints per
//    board the median of the significant peaks and the peak of the board's
//    SUMMED histogram (all its channels: statistics for weak boards).
//
// 2. time_map_boards(file, ref "M:C", window_ns, pulser_spec, step_spec, out_json, options)
//    TODO 72 §5: the LAST channel of each board carries a common pulser, the
//    others are detectors. Per board (module):
//      detector offset = median of its significant detector peaks (pulser
//                        excluded; the ref counts as 0 on its board) minus the
//                        ref board's median; a board with no significant
//                        channel falls back to its summed histogram (flagged
//                        SUM); a board whose rounded change differs between
//                        peak=mode/median/centroid is flagged (broad or
//                        asymmetric peaks: HPGe walk, slow Si) — all three are
//                        in the JSON (detector_offset_by_peak_ns);
//      pulser dt       = t(pulser of the board) - t(pulser of the ref board);
//      delay change    = sign * step * round(detector offset / step), relative
//                        to the ref board (+ a "non-negative" variant: the same
//                        plan plus a common constant, since a delay cannot go
//                        below 0 — only differences between boards matter);
//      expected pulser dt after = pulser dt - sign * delay change;
//      residual        = detector offset - sign * delay change (|.| <= step/2).
//    pulser_spec: "" = last channel of each module present, or "M:C,M:C,...".
//    step_spec: "" = 16 ns everywhere (DIG1 start_delay), "8", or "16,10=8,11=8"
//    (DIG2 rundelay = 8 ns). The JSON written is the input of time_map_verify
//    (edit delay_change_ns there if you applied something else).
//
// 3. time_map_verify(before_file, after_file, applied_json, options)
//    Pulser dt per board before/after (before_file "" = the values stored in
//    applied_json); PASS when (after - before) == -sign * (delay change of the
//    board - delay change of the ref board) within +-step/2, else FAIL.
//
// 4. time_map_ac(file, detectors, shields, window_ns, out_json, options)
//    For every (detector channel, shield channel) and every (detector group,
//    shield): dt = t(shield) - t(detector) -> prompt excess over accidentals,
//    peak, z and the VETO FRACTION = excess / detector hits. Prints the
//    group x shield matrix and assigns each shield to the group with the
//    largest significant veto fraction when every other significant group is
//    below `dominance` x that (default 0.5) — AMBIGUOUS otherwise, SILENT when
//    no group is significant. (Significance alone is not enough: gamma cascades
//    make distant detector/shield pairs prompt-coincident too.) Emits an
//    event_def v1.1 block {"tags", "anti_compton"} with one pair per group and
//    a window covering every assigned pair's central 90 % +- margin.
//    detectors: "name=M:C,M:C;name2=M:C-D" (one group per clover / position),
//               "M:C,M:C" (one group per channel) or "@event_def.json:tag1,tag2"
//               (one group per tag). shields: "8:*", "8:0-24", "M:C,..." or
//               "@event_def.json:BGO1,BGO2" (union).
//
// options (space-separated key=value; unknown keys are errors):
//   bin=2        Δt bin width, ns
//   nsigma=5     significance threshold of the background-subtracted peak
//   search=0.5   peak searched in |dt| < search*window
//   sb=0.75      side bands |dt| >= sb*window give the accidental level
//   cover=0.9    central fraction of the excess reported as the peak interval
//   multi=0.5    a second significant peak with >= multi x the main peak's
//                signal makes the channel ambiguous (not used); > 1 disables
//                (side-band asymmetry is only a note: dead-time dips and
//                after-pulse bumps beside a strong peak are harmless)
//   pairing=all  all | nearest (nearest: one entry per channel hit)
//   peak=median  median | mode | centroid: the value used as the offset
//                (median = 50 % of the excess, robust on the flat-topped HPGe
//                and Si peaks — the default; mode = most probable dt, best on
//                a sharp pulser but noisy on plateaus; centroid = mean, pulled
//                by walk tails)
//   calib=FILE   align the hits first (timeSettings.json or legacy time-calib)
//   tree=delila  hits tree name
//   sign=1       (boards) +1: start delay +D -> board timestamps -D; -1: reverse
//   dominance=0.5, margin=20   (ac) see 4.
//
// SIGN OF THE HARDWARE DELAY (sign=+1, the default). DIG1 start_delay (DevTree
// "Sync. start delay", docs/devtree_examples; CAEN register 0x8170 "Run Start/
// Stop Delay": the delay between the start signal reaching the board and the
// actual start of the run, used to compensate the S-IN daisy-chain propagation)
// and DIG2 rundelay (TODO 72 §2.1: the start delayed after the RUN signal edge)
// postpone the moment the board starts acquiring, and the trigger-time-tag
// counter starts with the run. A board started D later counts D less for the
// same physical instant: its timestamps get SMALLER by D. So a board that is
// LATE by D (dt > 0) needs its delay INCREASED by D. This follows the CAEN
// description and is NOT yet checked on hardware: the verification run (3.) is
// exactly that check — a wrong sign shows as FAIL with the change of the
// opposite sign, and sign=-1 then flips the plan.
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <map>
#include <set>
#include <sstream>
#include <string>
#include <vector>

#include "TChain.h"
#include "TFile.h"
#include "TList.h"
#include "TNamed.h"
#include "TTree.h"

#include "time_map.hpp"

namespace tmc {

using namespace rootsink;
using namespace rootsink::timemap;

inline bool read_text(const std::string& path, std::string& text, std::string& err) {
  std::ifstream in(path, std::ios::binary);
  if (!in) {
    err = "cannot read " + path;
    return false;
  }
  std::stringstream ss;
  ss << in.rdbuf();
  text = ss.str();
  return true;
}

struct Common {
  MeasureOpts mo;
  std::string tree = "delila";
  std::string calib_path;
  OffsetTable calib;
  bool has_calib = false;
  int sign = 1;
  double dominance = 0.5, margin = 20;
};

// Options shared by every entry point (+ `extra` keys).
inline bool parse_common(const char* options, const std::set<std::string>& extra, Common& c,
                         std::string& err) {
  std::set<std::string> allowed = {"bin", "nsigma", "search", "sb", "cover", "multi", "pairing",
                                   "peak", "calib", "tree"};
  allowed.insert(extra.begin(), extra.end());
  Options o;
  if (!parse_options(options ? options : "", allowed, o, err)) return false;
  if (!opt_num(o, "bin", 2.0, 0.01, 1000, c.mo.bin, err)) return false;
  if (!opt_num(o, "nsigma", 5.0, 0.0, 1000, c.mo.peak.nsigma, err)) return false;
  if (!opt_num(o, "search", 0.5, 0.01, 1.0, c.mo.peak.search, err)) return false;
  if (!opt_num(o, "sb", 0.75, 0.0, 0.99, c.mo.peak.sb, err)) return false;
  if (!opt_num(o, "cover", 0.9, 0.1, 1.0, c.mo.peak.cover, err)) return false;
  if (!opt_num(o, "multi", 0.5, 0.0, 1e9, c.mo.peak.multi, err)) return false;
  if (c.mo.peak.sb < c.mo.peak.search) {
    err = "sb must be >= search (side bands outside the search region)";
    return false;
  }
  std::string pairing = o.str("pairing", "all");
  if (pairing != "all" && pairing != "nearest") {
    err = "pairing must be all or nearest";
    return false;
  }
  c.mo.nearest = pairing == "nearest";
  if (!parse_peak_value(o.str("peak", "median"), c.mo.value)) {
    err = "peak must be mode, median or centroid";
    return false;
  }
  c.tree = o.str("tree", "delila");
  double s = 1;
  if (!opt_num(o, "sign", 1, -1, 1, s, err)) return false;
  if (s != 1 && s != -1) {
    err = "sign must be 1 or -1";
    return false;
  }
  c.sign = static_cast<int>(s);
  if (!opt_num(o, "dominance", 0.5, 0.0, 1.0, c.dominance, err)) return false;
  if (!opt_num(o, "margin", 20, 0.0, 1e6, c.margin, err)) return false;
  c.calib_path = o.str("calib", "");
  if (!c.calib_path.empty()) {
    std::string text;
    if (!read_text(c.calib_path, text, err)) return false;
    OffsetParseResult pr = parse_time_offsets(text);
    if (!pr.errors.empty()) {
      err = c.calib_path + ": " + pr.errors.front() +
            (pr.errors.size() > 1 ? " (+" + std::to_string(pr.errors.size() - 1) + " more)" : "");
      return false;
    }
    c.calib = pr.table;
    c.has_calib = true;
  }
  return true;
}

// Read the hits (only `keep` channels when non-null), apply the calib table,
// sort per channel. Prints one summary line.
inline bool read_hits(const char* pattern, const Common& c, const std::set<uint16_t>* keep,
                      HitTimes& ht, const char* who) {
  TChain ch(c.tree.c_str());
  int nfiles = ch.Add(pattern);
  Long64_t n = ch.GetEntries();
  if (nfiles == 0 || n <= 0) {
    std::printf("%s: no '%s' entries in %s\n", who, c.tree.c_str(), pattern);
    return false;
  }
  bool aligned_input = false;
  {
    ch.LoadTree(0);
    TTree* t0 = ch.GetTree();
    TNamed* ts = t0 ? dynamic_cast<TNamed*>(t0->GetUserInfo()->FindObject("timeSettings")) : nullptr;
    if (ts && std::string(ts->GetTitle()).find("\"applied\":true") != std::string::npos)
      aligned_input = true;
  }
  UChar_t mod = 0, chn = 0;
  Double_t ts = 0;
  ch.SetBranchStatus("*", false);
  ch.SetBranchStatus("module", true);
  ch.SetBranchStatus("channel", true);
  ch.SetBranchStatus("timestamp_ns", true);
  ch.SetBranchAddress("module", &mod);
  ch.SetBranchAddress("channel", &chn);
  ch.SetBranchAddress("timestamp_ns", &ts);
  std::set<uint16_t> no_calib;
  for (Long64_t i = 0; i < n; ++i) {
    ch.GetEntry(i);
    uint16_t k = mc_key(mod, chn);
    if (keep && !keep->count(k)) continue;
    double t = ts;
    if (c.has_calib) {
      auto it = c.calib.abs_ns.find(k);
      if (it != c.calib.abs_ns.end())
        t -= it->second;
      else
        no_calib.insert(k);
    }
    ht.add(k, t);
  }
  ch.ResetBranchAddresses();
  ht.sort_all();
  std::printf("%s: %s — %d file(s), %lld entries, %lld kept on %zu channels, span %.3f s\n", who,
              pattern, nfiles, static_cast<long long>(n), ht.n, ht.t.size(), ht.span_s());
  if (aligned_input)
    std::printf("  NOTE: input already ALIGNED by root_sink --time-calib: the offsets below are "
                "RESIDUALS on top of that table\n");
  if (c.has_calib) {
    std::printf("  aligned first with %s (%s form, %zu entries)", c.calib_path.c_str(),
                c.calib.format.c_str(), c.calib.abs_ns.size());
    if (!no_calib.empty()) {
      std::printf("; NO entry (raw time kept) for:");
      for (uint16_t k : no_calib) std::printf(" %s", mc_name(k).c_str());
    }
    std::printf("\n");
  }
  return ht.n > 0;
}

inline bool write_text(const char* path, const std::string& text, const char* who) {
  if (!path || !path[0]) return true;
  FILE* f = std::fopen(path, "w");
  if (!f) {
    std::printf("%s: cannot write %s\n", who, path);
    return false;
  }
  std::fputs(text.c_str(), f);
  std::fclose(f);
  std::printf("%s: wrote %s\n", who, path);
  return true;
}

inline std::string status_of(const Peak& p) {
  if (p.significant) return p.note.empty() ? "ok" : "ok; " + p.note;
  if (!p.candidate) return "NO PEAK" + (p.note.empty() ? "" : " (" + p.note + ")");
  return "NO PEAK (" + p.note + ")";
}

inline void print_peak_header() {
  std::printf("  %-8s %9s %8s %8s %9s %9s %9s %7s %19s %16s %8s  %s\n", "channel", "hits",
              "rate/kHz", "bg/bin", "mode", "median", "centroid", "FWHM", "central [lo, hi]",
              "signal", "z", "status");
}

inline void print_peak_row(const std::string& name, long long hits, double span_s, const Peak& p) {
  char cen[64], sig[48];
  std::snprintf(cen, sizeof cen, "[%+8.1f, %+8.1f]", p.q_lo, p.q_hi);
  std::snprintf(sig, sizeof sig, "%.0f ± %.0f", p.signal, p.signal_err);
  if (p.candidate)
    std::printf("  %-8s %9lld %8.2f %8.2f %+9.2f %+9.2f %+9.2f %7.1f %19s %16s %8.1f  %s\n",
                name.c_str(), hits, span_s > 0 ? hits / span_s * 1e-3 : 0.0, p.bg_per_bin, p.mode,
                p.median, p.centroid, p.fwhm, cen, sig, p.z, status_of(p).c_str());
  else
    std::printf("  %-8s %9lld %8.2f %8.2f %9s %9s %9s %7s %19s %16s %8s  %s\n", name.c_str(), hits,
                span_s > 0 ? hits / span_s * 1e-3 : 0.0, p.bg_per_bin, "-", "-", "-", "-", "-", "-",
                "-", status_of(p).c_str());
}

inline void print_settings(const Common& c, double W) {
  std::printf("  window ±%.0f ns, bin %.2f ns, search |dt| < %.0f ns, side bands |dt| >= %.0f ns, "
              "pairing %s, z >= %.1f, offset = %s\n",
              W, c.mo.bin, c.mo.peak.search * W, c.mo.peak.sb * W, c.mo.nearest ? "nearest" : "all",
              c.mo.peak.nsigma, peak_value_name(c.mo.value));
}

// Board table: median of the significant channel peaks (or the summed
// histogram when no channel is significant), relative to the ref board.
inline void print_boards(const std::map<int, BoardDet>& bd, int ref_module, PeakValue v) {
  const double r = bd.count(ref_module) ? bd.at(ref_module).median : 0.0;
  std::printf("\n  per board (module), relative to the ref board m%d: offset = median of the "
              "significant channel %ss\n  (no significant channel -> the board's SUMMED histogram); "
              "summed-histogram peak shown for every board\n", ref_module, peak_value_name(v));
  std::printf("  %-6s %9s %10s %21s %8s  | %-36s %7s %7s\n", "board", "sig/chans", "offset",
              "channel spread", "source", "summed: mode / median / centroid", "FWHM", "z");
  for (const auto& kv : bd) {
    const BoardDet& d = kv.second;
    char off[24] = "-", spr[48] = "-", sum[64] = "-", fw[16] = "-", z[16] = "-";
    if (d.ok) {
      std::snprintf(off, sizeof off, "%+10.2f", d.median - r);
      if (!d.from_sum) std::snprintf(spr, sizeof spr, "[%+7.1f .. %+7.1f]", d.min - r, d.max - r);
    }
    if (d.sum.candidate) {
      std::snprintf(sum, sizeof sum, "%+9.1f / %+9.1f / %+9.1f%s", d.sum.mode - r, d.sum.median - r,
                    d.sum.centroid - r, d.sum.significant ? "" : " n.s.");
      std::snprintf(fw, sizeof fw, "%7.1f", d.sum.fwhm);
      std::snprintf(z, sizeof z, "%7.1f", d.sum.z);
    }
    std::printf("  m%-5d %4d/%-4d %10s %21s %8s  | %-36s %7s %7s\n", d.module, d.n_sig, d.n_chan, off,
                spr, !d.ok ? "NONE" : d.from_sum ? "SUM" : "median", sum, fw, z);
  }
}

}  // namespace tmc

// ===========================================================================
// 1. per-channel offsets
// ===========================================================================
void time_map_offsets(const char* file, const char* ref = "0:0", double window_ns = 2000,
                      const char* out_json = "", const char* options = "") {
  using namespace tmc;
  const char* who = "time_map_offsets";
  Common c;
  std::string err;
  uint16_t rk = 0;
  if (!parse_common(options, {}, c, err) || !parse_mc(ref ? ref : "", rk) || !(window_ns > 0)) {
    std::printf("%s: %s\n", who, err.empty() ? "bad ref (M:C) or window_ns" : err.c_str());
    return;
  }
  HitTimes ht;
  if (!read_hits(file, c, nullptr, ht, who)) return;
  if (ht.hits(rk) == 0) {
    std::printf("%s: reference %s has no hits\n", who, mc_name(rk).c_str());
    return;
  }
  const double span = ht.span_s();
  std::printf("  reference %s: %lld hits (%.2f kHz); dt = t(ch) - t(ref), positive = channel LATE\n",
              mc_name(rk).c_str(), ht.hits(rk), ht.hits(rk) / span * 1e-3);
  print_settings(c, window_ns);
  std::map<int, Peak> sums;
  std::vector<ChannelPeak> cps = measure_vs_ref(ht, rk, window_ns, c.mo, {}, &sums);
  std::printf("\n");
  print_peak_header();
  std::vector<std::string> none;
  for (const ChannelPeak& cp : cps) {
    print_peak_row(mc_name(cp.key), cp.hits, span, cp.p);
    if (!cp.p.significant) none.push_back(mc_name(cp.key));
  }
  print_boards(board_detector_offsets(cps, rk, c.mo.value, &sums), rk >> 8, c.mo.value);
  std::printf("\n  %zu/%zu channels with a significant peak", cps.size() - none.size(), cps.size());
  if (!none.empty()) {
    std::printf("; NO significant peak (%zu):", none.size());
    for (const std::string& s : none) std::printf(" %s", s.c_str());
  }
  std::printf("\n");
  char buf[512];
  std::snprintf(buf, sizeof buf,
                "time_map_offsets on %s: ref %s, window ±%.0f ns, bin %.2f ns, pairing %s, offset = %s "
                "of the background-subtracted dt = t(ch) - t(ref), z >= %.1f",
                file, mc_name(rk).c_str(), window_ns, c.mo.bin, c.mo.nearest ? "nearest" : "all",
                peak_value_name(c.mo.value), c.mo.peak.nsigma);
  std::vector<std::string> com = {buf};
  if (c.has_calib)
    com.push_back("hits aligned with " + c.calib_path +
                  " before measuring: offset_ns = input offset (relative to the ref) + residual");
  std::string js = offsets_tree_json(cps, rk, c.mo.value, c.has_calib ? &c.calib : nullptr, com);
  if (out_json && out_json[0]) write_text(out_json, js, who);
}

// ===========================================================================
// 2. board plan
// ===========================================================================
void time_map_boards(const char* file, const char* ref = "0:0", double window_ns = 2000,
                     const char* pulser_spec = "", const char* step_spec = "",
                     const char* out_json = "", const char* options = "") {
  using namespace tmc;
  const char* who = "time_map_boards";
  Common c;
  std::string err;
  uint16_t rk = 0;
  StepSpec steps;
  if (!parse_common(options, {"sign"}, c, err) || !parse_steps(step_spec ? step_spec : "", steps, err) ||
      !parse_mc(ref ? ref : "", rk) || !(window_ns > 0)) {
    std::printf("%s: %s\n", who, err.empty() ? "bad ref (M:C) or window_ns" : err.c_str());
    return;
  }
  HitTimes ht;
  if (!read_hits(file, c, nullptr, ht, who)) return;
  std::map<int, uint16_t> pulsers;
  if (!parse_pulsers(pulser_spec ? pulser_spec : "", ht.keys(), pulsers, err)) {
    std::printf("%s: pulser_spec: %s\n", who, err.c_str());
    return;
  }
  for (const auto& kv : pulsers)
    if (kv.second == rk) {
      std::printf("%s: the reference %s is a pulser channel — choose a detector as ref\n", who,
                  mc_name(rk).c_str());
      return;
    }
  if (ht.hits(rk) == 0) {
    std::printf("%s: reference %s has no hits\n", who, mc_name(rk).c_str());
    return;
  }
  const double span = ht.span_s();
  std::printf("  reference %s (board m%d): %lld hits; dt = t(ch) - t(ref), positive = LATE\n",
              mc_name(rk).c_str(), rk >> 8, ht.hits(rk));
  print_settings(c, window_ns);
  std::printf("  pulsers%s:", (pulser_spec && pulser_spec[0]) ? "" : " (last channel of each module)");
  for (const auto& kv : pulsers) std::printf(" %s", mc_name(kv.second).c_str());
  std::printf("\n  sign %+d: %s\n", c.sign,
              c.sign > 0 ? "start delay +D -> that board's timestamps -D (CAEN semantics, see top)"
                         : "start delay +D -> that board's timestamps +D (sign=-1 override)");
  BoardsPlan bp = plan_boards(ht, rk, window_ns, pulsers, steps, c.sign, c.mo);

  std::printf("\n  detector channels (pulsers excluded):\n");
  print_peak_header();
  for (const ChannelPeak& cp : bp.chans) print_peak_row(mc_name(cp.key), cp.hits, span, cp.p);
  std::printf("\n  pulser dt = t(pulser) - t(pulser of the ref board m%d):\n", bp.ref_module);
  print_peak_header();
  for (const BoardRow& r : bp.rows)
    if (r.has_pulser && r.module != bp.ref_module && r.pulser_hits > 0)
      print_peak_row(mc_name(r.pulser), r.pulser_hits, span, r.pulser_peak);

  std::printf("\n  BOARD PLAN (relative to the ref board m%d; delays in ns)\n", bp.ref_module);
  std::printf("  %-6s %4s %9s %6s %10s %21s %8s %10s %10s %6s %8s %8s %9s %9s %11s\n", "board",
              "step", "det sig/n", "src", "det off", "det spread", "pulser", "pulser dt", "pul-det",
              "steps", "Δdelay", "Δ(>=0)", "ts shift", "residual", "pulser exp");
  for (const BoardRow& r : bp.rows) {
    char det[32] = "-", spread[48] = "-", pd[32] = "-", pmd[32] = "-", res[32] = "-", pexp[32] = "-";
    if (r.det.ok) {
      std::snprintf(det, sizeof det, "%+10.2f", r.det_rel);
      if (!r.det.from_sum)
        std::snprintf(spread, sizeof spread, "[%+7.1f..%+7.1f]", r.det.min - r.det.median + r.det_rel,
                      r.det.max - r.det.median + r.det_rel);
      std::snprintf(res, sizeof res, "%+9.2f", r.residual);
    }
    if (r.pulser_ok) {
      std::snprintf(pd, sizeof pd, "%+10.2f", r.pulser_dt);
      std::snprintf(pexp, sizeof pexp, "%+11.2f", r.expected_pulser_after);
      if (r.det.ok) std::snprintf(pmd, sizeof pmd, "%+10.2f", r.pulser_dt - r.det_rel);
    }
    std::string pul = r.has_pulser ? mc_name(r.pulser) : "-";
    std::printf("  m%-5d %4d %4d/%-4d %6s %10s %21s %8s %10s %10s %+6lld %+8.0f %8.0f %+9.0f %9s %11s\n",
                r.module, r.step, r.det.n_sig, r.det.n_chan,
                !r.det.ok ? "-" : r.det.from_sum ? "SUM" : "median", det, spread, pul.c_str(), pd, pmd,
                r.n_steps, r.delay_change + 0.0, r.delay_nonneg, r.ts_shift + 0.0, res, pexp);
  }
  std::printf("  Δdelay = sign*step*round(det off/step): change of start_delay (DIG1) / rundelay (DIG2)"
              "; Δ(>=0) = the same plan + a common constant (no negative change).\n"
              "  pulser exp = pulser dt expected after the change (time_map_verify checks it); "
              "residual = det off left after the change.\n");
  for (const BoardRow& r : bp.rows) {
    if (!r.det.ok)
      std::printf("  WARNING m%d: no significant detector channel nor summed peak — NOT adjusted "
                  "(Δdelay 0)%s\n", r.module, r.pulser_ok ? "; its pulser dt is listed above" : "");
    if (r.det.from_sum)
      std::printf("  NOTE m%d: no single channel significant — offset from the board's SUMMED "
                  "histogram (z %.1f)\n", r.module, r.det.sum.z);
    if (r.det.ok && r.definition_matters())
      std::printf("  WARNING m%d: the peak definition changes the plan — mode %+lld steps (%+.1f ns), "
                  "centroid %+lld (%+.1f), median %+lld (%+.1f); broad/asymmetric peaks (FWHM up to "
                  "%.0f ns). Using peak=%s; median is the robust choice for flat-topped peaks\n",
                  r.module, r.alt_steps[0], r.alt_det[0], r.alt_steps[1], r.alt_det[1], r.alt_steps[2],
                  r.alt_det[2], r.det.max_fwhm, peak_value_name(bp.value));
    if (!r.has_pulser)
      std::printf("  WARNING m%d: no pulser channel — the change cannot be verified\n", r.module);
    else if (r.pulser_hits == 0)
      std::printf("  WARNING m%d: pulser %s has no hits\n", r.module, mc_name(r.pulser).c_str());
    else if (!r.pulser_ok && r.module != bp.ref_module)
      std::printf("  WARNING m%d: no significant pulser peak (%s)\n", r.module,
                  status_of(r.pulser_peak).c_str());
  }
  for (const std::string& w : bp.warnings) std::printf("  WARNING: %s\n", w.c_str());
  char buf[512];
  std::snprintf(buf, sizeof buf, "time_map_boards on %s: ref %s, window ±%.0f ns, bin %.2f ns, z >= %.1f%s",
                file, mc_name(rk).c_str(), window_ns, c.mo.bin, c.mo.peak.nsigma,
                c.has_calib ? (", aligned first with " + c.calib_path).c_str() : "");
  std::string js = boards_json(bp, {buf});
  if (out_json && out_json[0]) write_text(out_json, js, who);
}

// ===========================================================================
// 3. verification run
// ===========================================================================
void time_map_verify(const char* before_file, const char* after_file, const char* applied_json,
                     const char* options = "") {
  using namespace tmc;
  const char* who = "time_map_verify";
  Common c;
  std::string err, text;
  if (!parse_common(options, {}, c, err) || !read_text(applied_json ? applied_json : "", text, err)) {
    std::printf("%s: %s\n", who, err.c_str());
    return;
  }
  AppliedPlan ap;
  std::vector<std::string> errs;
  if (!parse_applied(text, ap, errs)) {
    std::printf("%s: %s is not a time_map_boards plan:\n", who, applied_json);
    for (const std::string& e : errs) std::printf("  - %s\n", e.c_str());
    return;
  }
  // The measurement follows the plan (window, bin, peak value) unless overridden.
  Options o;
  parse_options(options ? options : "", {"bin", "nsigma", "search", "sb", "cover", "multi", "pairing",
                                         "peak", "calib", "tree"},
                o, err);
  if (!o.has("bin")) c.mo.bin = ap.bin;
  if (!o.has("peak")) c.mo.value = ap.value;
  ap.value = c.mo.value;
  uint16_t refp = 0;
  bool have_refp = false;
  std::set<uint16_t> keep;
  for (const auto& b : ap.boards)
    if (b.has_pulser) {
      keep.insert(b.pulser);
      if (b.module == ap.ref_module) {
        refp = b.pulser;
        have_refp = true;
      }
    }
  if (!have_refp) {
    std::printf("%s: the plan has no pulser on the ref board m%d — nothing to verify\n", who, ap.ref_module);
    return;
  }
  std::printf("%s: plan %s (ref board m%d, pulser %s, sign %+d, window ±%.0f ns)\n", who, applied_json,
              ap.ref_module, mc_name(refp).c_str(), ap.sign, ap.W);
  auto measure = [&](const char* f, std::map<int, Peak>& out, double& span) {
    HitTimes ht;
    if (!read_hits(f, c, &keep, ht, who)) return false;
    span = ht.span_s();
    if (ht.hits(refp) == 0) {
      std::printf("%s: %s: ref pulser %s has no hits\n", who, f, mc_name(refp).c_str());
      return false;
    }
    print_peak_header();
    for (const auto& b : ap.boards)
      if (b.has_pulser && b.module != ap.ref_module) {
        out[b.module] = measure_pulser_dt(ht, b.pulser, refp, ap.W, c.mo);
        print_peak_row(mc_name(b.pulser), ht.hits(b.pulser), span, out[b.module]);
      }
    return true;
  };
  std::map<int, Peak> pb, pa;
  double sb = 0, sa = 0;
  const bool have_before = before_file && before_file[0];
  if (have_before) {
    std::printf("\n  BEFORE:\n");
    if (!measure(before_file, pb, sb)) return;
  } else {
    std::printf("\n  BEFORE: pulser_dt_ns stored in the plan\n");
  }
  std::printf("\n  AFTER:\n");
  if (!measure(after_file, pa, sa)) return;
  std::vector<VerifyRow> rows = verify_boards(ap, have_before ? &pb : nullptr, pa);
  double ref_delay = 0;
  for (const auto& b : ap.boards)
    if (b.module == ap.ref_module) ref_delay = b.delay_change;
  std::printf("\n  %-6s %5s %9s %10s %10s %10s %10s %9s %7s  %s\n", "board", "step", "Δdelay",
              "before", "after", "change", "expected", "diff", "tol", "verdict");
  int npass = 0, nfail = 0, nna = 0;
  for (const VerifyRow& v : rows) {
    double dd = 0;
    for (const auto& b : ap.boards)
      if (b.module == v.module) dd = b.delay_change - ref_delay;
    if (v.measured) {
      std::printf("  m%-5d %5d %+9.0f %+10.2f %+10.2f %+10.2f %+10.0f %+9.2f %7.1f  %s\n", v.module,
                  v.step, dd, v.before, v.after, v.change + 0.0, v.expected + 0.0, v.diff + 0.0, 0.5 * v.step,
                  v.pass ? "PASS" : "FAIL");
      (v.pass ? npass : nfail)++;
    } else {
      std::printf("  m%-5d %5d %+9.0f %10s %10s %10s %+10.0f %9s %7.1f  N/A (%s)\n", v.module, v.step,
                  dd, "-", "-", "-", v.expected, "-", 0.5 * v.step, v.note.c_str());
      ++nna;
    }
  }
  std::printf("  expected = -sign * Δdelay (relative to the ref board); PASS when |change - expected| "
              "<= step/2.\n");
  std::printf("\n%s: %s — %d PASS, %d FAIL, %d not measurable\n", who,
              (nfail == 0 && nna == 0) ? "ALL PASS" : (nfail ? "FAIL" : "INCOMPLETE"), npass, nfail, nna);
  if (nfail)
    std::printf("  A FAIL whose change is the expected one with the OPPOSITE sign means the sign "
                "assumption is wrong (re-plan with sign=-1); a whole-clock jump means the board "
                "slipped.\n");
}

// ===========================================================================
// 4. anti-Compton map
// ===========================================================================
void time_map_ac(const char* file, const char* detectors, const char* shields, double window_ns = 1000,
                 const char* out_json = "", const char* options = "") {
  using namespace tmc;
  const char* who = "time_map_ac";
  Common c;
  std::string err;
  if (!parse_common(options, {"dominance", "margin"}, c, err) || !(window_ns > 0)) {
    std::printf("%s: %s\n", who, err.empty() ? "bad window_ns" : err.c_str());
    return;
  }
  HitTimes ht;
  if (!read_hits(file, c, nullptr, ht, who)) return;
  const std::set<uint16_t> present = ht.keys();
  // "@file:tags" specs read the event_def's tags table.
  auto tag_text = [&](const std::string& spec, std::string& text) {
    if (spec.empty() || spec[0] != '@') return true;
    std::size_t colon = spec.rfind(':');
    std::string path = spec.substr(1, colon == std::string::npos ? std::string::npos : colon - 1);
    return read_text(path, text, err);
  };
  std::string dspec = detectors ? detectors : "", sspec = shields ? shields : "", dtext, stext;
  std::vector<Group> groups;
  if (!tag_text(dspec, dtext) || !parse_groups(dspec, present, dtext, groups, err)) {
    std::printf("%s: detectors: %s\n", who, err.c_str());
    return;
  }
  std::vector<uint16_t> sh;
  if (!sspec.empty() && sspec[0] == '@') {
    std::vector<Group> sg;
    if (!tag_text(sspec, stext) || !parse_groups(sspec, present, stext, sg, err)) {
      std::printf("%s: shields: %s\n", who, err.c_str());
      return;
    }
    for (const Group& g : sg) sh.insert(sh.end(), g.chans.begin(), g.chans.end());
  } else if (!parse_mc_list(sspec, present, sh, err)) {
    std::printf("%s: shields: %s\n", who, err.c_str());
    return;
  }
  std::set<uint16_t> shset(sh.begin(), sh.end());
  for (const Group& g : groups)
    for (uint16_t k : g.chans)
      if (shset.count(k)) {
        std::printf("%s: %s is both a detector (group '%s') and a shield\n", who, mc_name(k).c_str(),
                    g.name.c_str());
        return;
      }
  const double span = ht.span_s();
  std::printf("  dt = t(shield) - t(detector); veto = prompt excess / detector hits; assignment: "
              "best significant veto, others < %.2f x best\n", c.dominance);
  print_settings(c, window_ns);
  AcResult r = measure_ac(ht, groups, sh, window_ns, c.mo, c.dominance, c.margin);

  std::printf("\n  detector groups:\n");
  for (std::size_t g = 0; g < groups.size(); ++g) {
    std::printf("  %-10s %9lld hits:", groups[g].name.c_str(), r.group_hits[g]);
    for (uint16_t k : groups[g].chans) std::printf(" %s(%lld)", mc_name(k).c_str(), ht.hits(k));
    std::printf("\n");
  }
  bool one_module = true;
  for (uint16_t k : sh) one_module = one_module && (k >> 8) == (sh[0] >> 8);
  auto col = [&](uint16_t k) {
    return one_module ? "c" + std::to_string(k & 0xFF) : mc_name(k);
  };
  std::printf("\n  VETO MATRIX (%%, significant cells only; * = assigned, ? = ambiguous candidate)%s\n",
              one_module ? (" — shields on m" + std::to_string(sh[0] >> 8)).c_str() : "");
  std::printf("  %-10s", "group");
  for (uint16_t k : sh) std::printf(" %6s", col(k).c_str());
  std::printf("\n");
  for (std::size_t g = 0; g < groups.size(); ++g) {
    std::printf("  %-10s", groups[g].name.c_str());
    for (std::size_t s = 0; s < sh.size(); ++s) {
      const AcCell& x = r.cell[g][s];
      const AcShield& a = r.sh[s];
      char mark = ' ';
      if (a.status == AcStatus::Assigned && a.group == static_cast<int>(g)) mark = '*';
      if (a.status == AcStatus::Ambiguous &&
          std::find(a.candidates.begin(), a.candidates.end(), static_cast<int>(g)) != a.candidates.end())
        mark = '?';
      if (x.sig)
        std::printf(" %5.1f%c", 100 * x.frac, mark);
      else
        std::printf(" %5s%c", ".", mark);
    }
    std::printf("\n");
  }

  std::printf("\n  SHIELD ASSIGNMENT (group-level dt = t(shield) - t(detectors of the group))\n");
  std::printf("  %-7s %9s %8s  %-10s %8s %8s %8s %7s %19s  %s\n", "shield", "hits", "rate/kHz",
              "-> group", "veto %", "z", "mode", "FWHM", "central [lo, hi]", "next best (veto %)");
  int na = 0, namb = 0, nsil = 0, nnh = 0;
  for (std::size_t s = 0; s < sh.size(); ++s) {
    const AcShield& a = r.sh[s];
    std::string next = "-";
    if (a.status == AcStatus::Assigned) {
      // the best other group (significant or not) — how dominant the assignment is
      double best = -1e300;
      int bg = -1;
      for (std::size_t g = 0; g < groups.size(); ++g)
        if (static_cast<int>(g) != a.group && r.cell[g][s].frac > best) {
          best = r.cell[g][s].frac;
          bg = static_cast<int>(g);
        }
      if (bg >= 0)
        next = groups[static_cast<std::size_t>(bg)].name + fmt(" (%.2f", 100 * best) +
               (r.cell[static_cast<std::size_t>(bg)][s].sig ? ", sig)" : ", n.s.)");
      const AcCell& x = r.cell[static_cast<std::size_t>(a.group)][s];
      std::printf("  %-7s %9lld %8.2f  %-10s %8.2f %8.1f %+8.1f %7.1f   [%+7.1f, %+7.1f]  %s\n",
                  mc_name(a.key).c_str(), a.hits, a.hits / span * 1e-3,
                  groups[static_cast<std::size_t>(a.group)].name.c_str(), 100 * x.frac, x.p.z,
                  x.p.mode, x.p.fwhm, x.p.q_lo, x.p.q_hi, next.c_str());
      ++na;
    } else {
      std::string t;
      for (int g : a.candidates)
        t += (t.empty() ? "" : ", ") + groups[static_cast<std::size_t>(g)].name +
             fmt(" %.2f", 100 * r.cell[static_cast<std::size_t>(g)][s].frac);
      std::printf("  %-7s %9lld %8.2f  %-10s %s\n", mc_name(a.key).c_str(), a.hits,
                  span > 0 ? a.hits / span * 1e-3 : 0.0, ac_status_name(a.status),
                  t.empty() ? "" : ("candidates: " + t).c_str());
      if (a.status == AcStatus::Ambiguous) ++namb;
      if (a.status == AcStatus::Silent) ++nsil;
      if (a.status == AcStatus::NoHits) ++nnh;
    }
  }
  std::printf("  %d assigned, %d ambiguous, %d silent, %d without hits\n", na, namb, nsil, nnh);

  std::printf("\n  PER DETECTOR CHANNEL x ASSIGNED SHIELD (channel-level dt peaks)\n");
  std::printf("  %-10s %-8s %-8s %8s %8s %9s %9s %7s %19s\n", "group", "detector", "shield", "veto %",
              "z", "mode", "centroid", "FWHM", "central [lo, hi]");
  std::vector<std::string> unshielded;
  for (std::size_t g = 0; g < groups.size(); ++g) {
    bool any = false;
    for (std::size_t s = 0; s < sh.size(); ++s) {
      if (r.sh[s].status != AcStatus::Assigned || r.sh[s].group != static_cast<int>(g)) continue;
      any = true;
      for (uint16_t d : groups[g].chans) {
        const AcCell& x = r.chan[std::make_pair(d, sh[s])];
        if (x.p.candidate)
          std::printf("  %-10s %-8s %-8s %8.2f %8.1f %+9.1f %+9.1f %7.1f   [%+7.1f, %+7.1f]%s\n",
                      groups[g].name.c_str(), mc_name(d).c_str(), mc_name(sh[s]).c_str(), 100 * x.frac,
                      x.p.z, x.p.mode, x.p.centroid, x.p.fwhm, x.p.q_lo, x.p.q_hi,
                      x.sig ? "" : "  (not significant)");
        else
          std::printf("  %-10s %-8s %-8s  no excess\n", groups[g].name.c_str(), mc_name(d).c_str(),
                      mc_name(sh[s]).c_str());
      }
    }
    if (!any) unshielded.push_back(groups[g].name);
  }
  if (!unshielded.empty()) {
    std::printf("  groups with NO assigned shield:");
    for (const std::string& u : unshielded) std::printf(" %s", u.c_str());
    std::printf("\n");
  }
  if (!r.window_ok) {
    std::printf("\n%s: no shield assigned — no anti_compton block\n", who);
    return;
  }
  std::printf("\n  suggested window: shield in [t_det - %.0f, t_det + %.0f] ns (assigned pairs' central "
              "%.0f %% span [%+.1f, %+.1f] ns + margin %.0f ns)\n",
              r.pre, r.post, 100 * c.mo.peak.cover, r.lo, r.hi, r.margin);
  int npairs = 0;
  for (std::size_t g = 0; g < groups.size(); ++g) {
    bool any = false;
    for (const AcShield& a : r.sh) any = any || (a.status == AcStatus::Assigned && a.group == static_cast<int>(g));
    npairs += any ? 1 : 0;
  }
  if (npairs > 64)
    std::printf("  WARNING: %d pairs > 64 (event_def limit) — merge groups\n", npairs);
  char buf[512];
  std::snprintf(buf, sizeof buf,
                "time_map_ac on %s: window ±%.0f ns, bin %.2f ns, z >= %.1f, dominance %.2f, margin %.0f "
                "ns, central %.0f %%%s",
                file, window_ns, c.mo.bin, c.mo.peak.nsigma, c.dominance, c.margin,
                100 * c.mo.peak.cover, c.has_calib ? (", aligned with " + c.calib_path).c_str() : "");
  std::string js = ac_block_json(r, {buf});
  std::printf("\n%s", js.c_str());
  if (out_json && out_json[0]) write_text(out_json, js, who);
}

// `root time_map.C+` -> usage.
void time_map() {
  std::printf(
      "time_map.C — time offsets, board start-delay plan, pulser verification, anti-Compton map\n"
      "  time_map_offsets(file, \"M:C\" ref, window_ns=2000, out_json=\"\", options=\"\")\n"
      "  time_map_boards(file, \"M:C\" ref, window_ns=2000, pulser_spec=\"\", step_spec=\"\", out_json=\"\", "
      "options=\"\")\n"
      "  time_map_verify(before_file, after_file, applied_json, options=\"\")\n"
      "  time_map_ac(file, detectors, shields, window_ns=1000, out_json=\"\", options=\"\")\n"
      "  options: bin= nsigma= search= sb= cover= multi= pairing=all|nearest peak=median|mode|centroid "
      "calib= tree= sign= dominance= margin=\n"
      "  see the comment block at the top of time_map.C\n");
}

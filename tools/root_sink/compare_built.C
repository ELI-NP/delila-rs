// compare_built.C — TODO 71 Phase B gate: rebuild the ThGEM `built` rows from
// the generic `events` tree (eb_core.hpp thgem_from_event) and compare them
// row by row with the `built` tree the chunk builder wrote into the SAME run
// file (root_sink --built-tree built --event-def ...).
//
//   root -l -b -q 'compare_built.C+("run.root","1,2,3,5,4","6,7,8,9,10",0,500)'
//   args: file, --xy1-ch, --xy2-ch ("" = single plane), --gamma-ch (-1 = none),
//         --window-ns (the built tree's window)
//
// Rows are matched on (trig1_t, trig2_t). Expected differences, by
// construction: a hit contested by two events is used by ONE row here (the
// chunk builder may use it twice), two triggers of the same plane in one group
// are one row here, and arms/LaBr3 outside the event window are invisible here
// — equality needs pre, post >= group_ns + window_ns (see eb_core.hpp §8).
// Prints both 4-fold counts (n_arms1 == 4), IDENTICAL or the differing rows.
#include <cmath>
#include <cstdio>
#include <algorithm>
#include <map>
#include <memory>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

#include "TFile.h"
#include "TTree.h"

#include "eb_core.hpp"

namespace {
std::vector<int> parse_ch(const char* s) {
  std::vector<int> v;
  std::stringstream ss(s ? s : "");
  std::string tok;
  while (std::getline(ss, tok, ','))
    if (!tok.empty()) v.push_back(std::stoi(tok));
  return v;
}
double tkey(double t) { return std::isnan(t) ? -1e300 : t; }
bool same_f(float a, float b) { return (std::isnan(a) && std::isnan(b)) || a == b; }
bool same_d(double a, double b) { return (std::isnan(a) && std::isnan(b)) || a == b; }
}  // namespace

void compare_built(const char* path, const char* xy1 = "1,2,3,5,4", const char* xy2 = "6,7,8,9,10",
                   int gamma = 0, double window = 1000.0) {
  using namespace rootsink;
  eb::BuilderConfig cfg;
  std::vector<int> p1 = parse_ch(xy1), p2 = parse_ch(xy2);
  if (p1.size() != 5 || (!p2.empty() && p2.size() != 5)) {
    printf("compare_built: xy lists must be 5 channels 'T,XL,XR,YU,YD'\n");
    return;
  }
  cfg.trig1 = p1[0]; cfg.xl1 = p1[1]; cfg.xr1 = p1[2]; cfg.yu1 = p1[3]; cfg.yd1 = p1[4];
  if (!p2.empty()) {
    cfg.trig2 = p2[0]; cfg.xl2 = p2[1]; cfg.xr2 = p2[2]; cfg.yu2 = p2[3]; cfg.yd2 = p2[4];
  }
  cfg.labr_ch = gamma;
  cfg.window_ns = window;

  TFile f(path, "READ");
  if (f.IsZombie()) return;
  TTree* bt = f.Get<TTree>("built");
  TTree* et = f.Get<TTree>("events");
  if (!bt || !et) {
    printf("compare_built: need trees 'built' and 'events' in %s\n", path);
    return;
  }

  // --- legacy rows, keyed on the trigger times ---
  eb::BuiltEvent b;
  bt->SetBranchAddress("trig1_t", &b.trig1_t);
  bt->SetBranchAddress("trig2_t", &b.trig2_t);
  bt->SetBranchAddress("labr_t", &b.labr_t);
  bt->SetBranchAddress("x1", &b.x1);
  bt->SetBranchAddress("y1", &b.y1);
  bt->SetBranchAddress("x2", &b.x2);
  bt->SetBranchAddress("y2", &b.y2);
  bt->SetBranchAddress("dt_trig", &b.dt_trig);
  bt->SetBranchAddress("tof1", &b.tof1);
  bt->SetBranchAddress("tof2", &b.tof2);
  bt->SetBranchAddress("e_sum1", &b.e_sum1);
  bt->SetBranchAddress("e_sum2", &b.e_sum2);
  bt->SetBranchAddress("e_labr", &b.e_labr);
  bt->SetBranchAddress("n_arms1", &b.n_arms1);
  bt->SetBranchAddress("n_arms2", &b.n_arms2);
  std::map<std::pair<double, double>, eb::BuiltEvent> legacy;
  long long legacy_4fold = 0;
  for (Long64_t i = 0; i < bt->GetEntries(); ++i) {
    bt->GetEntry(i);
    legacy[{tkey(b.trig1_t), tkey(b.trig2_t)}] = b;
    if (b.n_arms1 == 4) ++legacy_4fold;
  }

  // --- generic events -> rows ---
  Double_t tt = 0;
  UChar_t tmod = 0, tch = 0, tidx = 0;
  UInt_t ntrig = 0, nhits = 0;
  const std::size_t cap = static_cast<std::size_t>(std::max(1.0, et->GetMaximum("NHits")));
  std::vector<UChar_t> hmod(cap), hch(cap);
  std::vector<UShort_t> he(cap), hes(cap);
  std::vector<Double_t> htime(cap);
  std::unique_ptr<Bool_t[]> htrig(new Bool_t[cap]);
  et->SetBranchAddress("TriggerTime", &tt);
  et->SetBranchAddress("TriggerMod", &tmod);
  et->SetBranchAddress("TriggerCh", &tch);
  et->SetBranchAddress("TriggerIdx", &tidx);
  et->SetBranchAddress("NTriggers", &ntrig);
  et->SetBranchAddress("NHits", &nhits);
  et->SetBranchAddress("HitMod", hmod.data());
  et->SetBranchAddress("HitCh", hch.data());
  et->SetBranchAddress("HitEnergy", he.data());
  et->SetBranchAddress("HitEnergyShort", hes.data());
  et->SetBranchAddress("HitTime", htime.data());
  et->SetBranchAddress("HitIsTrigger", htrig.get());

  long long rows = 0, mine_4fold = 0, identical = 0, differing = 0, unmatched = 0;
  for (Long64_t i = 0; i < et->GetEntries(); ++i) {
    et->GetEntry(i);
    eb::GenericEvent ev;
    ev.t = tt;
    ev.mod = tmod;
    ev.ch = tch;
    ev.trig_idx = tidx;
    ev.n_triggers = ntrig;
    ev.hits.resize(nhits);
    for (UInt_t j = 0; j < nhits; ++j) {
      ev.hits[j].mod = hmod[j];
      ev.hits[j].ch = hch[j];
      ev.hits[j].energy = he[j];
      ev.hits[j].energy_short = hes[j];
      ev.hits[j].dt = htime[j];
      ev.hits[j].is_trigger = htrig[j];
    }
    eb::BuiltEvent m;
    if (!eb::thgem_from_event(ev, cfg, m)) continue;
    ++rows;
    if (m.n_arms1 == 4) ++mine_4fold;
    auto it = legacy.find({tkey(m.trig1_t), tkey(m.trig2_t)});
    if (it == legacy.end()) {
      if (unmatched < 5)
        printf("  no legacy row for trig1_t %.3f trig2_t %.3f (NTriggers %u)\n", m.trig1_t,
               m.trig2_t, ntrig);
      ++unmatched;
      continue;
    }
    const eb::BuiltEvent& l = it->second;
    bool ok = same_f(m.x1, l.x1) && same_f(m.y1, l.y1) && same_f(m.x2, l.x2) &&
              same_f(m.y2, l.y2) && same_f(m.dt_trig, l.dt_trig) && same_f(m.tof1, l.tof1) &&
              same_f(m.tof2, l.tof2) && same_d(m.labr_t, l.labr_t) && m.e_sum1 == l.e_sum1 &&
              m.e_sum2 == l.e_sum2 && m.e_labr == l.e_labr && m.n_arms1 == l.n_arms1 &&
              m.n_arms2 == l.n_arms2;
    if (ok) {
      ++identical;
    } else {
      if (differing < 8)
        printf("  DIFF trig1_t %.3f: n_arms1 %u/%u n_arms2 %u/%u x1 %g/%g y1 %g/%g labr %.3f/%.3f "
               "(NTriggers %u, NHits %u)\n",
               m.trig1_t, m.n_arms1, l.n_arms1, m.n_arms2, l.n_arms2, m.x1, l.x1, m.y1, l.y1,
               m.labr_t, l.labr_t, ntrig, nhits);
      ++differing;
    }
  }
  printf("compare_built: %s window %.0f ns\n", path, window);
  printf("  legacy built rows %lld (4-fold %lld) | from events %lld (4-fold %lld)\n",
         static_cast<long long>(legacy.size()), legacy_4fold, rows, mine_4fold);
  printf("  identical %lld, differing %lld, no legacy row %lld, legacy rows not rebuilt %lld\n",
         identical, differing, unmatched,
         static_cast<long long>(legacy.size()) - identical - differing);
  const bool all = differing == 0 && unmatched == 0 &&
                   identical == static_cast<long long>(legacy.size());
  printf("compare_built: %s\n", all ? "IDENTICAL" : "DIFFERENT (see expected causes in the header)");
}

// thgem_events.C — rebuild the ThGEM `built` rows from a run's generic `events`
// tree (TODO 71: the in-sink built tree was retired; the experiment-specific
// reconstruction runs offline, on top of the experiment-independent events).
//
//   root -l -b -q 'thgem_events.C+("run0056_0000_X.root","run0056_0000_X_built.root")'
//   args: input run file (with the `events` tree), output file,
//         --xy1-ch "T,XL,XR,YU,YD", --xy2-ch ("" = single plane),
//         --gamma-ch (-1 = no LaBr3/TOF), window_ns (arm/LaBr3 match window)
//
// The output holds a TTree `built` with exactly the branches root_sink used to
// write (trig1_t … n_arms2; NaN = absent piece, completeness is an offline
// cut), so delayline_offsets.C's delayline_centres_built() and the existing
// analysis macros read it unchanged. One row per event that holds a ThGEM
// trigger (eb::thgem_from_event, thgem_reco.hpp). The input's UserInfo
// (timeSettings, eventDef) is copied so the output says which time axis and
// which event definition it came from.
//
// Equal to the retired sink tree row by row when the event window covers
// group_ns + window_ns on both sides and no hit is contested (TODO 71 §9:
// run0023 15,644 four-fold / 40,045 rows, run 56 33,463 rows). Two triggers of
// the same plane in one event make one row (the earliest).
#include <cmath>
#include <cstdio>
#include <memory>
#include <sstream>
#include <string>
#include <vector>

#include "TFile.h"
#include "TList.h"
#include "TNamed.h"
#include "TTree.h"

#include "thgem_reco.hpp"

namespace {
std::vector<int> thgem_parse_ch(const char* s) {
  std::vector<int> v;
  std::stringstream ss(s ? s : "");
  std::string tok;
  while (std::getline(ss, tok, ','))
    if (!tok.empty()) v.push_back(std::stoi(tok));
  return v;
}
}  // namespace

void thgem_events(const char* in_path, const char* out_path, const char* xy1 = "1,2,3,5,4",
                  const char* xy2 = "6,7,8,9,10", int gamma = 0, double window = 1000.0) {
  using namespace rootsink;
  eb::BuilderConfig cfg;
  std::vector<int> p1 = thgem_parse_ch(xy1), p2 = thgem_parse_ch(xy2);
  if (p1.size() != 5 || (!p2.empty() && p2.size() != 5)) {
    printf("thgem_events: xy lists must be 5 channels 'T,XL,XR,YU,YD'\n");
    return;
  }
  cfg.trig1 = p1[0]; cfg.xl1 = p1[1]; cfg.xr1 = p1[2]; cfg.yu1 = p1[3]; cfg.yd1 = p1[4];
  if (!p2.empty()) {
    cfg.trig2 = p2[0]; cfg.xl2 = p2[1]; cfg.xr2 = p2[2]; cfg.yu2 = p2[3]; cfg.yd2 = p2[4];
  }
  cfg.labr_ch = gamma;
  cfg.window_ns = window;

  TFile fin(in_path, "READ");
  if (fin.IsZombie()) return;
  TTree* et = fin.Get<TTree>("events");
  if (!et) {
    printf("thgem_events: no 'events' tree in %s (record with --event-def)\n", in_path);
    return;
  }
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

  TFile fout(out_path, "RECREATE", "", 505);
  TTree bt("built", "ThGEM rows rebuilt from the generic events (thgem_events.C, NaN = absent)");
  eb::BuiltEvent b;
  bt.Branch("trig1_t", &b.trig1_t, "trig1_t/D");
  bt.Branch("trig2_t", &b.trig2_t, "trig2_t/D");
  bt.Branch("labr_t", &b.labr_t, "labr_t/D");
  bt.Branch("x1", &b.x1, "x1/F");
  bt.Branch("y1", &b.y1, "y1/F");
  bt.Branch("x2", &b.x2, "x2/F");
  bt.Branch("y2", &b.y2, "y2/F");
  bt.Branch("dt_xl1", &b.dt_xl1, "dt_xl1/F");
  bt.Branch("dt_xr1", &b.dt_xr1, "dt_xr1/F");
  bt.Branch("dt_yu1", &b.dt_yu1, "dt_yu1/F");
  bt.Branch("dt_yd1", &b.dt_yd1, "dt_yd1/F");
  bt.Branch("dt_xl2", &b.dt_xl2, "dt_xl2/F");
  bt.Branch("dt_xr2", &b.dt_xr2, "dt_xr2/F");
  bt.Branch("dt_yu2", &b.dt_yu2, "dt_yu2/F");
  bt.Branch("dt_yd2", &b.dt_yd2, "dt_yd2/F");
  bt.Branch("dt_trig", &b.dt_trig, "dt_trig/F");
  bt.Branch("tof1", &b.tof1, "tof1/F");
  bt.Branch("tof2", &b.tof2, "tof2/F");
  bt.Branch("e_trig1", &b.e_trig1, "e_trig1/s");
  bt.Branch("e_trig2", &b.e_trig2, "e_trig2/s");
  bt.Branch("e_labr", &b.e_labr, "e_labr/s");
  bt.Branch("e_xl1", &b.e_xl1, "e_xl1/s");
  bt.Branch("e_xr1", &b.e_xr1, "e_xr1/s");
  bt.Branch("e_yu1", &b.e_yu1, "e_yu1/s");
  bt.Branch("e_yd1", &b.e_yd1, "e_yd1/s");
  bt.Branch("e_xl2", &b.e_xl2, "e_xl2/s");
  bt.Branch("e_xr2", &b.e_xr2, "e_xr2/s");
  bt.Branch("e_yu2", &b.e_yu2, "e_yu2/s");
  bt.Branch("e_yd2", &b.e_yd2, "e_yd2/s");
  bt.Branch("e_sum1", &b.e_sum1, "e_sum1/i");
  bt.Branch("e_sum2", &b.e_sum2, "e_sum2/i");
  bt.Branch("n_arms1", &b.n_arms1, "n_arms1/b");
  bt.Branch("n_arms2", &b.n_arms2, "n_arms2/b");
  for (TObject* o : *et->GetUserInfo())
    if (auto* n = dynamic_cast<TNamed*>(o)) bt.GetUserInfo()->Add(new TNamed(*n));

  long long rows = 0, four1 = 0, four2 = 0, both = 0;
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
    if (!eb::thgem_from_event(ev, cfg, b)) continue;
    bt.Fill();
    ++rows;
    if (b.n_arms1 == 4) ++four1;
    if (b.n_arms2 == 4) ++four2;
    if (!std::isnan(b.dt_trig)) ++both;
  }
  fout.cd();
  bt.Write();
  printf("thgem_events: %s -> %s: %lld events -> %lld built rows (4-fold plane1 %lld, "
         "plane2 %lld, both planes %lld; window %.0f ns)\n",
         in_path, out_path, static_cast<long long>(et->GetEntries()), rows, four1, four2, both,
         window);
}

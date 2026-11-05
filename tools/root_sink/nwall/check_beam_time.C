// check_beam_time.C — verify BeamTime and rfPeriodNs of a finished nwall_sink run
// against the hits tree of the same file, and show the TOF they give.
//
//   root -l -b -q 'check_beam_time.C+("run0008_0000_N_Wall.root", 15.0, 0)'
//      (file, si_delay [ns], beam module, beam channel = 4th argument, default 0)
//
// Every event: BeamTime must be exactly the beam hit nearest to TriggerTime, or -1
// when none is within 1 ms. Then the bunch of the event and the TOF of its LS hits
// the way an analysis gets them from the events tree alone:
//   tb  = BeamTime + round((TriggerTime - si_delay - BeamTime) / T) * T
//   TOF = TriggerTime + HitTime[i] - tb
// The gamma peak of that TOF gives the offset between the beam signal and the
// bunch (rf_t0 + L/c). A run written without --channel-map has BeamTime = -1
// although its hits tree holds beam hits: it fails here, by design.
//
// License: BSD-3-Clause (same as delila-rs).

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

#include "TFile.h"
#include "TH1D.h"
#include "TList.h"
#include "TParameter.h"
#include "TTree.h"

int check_beam_time(const char* path, double si_delay = 0.0, int beam_mod = 0, int beam_ch = 0,
                    int ls_mod = 0, int ls_ch_min = 8, int ls_ch_max = 31) {
  TFile f(path);
  if (f.IsZombie()) return 2;
  TTree* hits = dynamic_cast<TTree*>(f.Get("hits"));
  if (!hits) hits = dynamic_cast<TTree*>(f.Get("delila"));
  TTree* ev = dynamic_cast<TTree*>(f.Get("events"));
  if (!hits || !ev) {
    std::printf("FAIL: %s needs a hits tree and an events tree\n", path);
    return 2;
  }

  UChar_t mod = 0, ch = 0;
  Double_t ts = 0;
  hits->SetBranchStatus("*", false);
  for (const char* b : {"module", "channel", "timestamp_ns"}) hits->SetBranchStatus(b, true);
  hits->SetBranchAddress("module", &mod);
  hits->SetBranchAddress("channel", &ch);
  hits->SetBranchAddress("timestamp_ns", &ts);
  std::vector<double> beams;
  for (Long64_t i = 0; i < hits->GetEntries(); ++i) {
    hits->GetEntry(i);
    if (mod == beam_mod && ch == beam_ch) beams.push_back(ts);
  }
  std::sort(beams.begin(), beams.end());

  double T = 0.0;
  for (TTree* t : {hits, ev}) {
    auto* p = dynamic_cast<TParameter<double>*>(t->GetUserInfo()->FindObject("rfPeriodNs"));
    std::printf("%-6s rfPeriodNs: %s", t->GetName(), p ? "" : "absent\n");
    if (p) {
      std::printf("%.4f ns\n", p->GetVal());
      T = p->GetVal();
    }
  }

  const int kMaxHits = 4096;
  Double_t trig = 0, beam = 0, hit_t[kMaxHits];
  UInt_t nhits = 0;
  UChar_t hit_mod[kMaxHits], hit_ch[kMaxHits];
  ev->SetBranchAddress("TriggerTime", &trig);
  ev->SetBranchAddress("BeamTime", &beam);
  ev->SetBranchAddress("NHits", &nhits);
  ev->SetBranchAddress("HitTime", hit_t);
  ev->SetBranchAddress("HitMod", hit_mod);
  ev->SetBranchAddress("HitCh", hit_ch);

  TH1D tof("tof", "LS TOF;t(LS) - bunch [ns]", 400, -100, 300);
  Long64_t bad = 0, no_beam = 0, with_beam_hit_in_window = 0;
  for (Long64_t i = 0; i < ev->GetEntries(); ++i) {
    ev->GetEntry(i);
    double want = -1.0;
    auto it = std::lower_bound(beams.begin(), beams.end(), trig);
    if (it != beams.end()) want = *it;
    if (it != beams.begin() && (want < 0 || trig - *(it - 1) < want - trig)) want = *(it - 1);
    if (want >= 0 && std::fabs(trig - want) > 1e6) want = -1.0;
    if (beam != want) {
      if (++bad <= 5)
        std::printf("  event %lld: t=%.3f BeamTime=%.3f, nearest beam hit %.3f\n", i, trig, beam, want);
    }
    if (beam < 0) {
      ++no_beam;
      continue;
    }
    bool in_window = false;
    for (UInt_t k = 0; k < nhits; ++k)
      if (hit_mod[k] == beam_mod && hit_ch[k] == beam_ch) in_window = true;
    if (in_window) ++with_beam_hit_in_window;
    if (T <= 0) continue;
    const double tb = beam + std::round((trig - si_delay - beam) / T) * T;
    for (UInt_t k = 0; k < nhits; ++k)
      if (hit_mod[k] == ls_mod && hit_ch[k] >= ls_ch_min && hit_ch[k] <= ls_ch_max)
        tof.Fill(trig + hit_t[k] - tb);
  }

  const Long64_t n = ev->GetEntries();
  std::printf("beam hits %zu, events %lld, without a beam hit %lld, with a beam hit in their "
              "window %lld (%.1f %%)\n",
              beams.size(), n, no_beam, with_beam_hit_in_window,
              n ? 100.0 * with_beam_hit_in_window / n : 0.0);
  std::printf("LS hits with a bunch %.0f, TOF peak bin at %.1f ns, entries under -100 or over 300 "
              "ns: %.0f\n",
              tof.GetEntries(), tof.GetXaxis()->GetBinCenter(tof.GetMaximumBin()),
              tof.GetBinContent(0) + tof.GetBinContent(tof.GetNbinsX() + 1));
  std::printf(bad ? "FAIL: %lld events with a wrong BeamTime\n" : "OK: every BeamTime is the nearest beam hit\n",
              bad);
  return bad ? 1 : 0;
}

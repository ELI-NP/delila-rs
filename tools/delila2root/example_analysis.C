// example_analysis.C — read a `.delila` file directly in a ROOT macro.
//
// This is the "no conversion" workflow: instead of running delila2root, you
// #include the single header and loop over events, filling whatever histograms
// you want. No intermediate ROOT file, no giant auto-tree.
//
//   root -l 'example_analysis.C("run0003_0000_X743_ThGEM_Test.delila")'
//
// License: BSD-3-Clause.

#include <cstdio>
#include <string>

#include "TDelila.hpp"
#include "TCanvas.h"
#include "TGraph.h"
#include "TH1D.h"

void example_analysis(const char* path) {
  tdelila::TDelila d(path);
  if (!d.good()) {
    std::printf("open failed: %s\n", d.error().c_str());
    return;
  }
  std::printf("run %u '%s' (format v%u), footer says %llu events\n",
              d.header().run_number, d.header().exp_name.c_str(),
              d.header().version, (unsigned long long)d.footer().total_events);

  auto* hEnergy = new TH1D("hEnergy", "Energy;ADC;counts", 2048, 0, 16384);
  auto* hChan = new TH1D("hChan", "Channel occupancy;channel;counts", 64, 0, 64);

  TGraph* gWave = nullptr;  // first waveform found

  tdelila::Event ev;
  long n = 0;
  while (d.next(ev)) {
    hEnergy->Fill(ev.energy());
    hChan->Fill(ev.channel());

    if (!gWave && ev.has_waveform()) {
      const auto& wf = ev.waveform();
      std::vector<short> a = wf.analog_probe(1);
      if (!a.empty()) {
        gWave = new TGraph((Int_t)a.size());
        double ns = wf.ns_per_sample();
        for (size_t k = 0; k < a.size(); ++k)
          gWave->SetPoint((Int_t)k, ns > 0 ? k * ns : (double)k, a[k]);
        gWave->SetTitle(Form("First waveform (ch %d);%s;ADC", ev.channel(),
                             ns > 0 ? "Time (ns)" : "Sample"));
      }
    }
    n++;
  }
  std::printf("processed %ld events\n", n);

  auto* c = new TCanvas("c", "delila", 1200, 400);
  c->Divide(gWave ? 3 : 2, 1);
  c->cd(1); hEnergy->Draw();
  c->cd(2); hChan->Draw();
  if (gWave) { c->cd(3); gWave->Draw("AL"); }
}

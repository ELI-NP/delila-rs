// analyse_si_e_de.C
//
// Quick-look analysis of the delila-rs EB output on the ELIFANT2025 p91Zr
// data set. Produces:
//
//   - Multiplicity distribution
//   - Per-event count of hits broken down by detector role
//   - 2D Si E vs dE plot, where:
//       * "E" hit  = first hit on (mod == 0) — E_Sector layer
//       * "dE" hit = first hit on (mod == 4) — dE_Sector layer
//     (corresponds to the L2 `Si_Both` Accept op that gates the EB output)
//   - 1D ADC spectra for E_Sector / dE_Sector for sanity
//
// Run:
//   root -l -b -q 'analyse_si_e_de.C("eb_output")'
//
// The macro chains every `eb_run*_events.root` it finds in the directory.

#include <TCanvas.h>
#include <TChain.h>
#include <TFile.h>
#include <TH1.h>
#include <TH2.h>
#include <TStyle.h>
#include <TSystemDirectory.h>
#include <TSystemFile.h>

#include <iostream>
#include <string>
#include <vector>

void analyse_si_e_de(const char* output_dir = "eb_output",
                     const char* out_root = "si_e_de.root") {
  gStyle->SetOptStat(111111);

  // ---- Collect input files --------------------------------------------------
  TChain ch("EventTree");
  TSystemDirectory dir(output_dir, output_dir);
  TList* files = dir.GetListOfFiles();
  int n_added = 0;
  if (files) {
    TIter next(files);
    while (TSystemFile* f = (TSystemFile*)next()) {
      TString name = f->GetName();
      if (f->IsDirectory()) continue;
      if (!name.BeginsWith("eb_run")) continue;
      if (!name.EndsWith("_events.root")) continue;
      TString full = TString(output_dir) + "/" + name;
      ch.Add(full);
      n_added++;
    }
  }
  std::cout << "Chained " << n_added << " files from " << output_dir
            << " (entries = " << ch.GetEntries() << ")\n";
  if (ch.GetEntries() == 0) {
    std::cerr << "No events in chain — nothing to analyse.\n";
    return;
  }

  // ---- Branches -------------------------------------------------------------
  ULong64_t event_id = 0;
  Double_t trigger_time = 0.0;
  UChar_t trigger_mod = 0, trigger_ch = 0;
  UInt_t multiplicity = 0;
  std::vector<unsigned char>* mods = nullptr;
  std::vector<unsigned char>* chs = nullptr;
  std::vector<unsigned short>* energy = nullptr;
  std::vector<unsigned short>* energy_short = nullptr;
  std::vector<double>* rel_time = nullptr;
  std::vector<unsigned char>* with_ac = nullptr;

  ch.SetBranchAddress("EventID", &event_id);
  ch.SetBranchAddress("TriggerTime", &trigger_time);
  ch.SetBranchAddress("TriggerMod", &trigger_mod);
  ch.SetBranchAddress("TriggerCh", &trigger_ch);
  ch.SetBranchAddress("Multiplicity", &multiplicity);
  ch.SetBranchAddress("Mod", &mods);
  ch.SetBranchAddress("Ch", &chs);
  ch.SetBranchAddress("Energy", &energy);
  ch.SetBranchAddress("EnergyShort", &energy_short);
  ch.SetBranchAddress("RelTime", &rel_time);
  ch.SetBranchAddress("WithAC", &with_ac);

  // ---- Histograms -----------------------------------------------------------
  TH1F* h_mult = new TH1F("h_mult", "Event multiplicity;hits / event;count",
                         32, -0.5, 31.5);
  TH1F* h_e_adc = new TH1F("h_e_adc", "E_Sector raw ADC (mod=0);ADC;count",
                          4096, 0, 65536);
  TH1F* h_de_adc = new TH1F("h_de_adc", "dE_Sector raw ADC (mod=4);ADC;count",
                           4096, 0, 65536);
  TH1F* h_trigger_mod =
      new TH1F("h_trigger_mod", "Trigger module distribution;module;count",
               12, -0.5, 11.5);
  TH1F* h_rel_time =
      new TH1F("h_rel_time",
               "Hit relative time to trigger;rel_time [ns];count",
               401, -200.5, 200.5);

  // 2D: E (mod 0) vs dE (mod 4). Use raw ADC values (calibration could be
  // applied in a second pass once we trust the ADC scale).
  TH2F* h_e_de_2d = new TH2F("h_e_de_2d",
                             "Si E vs dE (raw ADC; first hit per layer);"
                             "dE (mod 4) ADC;E (mod 0) ADC",
                             256, 0, 65536, 256, 0, 65536);

  // ---- Loop -----------------------------------------------------------------
  const Long64_t n_entries = ch.GetEntries();
  Long64_t coincident_pairs = 0;
  Long64_t orphans_e_only = 0;
  Long64_t orphans_de_only = 0;
  Long64_t orphans_none = 0;
  for (Long64_t i = 0; i < n_entries; ++i) {
    ch.GetEntry(i);
    h_mult->Fill(multiplicity);
    h_trigger_mod->Fill(trigger_mod);

    int e_adc = -1, de_adc = -1;
    for (size_t j = 0; j < mods->size(); ++j) {
      auto m = (*mods)[j];
      auto c = (*chs)[j];
      auto e = (*energy)[j];
      auto rt = (*rel_time)[j];
      h_rel_time->Fill(rt);
      (void)c;
      if (m == 0) {
        h_e_adc->Fill(e);
        if (e_adc < 0) e_adc = e;
      } else if (m == 4) {
        h_de_adc->Fill(e);
        if (de_adc < 0) de_adc = e;
      }
    }
    if (e_adc >= 0 && de_adc >= 0) {
      h_e_de_2d->Fill(de_adc, e_adc);
      coincident_pairs++;
    } else if (e_adc >= 0) {
      orphans_e_only++;
    } else if (de_adc >= 0) {
      orphans_de_only++;
    } else {
      orphans_none++;
    }
  }

  std::cout << "\n=== Summary ===\n";
  std::cout << "Total events:           " << n_entries << "\n";
  std::cout << "E + dE coincident:      " << coincident_pairs
            << " (" << 100.0 * coincident_pairs / n_entries << "%)\n";
  std::cout << "Only E_Sector (mod 0):  " << orphans_e_only << "\n";
  std::cout << "Only dE_Sector (mod 4): " << orphans_de_only << "\n";
  std::cout << "Neither:                " << orphans_none << "\n";
  std::cout << "Trigger-mod distribution:\n";
  for (int b = 1; b <= h_trigger_mod->GetNbinsX(); ++b) {
    if (h_trigger_mod->GetBinContent(b) > 0) {
      std::cout << "  mod " << (b - 1) << ": "
                << h_trigger_mod->GetBinContent(b) << "\n";
    }
  }

  // ---- Save -----------------------------------------------------------------
  TFile fout(out_root, "RECREATE");
  h_mult->Write();
  h_e_adc->Write();
  h_de_adc->Write();
  h_e_de_2d->Write();
  h_trigger_mod->Write();
  h_rel_time->Write();
  fout.Close();
  std::cout << "Wrote " << out_root << "\n";

  // Save a PNG of the 2D plot for quick visual check.
  TCanvas c("c_e_de", "Si E vs dE", 800, 700);
  c.SetLogz();
  h_e_de_2d->Draw("COLZ");
  TString png = TString(out_root);
  if (png.EndsWith(".root")) png.ReplaceAll(".root", ".png");
  else png += ".png";
  c.SaveAs(png);
  std::cout << "Wrote " << png << "\n";
}

// analyse_si_e_de.C
//
// Quick-look analysis of the delila-rs EB output on the ELIFANT2025 p91Zr
// data set. Now applies chSettings.json calibration (p0..p3) and zooms
// into the physics region.
//
// Outputs:
//   - h_e_de_2d_raw:        Si E vs dE in raw ADC, zoomed to the physics
//                           region (0..10k × 0..6k) where the PID banana
//                           lives
//   - h_e_de_2d_raw_full:   Same plot but on the full 0..65k × 0..65k axes
//                           (saturation stripes visible)
//   - h_e_de_2d_kev:        Si E vs dE in calibrated keV
//                           (axes auto-sized via chSettings polynomial)
//   - h_mult, h_e_adc, h_de_adc, h_trigger_mod, h_rel_time
//
// Run:
//   root -l -b -q 'analyse_si_e_de.C("eb_output", "chSettings.json",
//                                    "si_e_de.root")'
//
// Chains every `eb_run*_events.root` under `output_dir`.

#include <TCanvas.h>
#include <TChain.h>
#include <TFile.h>
#include <TH1.h>
#include <TH2.h>
#include <TStyle.h>
#include <TSystemDirectory.h>
#include <TSystemFile.h>
#include <nlohmann/json.hpp>

#include <array>
#include <fstream>
#include <iostream>
#include <map>
#include <string>
#include <vector>

namespace {

struct Calib {
  double p0 = 0.0, p1 = 1.0, p2 = 0.0, p3 = 0.0;

  double apply(double adc) const {
    return p0 + p1 * adc + p2 * adc * adc + p3 * adc * adc * adc;
  }
};

using CalibMap = std::map<std::pair<int, int>, Calib>;

CalibMap load_calib(const std::string& path) {
  using nlohmann::json;
  CalibMap m;
  std::ifstream f(path);
  if (!f) {
    std::cerr << "warning: failed to open " << path
              << " — calibration disabled (using raw ADC)\n";
    return m;
  }
  json j;
  f >> j;
  // chSettings.json is `[[ch, ch, ...], [ch, ch, ...], ...]`
  for (auto& mod : j) {
    for (auto& ch : mod) {
      Calib c;
      c.p0 = ch.value("p0", 0.0);
      c.p1 = ch.value("p1", 1.0);
      c.p2 = ch.value("p2", 0.0);
      c.p3 = ch.value("p3", 0.0);
      int m_id = ch.value("Module", -1);
      int c_id = ch.value("Channel", -1);
      if (m_id >= 0 && c_id >= 0) {
        m[{m_id, c_id}] = c;
      }
    }
  }
  std::cout << "Loaded calibration for " << m.size() << " channels\n";
  return m;
}

double cal_or_raw(const CalibMap& cal, int mod, int ch, double adc) {
  auto it = cal.find({mod, ch});
  if (it == cal.end()) return adc;
  return it->second.apply(adc);
}

}  // namespace

void analyse_si_e_de(const char* output_dir = "eb_output",
                     const char* ch_settings = "chSettings.json",
                     const char* out_root = "si_e_de.root") {
  gStyle->SetOptStat(111111);

  CalibMap calib = load_calib(ch_settings);

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

  // Raw ADC: full range and zoomed to the physics region.
  TH2F* h_e_de_2d_raw_full =
      new TH2F("h_e_de_2d_raw_full",
               "Si E vs dE (raw ADC, full range);"
               "dE (mod 4) ADC;E (mod 0) ADC",
               256, 0, 65536, 256, 0, 65536);
  TH2F* h_e_de_2d_raw =
      new TH2F("h_e_de_2d_raw",
               "Si E vs dE (raw ADC, physics zoom);"
               "dE (mod 4) ADC;E (mod 0) ADC",
               400, 0, 10000, 300, 0, 6000);

  // Calibrated: axes chosen so the proton/alpha banana fits comfortably.
  // The exact keV range depends on the per-channel polynomials; if needed
  // the limits below are easy to widen.
  TH2F* h_e_de_2d_kev =
      new TH2F("h_e_de_2d_kev",
               "Si E vs dE (calibrated);"
               "dE [keV];E [keV]",
               400, 0, 12000, 300, 0, 8000);

  // ---- Loop -----------------------------------------------------------------
  const Long64_t n_entries = ch.GetEntries();
  Long64_t coincident_pairs = 0;
  for (Long64_t i = 0; i < n_entries; ++i) {
    ch.GetEntry(i);
    h_mult->Fill(multiplicity);
    h_trigger_mod->Fill(trigger_mod);

    int e_adc = -1, de_adc = -1;
    int e_mod = -1, e_ch = -1, de_mod = -1, de_ch = -1;
    for (size_t j = 0; j < mods->size(); ++j) {
      auto m = (*mods)[j];
      auto c = (*chs)[j];
      auto e = (*energy)[j];
      auto rt = (*rel_time)[j];
      h_rel_time->Fill(rt);
      if (m == 0) {
        h_e_adc->Fill(e);
        if (e_adc < 0) {
          e_adc = e;
          e_mod = m;
          e_ch = c;
        }
      } else if (m == 4) {
        h_de_adc->Fill(e);
        if (de_adc < 0) {
          de_adc = e;
          de_mod = m;
          de_ch = c;
        }
      }
    }
    if (e_adc >= 0 && de_adc >= 0) {
      h_e_de_2d_raw->Fill(de_adc, e_adc);
      h_e_de_2d_raw_full->Fill(de_adc, e_adc);
      double e_kev = cal_or_raw(calib, e_mod, e_ch, e_adc);
      double de_kev = cal_or_raw(calib, de_mod, de_ch, de_adc);
      h_e_de_2d_kev->Fill(de_kev, e_kev);
      coincident_pairs++;
    }
  }

  std::cout << "\n=== Summary ===\n";
  std::cout << "Total events:     " << n_entries << "\n";
  std::cout << "E + dE pairs:     " << coincident_pairs << " ("
            << 100.0 * coincident_pairs / n_entries << "%)\n";
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
  h_e_de_2d_raw->Write();
  h_e_de_2d_raw_full->Write();
  h_e_de_2d_kev->Write();
  h_trigger_mod->Write();
  h_rel_time->Write();
  fout.Close();
  std::cout << "Wrote " << out_root << "\n";

  // ---- PNGs -----------------------------------------------------------------
  auto save_png = [&](TH2* h, const char* tag) {
    TString name = Form("si_e_de_%s.png", tag);
    TCanvas c(Form("c_%s", tag), h->GetTitle(), 900, 700);
    c.SetLogz();
    h->Draw("COLZ");
    c.SaveAs(name);
    std::cout << "Wrote " << name << "\n";
  };
  save_png(h_e_de_2d_raw, "raw");
  save_png(h_e_de_2d_raw_full, "raw_full");
  save_png(h_e_de_2d_kev, "kev");
}

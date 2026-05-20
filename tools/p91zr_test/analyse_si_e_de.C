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
  TH1F* h_e_adc = new TH1F("h_e_adc", "E raw ADC (mod 4, det-B front);ADC;count",
                          4096, 0, 65536);
  TH1F* h_de_adc = new TH1F("h_de_adc", "dE raw ADC (mod 0, det-A front);ADC;count",
                           4096, 0, 65536);
  TH1F* h_trigger_mod =
      new TH1F("h_trigger_mod", "Trigger module distribution;module;count",
               12, -0.5, 11.5);
  TH1F* h_rel_time =
      new TH1F("h_rel_time",
               "Hit relative time to trigger;rel_time [ns];count",
               401, -200.5, 200.5);

  // Raw ADC: full range and zoomed to the physics region.
  // Convention: X = E (total, mod 4), Y = dE (energy loss, mod 0).
  TH2F* h_e_de_2d_raw_full =
      new TH2F("h_e_de_2d_raw_full",
               "Si dE vs E (raw ADC, full range, **naive any+any pairing**);"
               "E (mod 4) ADC;dE (mod 0) ADC",
               256, 0, 65536, 256, 0, 65536);
  TH2F* h_e_de_2d_raw =
      new TH2F("h_e_de_2d_raw",
               "Si dE vs E (raw ADC, zoom, **naive any+any pairing**);"
               "E (mod 4) ADC;dE (mod 0) ADC",
               400, 0, 12000, 300, 0, 12000);

  TH2F* h_e_de_2d_kev =
      new TH2F("h_e_de_2d_kev",
               "Si dE vs E (calibrated, **naive any+any pairing**);"
               "E [keV] (mod 4);dE [keV] (mod 0)",
               400, 0, 12000, 300, 0, 12000);

  // Kinematic-pair (anti-diagonal) mapping between the two telescope
  // fronts. The user confirmed mod0_ch X ↔ mod4_ch (15 - X) by inspection
  // of the histSectorSector image (a clean anti-diagonal band — the
  // expected 2-body back-to-back kinematics for this layout). The
  // ring_ring.cpp variable naming makes mod 0 the *dE* (det-A front)
  // and mod 4 the *E* (det-B front); the chSettings.json tags use
  // the opposite labels and should be ignored for physics.
  auto partner_of_mod0 = [](int ch) -> int { return 15 - ch; };
  auto partner_of_mod4 = [](int ch) -> int { return 15 - ch; };

  TH2F* h_e_de_paired_kev =
      new TH2F("h_e_de_paired_kev",
               "Si dE vs E (calibrated, **anti-diagonal pairing**);"
               "E [keV] (mod 4);dE [keV] (mod 0)",
               400, 0, 12000, 300, 0, 12000);
  TH2F* h_e_de_paired_raw =
      new TH2F("h_e_de_paired_raw",
               "Si dE vs E (raw ADC, **anti-diagonal pairing**);"
               "E (mod 4) ADC;dE (mod 0) ADC",
               400, 0, 12000, 300, 0, 12000);

  // Additional view: anti-diagonal pairing **and** require that each
  // telescope fires exactly one front-sector channel in this event.
  // This mimics ring_ring.cpp's `dECounter == 1 || eCounter == 1` rule.
  // Lives in the analysis macro (NOT in the EB) — see SPEC § 1.4: this
  // is a multiplicity-conditional pairing cut, which is on the
  // forbidden list for the EB.
  TH2F* h_e_de_paired_mult1_kev =
      new TH2F("h_e_de_paired_mult1_kev",
               "Si dE vs E (anti-diag, **mult==1 per telescope**);"
               "E [keV] (mod 4);dE [keV] (mod 0)",
               400, 0, 12000, 300, 0, 12000);
  TH2F* h_e_de_paired_mult1_raw =
      new TH2F("h_e_de_paired_mult1_raw",
               "Si dE vs E (raw ADC, anti-diag, **mult==1 per telescope**);"
               "E (mod 4) ADC;dE (mod 0) ADC",
               400, 0, 12000, 300, 0, 12000);

  // Per-channel diagnostics: dE_ch vs dE_ADC (mod 4) and E_ch vs E_ADC
  // (mod 0). Helps identify whether the vertical stripe at dE ≈ 8.1 MeV
  // comes from one specific channel firing at fixed amplitude.
  TH2F* h_de_ch_vs_adc =
      new TH2F("h_de_ch_vs_adc",
               "Per-channel dE spectrum (mod 4);ch;ADC",
               16, -0.5, 15.5, 400, 0, 10000);
  TH2F* h_e_ch_vs_adc =
      new TH2F("h_e_ch_vs_adc",
               "Per-channel E spectrum (mod 0);ch;ADC",
               16, -0.5, 15.5, 400, 0, 10000);

  // ---- Loop -----------------------------------------------------------------
  const Long64_t n_entries = ch.GetEntries();
  Long64_t coincident_pairs = 0;
  Long64_t paired_sectors = 0;
  Long64_t paired_sectors_mult1 = 0;
  for (Long64_t i = 0; i < n_entries; ++i) {
    ch.GetEntry(i);
    h_mult->Fill(multiplicity);
    h_trigger_mod->Fill(trigger_mod);

    // Per-channel first-hit arrays for the anti-diagonal pairing:
    // index by raw ch (0..15), record ADC + ch.
    std::array<int, 16> mod0_adc{};  // dE side (front, thin)
    std::array<int, 16> mod4_adc{};  // E side (front, thick)
    mod0_adc.fill(-1);
    mod4_adc.fill(-1);

    int e_adc = -1, de_adc = -1;
    int e_mod = -1, e_ch = -1, de_mod = -1, de_ch = -1;
    for (size_t j = 0; j < mods->size(); ++j) {
      auto m = (*mods)[j];
      auto c = (*chs)[j];
      auto e = (*energy)[j];
      auto rt = (*rel_time)[j];
      h_rel_time->Fill(rt);
      if (m == 0) {
        // mod 0 → dE detector (per ring_ring.cpp variable naming)
        h_de_adc->Fill(e);  // <-- mod 0 is dE
        h_de_ch_vs_adc->Fill(c, e);
        if (de_adc < 0) {
          de_adc = e;
          de_mod = m;
          de_ch = c;
        }
        if (c < 16 && mod0_adc[c] < 0) {
          mod0_adc[c] = e;
        }
      } else if (m == 4) {
        // mod 4 → E detector (per ring_ring.cpp variable naming)
        h_e_adc->Fill(e);  // <-- mod 4 is E
        h_e_ch_vs_adc->Fill(c, e);
        if (e_adc < 0) {
          e_adc = e;
          e_mod = m;
          e_ch = c;
        }
        if (c < 16 && mod4_adc[c] < 0) {
          mod4_adc[c] = e;
        }
      }
    }
    if (e_adc >= 0 && de_adc >= 0) {
      // Convention: X = E (mod 4), Y = dE (mod 0)
      h_e_de_2d_raw->Fill(e_adc, de_adc);
      h_e_de_2d_raw_full->Fill(e_adc, de_adc);
      double e_kev = cal_or_raw(calib, e_mod, e_ch, e_adc);
      double de_kev = cal_or_raw(calib, de_mod, de_ch, de_adc);
      h_e_de_2d_kev->Fill(e_kev, de_kev);
      coincident_pairs++;
    }

    // Anti-diagonal pairing: for each ch X in mod 0, pair with ch (15-X)
    // in mod 4. Both must have fired.
    int mod0_count = 0;
    int mod4_count = 0;
    for (int x = 0; x < 16; ++x) {
      if (mod0_adc[x] >= 0) mod0_count++;
      if (mod4_adc[x] >= 0) mod4_count++;
    }
    const bool mult1 = (mod0_count == 1 && mod4_count == 1);

    for (int x = 0; x < 16; ++x) {
      int partner = partner_of_mod0(x);  // = 15 - x
      if (mod0_adc[x] >= 0 && mod4_adc[partner] >= 0) {
        int de = mod0_adc[x];
        int e = mod4_adc[partner];
        // Convention: X = E, Y = dE
        h_e_de_paired_raw->Fill(e, de);
        double de_kev = cal_or_raw(calib, 0, x, de);
        double e_kev = cal_or_raw(calib, 4, partner, e);
        h_e_de_paired_kev->Fill(e_kev, de_kev);
        paired_sectors++;
        if (mult1) {
          h_e_de_paired_mult1_raw->Fill(e, de);
          h_e_de_paired_mult1_kev->Fill(e_kev, de_kev);
          paired_sectors_mult1++;
        }
      }
    }
    // Silence "unused" warning when partner_of_mod4 is not used directly.
    (void)partner_of_mod4;
  }

  std::cout << "\n=== Summary ===\n";
  std::cout << "Total events:           " << n_entries << "\n";
  std::cout << "E + dE pairs (naive):   " << coincident_pairs << " ("
            << 100.0 * coincident_pairs / n_entries << "%)\n";
  std::cout << "Anti-diag pairs (all):  " << paired_sectors << " ("
            << 100.0 * paired_sectors / n_entries << "%)\n";
  std::cout << "Anti-diag, mult==1:     " << paired_sectors_mult1 << " ("
            << 100.0 * paired_sectors_mult1 / n_entries << "%)\n";
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
  h_e_de_paired_raw->Write();
  h_e_de_paired_kev->Write();
  h_e_de_paired_mult1_raw->Write();
  h_e_de_paired_mult1_kev->Write();
  h_de_ch_vs_adc->Write();
  h_e_ch_vs_adc->Write();
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
  save_png(h_e_de_paired_raw, "paired_raw");
  save_png(h_e_de_paired_kev, "paired_kev");
  save_png(h_e_de_paired_mult1_raw, "paired_mult1_raw");
  save_png(h_e_de_paired_mult1_kev, "paired_mult1_kev");
  save_png(h_de_ch_vs_adc, "de_per_channel");
  save_png(h_e_ch_vs_adc, "e_per_channel");
}

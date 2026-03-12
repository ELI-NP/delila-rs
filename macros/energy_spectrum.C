// Energy spectrum for all channels: Mod 0-2, Ch 0-15
// Usage: root -l -b -q 'macros/energy_spectrum.C("data/test.root")'
#include <TCanvas.h>
#include <TFile.h>
#include <TH1.h>
#include <TString.h>
#include <TTree.h>

#include <iostream>

const int nMod = 3;
const int nCh = 16;
TH1D *hEne[nMod][nCh];

void energy_spectrum(const char *fileName = "run0002_sum_Fission2026.root")
{
  TFile *f = TFile::Open(fileName);
  TTree *t = (TTree *)f->Get("delila");
  std::cout << "Entries: " << t->GetEntries() << std::endl;

  UChar_t mod, ch;
  UShort_t energy;
  t->SetBranchAddress("Mod", &mod);
  t->SetBranchAddress("Ch", &ch);
  t->SetBranchAddress("Energy", &energy);

  for (int m = 0; m < nMod; m++) {
    for (int c = 0; c < nCh; c++) {
      hEne[m][c] =
          new TH1D(Form("hEne_m%d_c%d", m, c),
                   Form("Mod%d Ch%d;Energy;Counts", m, c), 32000, 0, 32000);
    }
  }

  Long64_t nEntries = t->GetEntries();
  for (Long64_t i = 0; i < nEntries; i++) {
    t->GetEntry(i);
    if (mod < nMod && ch < nCh) {
      hEne[mod][ch]->Fill(energy);
    }
  }
}

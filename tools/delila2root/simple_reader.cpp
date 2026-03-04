// Simple ROOT file reader for delila2root output (scalar branches)
//
// Usage (ROOT interpreter):
//   root -l 'simple_reader.cpp("output.root")'

#include <TFile.h>
#include <TTree.h>

#include <iostream>
#include <string>

void simple_reader(std::string fileName = "output.root")
{
  auto file = TFile::Open(fileName.c_str(), "READ");
  auto tree = file->Get<TTree>("delila");
  const auto nEntries = tree->GetEntries();
  std::cout << "Number of entries: " << nEntries << std::endl;

  UChar_t Mod, Ch;
  UShort_t Energy, EnergyShort;
  Double_t Timestamp;
  ULong64_t Flags;

  tree->SetBranchAddress("Mod", &Mod);
  tree->SetBranchAddress("Ch", &Ch);
  tree->SetBranchAddress("Energy", &Energy);
  tree->SetBranchAddress("EnergyShort", &EnergyShort);
  tree->SetBranchAddress("Timestamp", &Timestamp);
  tree->SetBranchAddress("Flags", &Flags);

  for (Long64_t i = 0; i < nEntries && i < 100; i++) {
    tree->GetEntry(i);
    std::cout << "  Mod=" << Int_t(Mod)
              << ", Ch=" << Int_t(Ch)
              << ", Energy=" << Energy
              << ", Timestamp=" << Timestamp << std::endl;
  }

  file->Close();
}

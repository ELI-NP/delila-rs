// Check mod7 hit inclusion rate when mod6 is the trigger
// Usage: root -l -b -q 'macros/mod6_mod7_correlation.C("tmp/eb_run0209_*_events.root")'
#include <TChain.h>
#include <TH1D.h>
#include <TCanvas.h>
#include <iostream>
#include <vector>

void mod6_mod7_correlation(const char *filePattern = "tmp/eb_run0209_*_events.root")
{
    TChain chain("EventTree");
    int nFiles = chain.Add(filePattern);
    Long64_t nEntries = chain.GetEntries();
    std::cout << "Loaded " << nFiles << " files, " << nEntries << " entries" << std::endl;

    // Branch types match Rust output:
    // EventID: u64, TriggerMod/TriggerCh: u8, Multiplicity: u32
    // Mod/Ch: Vec<u8>, Energy: Vec<u16>, RelTime: Vec<f64>
    ULong64_t eventID;
    UChar_t triggerMod, triggerCh;
    UInt_t multiplicity;
    std::vector<unsigned char> *modVec = nullptr;
    std::vector<unsigned char> *chVec = nullptr;
    std::vector<unsigned short> *energyVec = nullptr;
    std::vector<double> *relTimeVec = nullptr;

    chain.SetBranchAddress("EventID", &eventID);
    chain.SetBranchAddress("TriggerMod", &triggerMod);
    chain.SetBranchAddress("TriggerCh", &triggerCh);
    chain.SetBranchAddress("Multiplicity", &multiplicity);
    chain.SetBranchAddress("Mod", &modVec);
    chain.SetBranchAddress("Ch", &chVec);
    chain.SetBranchAddress("Energy", &energyVec);
    chain.SetBranchAddress("RelTime", &relTimeVec);

    // Counters
    long long nMod6Trigger = 0;
    long long nMod6WithMod7 = 0;
    long long nMod7Trigger = 0;
    long long nMod7WithMod6 = 0;
    long long nMod8Trigger = 0;
    long long nMod8WithMod9 = 0;
    long long nMod9Trigger = 0;
    long long nMod9WithMod8 = 0;

    TH1D *hMult6 = new TH1D("hMult6", "Multiplicity (mod6 trigger);Multiplicity;Events", 50, 0, 50);
    TH1D *hMult8 = new TH1D("hMult8", "Multiplicity (mod8 trigger);Multiplicity;Events", 50, 0, 50);
    TH1D *hModPresence6 = new TH1D("hModPresence6", "Modules present (mod6 trigger);Module;Fraction", 12, 0, 12);
    TH1D *hModPresence8 = new TH1D("hModPresence8", "Modules present (mod8 trigger);Module;Fraction", 12, 0, 12);

    for (Long64_t i = 0; i < nEntries; i++) {
        chain.GetEntry(i);
        if (i % 10000000 == 0 && i > 0)
            std::cout << "  " << i / 1000000 << "M / " << nEntries / 1000000 << "M" << std::endl;

        // Check which modules are present in this event
        bool hasMod[12] = {};
        for (size_t j = 0; j < modVec->size(); j++) {
            int m = (*modVec)[j];
            if (m < 12) hasMod[m] = true;
        }

        if (triggerMod == 6) {
            nMod6Trigger++;
            if (hasMod[7]) nMod6WithMod7++;
            hMult6->Fill(multiplicity);
            for (int m = 0; m < 12; m++) {
                if (hasMod[m]) hModPresence6->Fill(m);
            }
        }
        if (triggerMod == 7) {
            nMod7Trigger++;
            if (hasMod[6]) nMod7WithMod6++;
        }
        if (triggerMod == 8) {
            nMod8Trigger++;
            if (hasMod[9]) nMod8WithMod9++;
            hMult8->Fill(multiplicity);
            for (int m = 0; m < 12; m++) {
                if (hasMod[m]) hModPresence8->Fill(m);
            }
        }
        if (triggerMod == 9) {
            nMod9Trigger++;
            if (hasMod[8]) nMod9WithMod8++;
        }
    }

    std::cout << "\n=== Correlation Results ===" << std::endl;
    std::cout << "Mod6 trigger events: " << nMod6Trigger << std::endl;
    if (nMod6Trigger > 0) {
        std::cout << "  with Mod7 hit: " << nMod6WithMod7
                  << " (" << 100.0 * nMod6WithMod7 / nMod6Trigger << "%)" << std::endl;
    }
    std::cout << "Mod7 trigger events: " << nMod7Trigger << std::endl;
    if (nMod7Trigger > 0) {
        std::cout << "  with Mod6 hit: " << nMod7WithMod6
                  << " (" << 100.0 * nMod7WithMod6 / nMod7Trigger << "%)" << std::endl;
    }
    std::cout << "Mod8 trigger events: " << nMod8Trigger << std::endl;
    if (nMod8Trigger > 0) {
        std::cout << "  with Mod9 hit: " << nMod8WithMod9
                  << " (" << 100.0 * nMod8WithMod9 / nMod8Trigger << "%)" << std::endl;
    }
    std::cout << "Mod9 trigger events: " << nMod9Trigger << std::endl;
    if (nMod9Trigger > 0) {
        std::cout << "  with Mod8 hit: " << nMod9WithMod8
                  << " (" << 100.0 * nMod9WithMod8 / nMod9Trigger << "%)" << std::endl;
    }

    // Normalize module presence histograms to fraction
    if (nMod6Trigger > 0) hModPresence6->Scale(1.0 / nMod6Trigger);
    if (nMod8Trigger > 0) hModPresence8->Scale(1.0 / nMod8Trigger);

    TCanvas *c = new TCanvas("c", "Trigger Correlations", 1200, 800);
    c->Divide(2, 2);
    c->cd(1); hMult6->Draw();
    c->cd(2); hModPresence6->Draw(); hModPresence6->SetMinimum(0);
    c->cd(3); hMult8->Draw();
    c->cd(4); hModPresence8->Draw(); hModPresence8->SetMinimum(0);
    c->SaveAs("tmp/trigger_correlation.pdf");
    std::cout << "\nSaved: tmp/trigger_correlation.pdf" << std::endl;
}

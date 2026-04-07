// Verify waveform branches in delila2root output
// Compile: c++ -O2 $(root-config --cflags --libs) -o verify_waveform verify_waveform.cpp

#include <TFile.h>
#include <TTree.h>
#include <TBranch.h>

#include <cstdint>
#include <iostream>
#include <string>
#include <vector>

int main(int argc, char* argv[]) {
    if (argc < 2) {
        std::cerr << "Usage: " << argv[0] << " <file.root>" << std::endl;
        return 1;
    }

    TFile file(argv[1], "READ");
    if (file.IsZombie()) {
        std::cerr << "Error: Cannot open " << argv[1] << std::endl;
        return 1;
    }

    auto* tree = file.Get<TTree>("delila");
    if (!tree) {
        std::cerr << "Error: TTree 'delila' not found" << std::endl;
        return 1;
    }

    const auto nEntries = tree->GetEntries();
    std::cout << "Entries: " << nEntries << std::endl;

    // List all branches
    std::cout << "\nBranches:" << std::endl;
    auto* branches = tree->GetListOfBranches();
    for (int i = 0; i < branches->GetEntries(); i++) {
        auto* b = static_cast<TBranch*>(branches->At(i));
        std::string cls = b->GetClassName();
        std::cout << "  " << b->GetName();
        if (!cls.empty()) std::cout << " (" << cls << ")";
        std::cout << std::endl;
    }

    // Scalar branches
    UChar_t mod, ch;
    UShort_t energy, eshort;
    Double_t timestamp;
    ULong64_t flags;

    tree->SetBranchAddress("Mod", &mod);
    tree->SetBranchAddress("Ch", &ch);
    tree->SetBranchAddress("Energy", &energy);
    tree->SetBranchAddress("EnergyShort", &eshort);
    tree->SetBranchAddress("Timestamp", &timestamp);
    tree->SetBranchAddress("Flags", &flags);

    // Waveform branches
    std::vector<Short_t>* ap1 = nullptr;
    std::vector<Short_t>* ap2 = nullptr;
    std::vector<UChar_t>* dp1 = nullptr;
    std::vector<UChar_t>* dp2 = nullptr;
    std::vector<UChar_t>* dp3 = nullptr;
    std::vector<UChar_t>* dp4 = nullptr;
    UChar_t time_resolution;
    UShort_t trigger_threshold;
    Double_t ns_per_sample;

    tree->SetBranchAddress("AnalogProbe1", &ap1);
    tree->SetBranchAddress("AnalogProbe2", &ap2);
    tree->SetBranchAddress("DigitalProbe1", &dp1);
    tree->SetBranchAddress("DigitalProbe2", &dp2);
    tree->SetBranchAddress("DigitalProbe3", &dp3);
    tree->SetBranchAddress("DigitalProbe4", &dp4);
    tree->SetBranchAddress("TimeResolution", &time_resolution);
    tree->SetBranchAddress("TriggerThreshold", &trigger_threshold);
    tree->SetBranchAddress("NsPerSample", &ns_per_sample);

    // Statistics
    uint64_t with_wf = 0, without_wf = 0;
    size_t max_ap1_size = 0;
    Short_t min_sample = 0, max_sample = 0;

    for (Long64_t i = 0; i < nEntries; i++) {
        tree->GetEntry(i);
        if (ap1 && ap1->size() > 0) {
            with_wf++;
            if (ap1->size() > max_ap1_size) max_ap1_size = ap1->size();
            for (auto s : *ap1) {
                if (s < min_sample) min_sample = s;
                if (s > max_sample) max_sample = s;
            }
        } else {
            without_wf++;
        }
    }

    std::cout << "\nWaveform statistics:" << std::endl;
    std::cout << "  With waveform:    " << with_wf << std::endl;
    std::cout << "  Without waveform: " << without_wf << std::endl;
    std::cout << "  Max AP1 samples:  " << max_ap1_size << std::endl;
    std::cout << "  AP1 sample range: [" << min_sample << ", " << max_sample << "]" << std::endl;

    // Print first 5 events with waveforms
    std::cout << "\nFirst events with waveform data:" << std::endl;
    int printed = 0;
    for (Long64_t i = 0; i < nEntries && printed < 5; i++) {
        tree->GetEntry(i);
        if (ap1 && ap1->size() > 0) {
            std::cout << "  Entry " << i
                      << ": Mod=" << int(mod)
                      << " Ch=" << int(ch)
                      << " E=" << energy
                      << " AP1[" << ap1->size() << "]"
                      << " AP2[" << ap2->size() << "]"
                      << " DP1[" << dp1->size() << "]"
                      << " DP2[" << dp2->size() << "]"
                      << " DP3[" << dp3->size() << "]"
                      << " DP4[" << dp4->size() << "]"
                      << " TR=" << int(time_resolution)
                      << " TT=" << trigger_threshold
                      << " ns/s=" << ns_per_sample;
            // Print first few samples
            std::cout << " samples=[";
            for (size_t j = 0; j < std::min(ap1->size(), size_t(8)); j++) {
                if (j > 0) std::cout << ",";
                std::cout << (*ap1)[j];
            }
            if (ap1->size() > 8) std::cout << ",...";
            std::cout << "]" << std::endl;
            printed++;
        }
    }

    // Print first 5 events without waveforms
    if (without_wf > 0) {
        std::cout << "\nFirst events without waveform:" << std::endl;
        printed = 0;
        for (Long64_t i = 0; i < nEntries && printed < 5; i++) {
            tree->GetEntry(i);
            if (!ap1 || ap1->size() == 0) {
                std::cout << "  Entry " << i
                          << ": Mod=" << int(mod)
                          << " Ch=" << int(ch)
                          << " E=" << energy
                          << " AP1[0]" << std::endl;
                printed++;
            }
        }
    }

    file.Close();
    std::cout << "\nFile size: ";
    // Quick file size check
    FILE* fp = fopen(argv[1], "rb");
    if (fp) {
        fseek(fp, 0, SEEK_END);
        auto sz = ftell(fp);
        fclose(fp);
        std::cout << sz / 1024 / 1024 << " MB" << std::endl;
    }

    return 0;
}

// Time Alignment Visualization - ROOT Macro
// Reads TTree data from timeAlignment.root and creates 2D heatmap + 1D projections
//
// Usage:
//   root -l 'macros/plot_time_alignment.C("timeAlignment.root")'
//   root -l 'macros/plot_time_alignment.C("timeAlignment.root", "TimeAlignment")'
//
// TTree branch structure (1 entry per channel):
//   Module: u8, Channel: u8
//   BinCenters: vector<double> [ns], Counts: vector<unsigned long>
//   Entries: unsigned long, PeakPosition: double [ns]

#include <TFile.h>
#include <TTree.h>
#include <TH2D.h>
#include <TH1D.h>
#include <TCanvas.h>
#include <TStyle.h>
#include <TLine.h>
#include <TLatex.h>
#include <TColor.h>
#include <iostream>
#include <vector>
#include <algorithm>
#include <map>

void plot_time_alignment(const char* filename = "timeAlignment.root",
                         const char* treename = "TimeAlignment") {
    gStyle->SetOptStat(0);
    gStyle->SetPalette(kBird);

    TFile* f = TFile::Open(filename);
    if (!f || f->IsZombie()) {
        std::cerr << "Error: Cannot open " << filename << std::endl;
        return;
    }

    TTree* tree = dynamic_cast<TTree*>(f->Get(treename));
    if (!tree) {
        std::cerr << "Error: Tree '" << treename << "' not found" << std::endl;
        f->Close();
        return;
    }

    Long64_t nChannels = tree->GetEntries();
    std::cout << "Found " << nChannels << " channels" << std::endl;

    // Read all channel data
    UChar_t module, channel;
    std::vector<double>* binCenters = nullptr;
    std::vector<unsigned long>* counts = nullptr;
    ULong64_t entries;
    Double_t peakPosition;

    tree->SetBranchAddress("Module", &module);
    tree->SetBranchAddress("Channel", &channel);
    tree->SetBranchAddress("BinCenters", &binCenters);
    tree->SetBranchAddress("Counts", &counts);
    tree->SetBranchAddress("Entries", &entries);
    tree->SetBranchAddress("PeakPosition", &peakPosition);

    // First pass: determine global time range and collect channel IDs
    double globalMin = 1e18, globalMax = -1e18;
    std::vector<int> channelIds;  // module*100 + channel
    std::map<int, Long64_t> channelEntryMap;  // channelId -> tree entry index

    for (Long64_t i = 0; i < nChannels; i++) {
        tree->GetEntry(i);
        int chId = static_cast<int>(module) * 100 + static_cast<int>(channel);
        channelIds.push_back(chId);
        channelEntryMap[chId] = i;

        if (binCenters && !binCenters->empty()) {
            double lo = binCenters->front();
            double hi = binCenters->back();
            if (lo < globalMin) globalMin = lo;
            if (hi > globalMax) globalMax = hi;
        }
    }

    std::sort(channelIds.begin(), channelIds.end());

    if (channelIds.empty()) {
        std::cerr << "No channels found" << std::endl;
        f->Close();
        return;
    }

    // Create 2D histogram: X = time difference [ns], Y = channel ID
    // Fixed range: ±1000 ns with 1 ns bin width (2000 bins)
    int nTimeBins = 2000;
    double xMin = -1000.0;
    double xMax = 1000.0;

    TH2D* h2 = new TH2D("h2_time_alignment",
                         "Time Alignment;#Deltat [ns];Channel (Mod*100+Ch);Counts",
                         nTimeBins, xMin, xMax,
                         channelIds.size(), 0, channelIds.size());

    // Set Y-axis bin labels
    for (size_t yi = 0; yi < channelIds.size(); yi++) {
        h2->GetYaxis()->SetBinLabel(yi + 1, Form("%d", channelIds[yi]));
    }

    // Fill 2D histogram
    std::vector<double> peakPositions(channelIds.size());
    std::vector<ULong64_t> channelEntries(channelIds.size());

    for (size_t yi = 0; yi < channelIds.size(); yi++) {
        Long64_t idx = channelEntryMap[channelIds[yi]];
        tree->GetEntry(idx);
        peakPositions[yi] = peakPosition;
        channelEntries[yi] = entries;

        if (binCenters && counts && binCenters->size() == counts->size()) {
            for (size_t bi = 0; bi < binCenters->size(); bi++) {
                if ((*counts)[bi] > 0) {
                    int xBin = h2->GetXaxis()->FindBin((*binCenters)[bi]);
                    if (xBin >= 1 && xBin <= nTimeBins) {
                        h2->SetBinContent(xBin, yi + 1,
                                          h2->GetBinContent(xBin, yi + 1) + (*counts)[bi]);
                    }
                }
            }
        }
    }

    // === Canvas 1: 2D Heatmap ===
    TCanvas* c1 = new TCanvas("c_heatmap", "Time Alignment Heatmap", 1200, 800);
    c1->SetLeftMargin(0.12);
    c1->SetRightMargin(0.14);
    c1->SetLogz();

    h2->GetYaxis()->SetLabelSize(0.02);
    h2->GetYaxis()->SetTickLength(0);
    h2->Draw("colz");

    // Draw peak position markers
    for (size_t yi = 0; yi < channelIds.size(); yi++) {
        if (peakPositions[yi] != 0.0) {
            TLine* line = new TLine(peakPositions[yi], yi, peakPositions[yi], yi + 1);
            line->SetLineColor(kRed);
            line->SetLineWidth(2);
            line->Draw("same");
        }
    }

    c1->Update();

    // === Canvas 2: Individual channel projections (top 12 by entries) ===
    // Sort channels by entries count (descending)
    std::vector<size_t> sortedIdx(channelIds.size());
    for (size_t i = 0; i < sortedIdx.size(); i++) sortedIdx[i] = i;
    std::sort(sortedIdx.begin(), sortedIdx.end(), [&](size_t a, size_t b) {
        return channelEntries[a] > channelEntries[b];
    });

    int nPlots = std::min(static_cast<int>(channelIds.size()), 12);
    int nCols = (nPlots <= 4) ? 2 : ((nPlots <= 6) ? 3 : 4);
    int nRows = (nPlots + nCols - 1) / nCols;

    TCanvas* c2 = new TCanvas("c_projections", "Channel Time Projections", 1600, 300 * nRows);
    c2->Divide(nCols, nRows);

    for (int p = 0; p < nPlots; p++) {
        c2->cd(p + 1);
        size_t yi = sortedIdx[p];
        Long64_t idx = channelEntryMap[channelIds[yi]];
        tree->GetEntry(idx);

        if (!binCenters || binCenters->empty()) continue;

        TH1D* h1 = new TH1D(Form("h1_ch%d", channelIds[yi]),
                             Form("Ch %d (Mod %d, Ch %d) - %llu entries;#Deltat [ns];Counts",
                                  channelIds[yi], channelIds[yi] / 100, channelIds[yi] % 100,
                                  channelEntries[yi]),
                             binCenters->size(),
                             binCenters->front() - 0.5 * ((*binCenters)[1] - (*binCenters)[0]),
                             binCenters->back() + 0.5 * ((*binCenters)[1] - (*binCenters)[0]));

        for (size_t bi = 0; bi < binCenters->size() && bi < counts->size(); bi++) {
            h1->SetBinContent(bi + 1, (*counts)[bi]);
        }

        h1->SetLineColor(kBlue);
        h1->SetFillColor(kBlue - 9);
        h1->Draw();

        // Mark peak position
        if (peakPositions[yi] != 0.0) {
            TLine* pLine = new TLine(peakPositions[yi], 0, peakPositions[yi], h1->GetMaximum());
            pLine->SetLineColor(kRed);
            pLine->SetLineWidth(2);
            pLine->SetLineStyle(2);
            pLine->Draw("same");

            TLatex* label = new TLatex(peakPositions[yi], h1->GetMaximum() * 0.9,
                                       Form("%.1f ns", peakPositions[yi]));
            label->SetTextColor(kRed);
            label->SetTextSize(0.04);
            label->Draw("same");
        }
    }

    c2->Update();

    // Print summary table
    std::cout << "\n=== Time Calibration Summary ===" << std::endl;
    std::cout << Form("%-8s %-10s %-12s %-10s", "ChID", "Offset[ns]", "Entries", "Status") << std::endl;
    std::cout << "---------- ---------- ------------ ----------" << std::endl;

    for (size_t i = 0; i < channelIds.size(); i++) {
        size_t yi = sortedIdx[i];
        const char* status = (peakPositions[yi] != 0.0) ? "OK" : "NO_PEAK";
        std::cout << Form("%-8d %+10.2f %12llu %-10s",
                         channelIds[yi], peakPositions[yi], channelEntries[yi], status) << std::endl;
    }

    std::cout << "\nPlots created. Use ROOT interactive mode to explore." << std::endl;
}

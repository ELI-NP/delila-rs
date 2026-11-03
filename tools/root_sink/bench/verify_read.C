// verify_read.C — read-back sanity checks for the benchmark outputs.
//   root -l -b -q 'verify_read.C("hits_ttree.root","hits_rntuple.root")'
// 1. TTree::Draw on the TTree file (classic analysis path)
// 2. RDataFrame on the RNTuple (the only "Draw"-like path for RNTuple)
// 3. RDataFrame Snapshot RNTuple -> TTree (escape hatch for TTree-only macros)
// 4. RNTupleReader::PrintInfo (format version as seen by this ROOT)
#include "ROOT/RDataFrame.hxx"
#include "ROOT/RNTupleReader.hxx"
#include "TFile.h"
#include "TH1.h"
#include "TStopwatch.h"
#include "TTree.h"

#include <cstdio>

void verify_read(const char* ttree_file, const char* rntuple_file) {
  TStopwatch sw;
  {
    TFile f(ttree_file);
    TTree* t = f.Get<TTree>("delila");
    sw.Start();
    const Long64_t n = t->Draw("energy>>h(1000,0,16000)", "module==3", "goff");
    sw.Stop();
    std::printf("[1] TTree::Draw: %lld selected of %lld in %.2f s\n", n, t->GetEntries(), sw.RealTime());
  }
  {
    ROOT::RDataFrame df("delila", rntuple_file);
    sw.Start();
    auto h = df.Filter("module==3").Histo1D({"h", "", 1000, 0, 16000}, "energy");
    std::printf("[2] RDataFrame(RNTuple): %.0f entries in hist, %.2f s\n", h->GetEntries(), sw.RealTime());
    sw.Stop();
  }
  {
    ROOT::RDataFrame df("delila", rntuple_file);
    sw.Start();
    auto out = df.Snapshot("delila", "rntuple_as_ttree.root");
    sw.Stop();
    TFile f("rntuple_as_ttree.root");
    TTree* t = f.Get<TTree>("delila");
    std::printf("[3] Snapshot RNTuple->TTree: %lld entries, %.2f s; TTree::Draw works: %lld\n", t->GetEntries(),
                sw.RealTime(), t->Draw("timestamp_ns", "", "goff", 1000));
  }
  {
    auto r = ROOT::RNTupleReader::Open("delila", rntuple_file);
    r->PrintInfo(ROOT::ENTupleInfo::kSummary);
    std::printf("[4] entries = %llu\n", (unsigned long long)r->GetNEntries());
  }
}

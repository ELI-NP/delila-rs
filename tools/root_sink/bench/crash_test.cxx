// crash_test.cxx — what survives a process crash in the middle of a run?
//   ./crash_test write-ttree   F  N   : TTree, AutoSave("SaveSelf") every 1M entries, then _exit(1)
//   ./crash_test write-rntuple F  N   : RNTupleWriter, CommitCluster(true) every 1M entries, then _exit(1)
//   ./crash_test write-rntuple-nocommit F N : RNTupleWriter, no explicit commit, then _exit(1)
//   ./crash_test read-ttree    F      : TFile::Open + GetEntries (TFile::Recover path)
//   ./crash_test read-rntuple  F      : RNTupleReader::Open + GetNEntries
// Build: g++ -O2 -std=c++17 crash_test.cxx $(root-config --cflags --libs) -lROOTNTuple -o crash_test
#include <unistd.h>

#include <cstdio>
#include <cstdlib>
#include <string>

#include "ROOT/RNTupleModel.hxx"
#include "ROOT/RNTupleReader.hxx"
#include "ROOT/RNTupleWriter.hxx"
#include "TFile.h"
#include "TTree.h"

#include "hits_gen.hpp"

int main(int argc, char** argv) {
  if (argc < 3) { std::fprintf(stderr, "usage: see header\n"); return 2; }
  const std::string mode = argv[1], file = argv[2];
  if (mode == "read-ttree") {
    TFile* f = TFile::Open(file.c_str());
    if (!f || f->IsZombie()) { std::printf("read-ttree: cannot open (zombie)\n"); return 1; }
    TTree* t = f->Get<TTree>("delila");
    std::printf("read-ttree: tree %s, entries = %lld\n", t ? "found" : "MISSING", t ? t->GetEntries() : -1LL);
    return 0;
  }
  if (mode == "read-rntuple") {
    try {
      auto r = ROOT::RNTupleReader::Open("delila", file);
      std::printf("read-rntuple: entries = %llu\n", (unsigned long long)r->GetNEntries());
    } catch (const std::exception& e) {
      std::printf("read-rntuple: FAILED: %s\n", e.what());
      return 1;
    }
    return 0;
  }
  GenConfig cfg;
  cfg.n = argc > 3 ? std::stoull(argv[3]) : 5'000'000;
  auto hits = generate_hits(cfg);
  const std::size_t every = 1'000'000;
  if (mode == "write-ttree") {
    TFile* f = TFile::Open(file.c_str(), "RECREATE", "", 505);
    TTree* t = new TTree("delila", "hits");
    UChar_t m, c; UShort_t e, es; Double_t ts;
    t->Branch("module", &m, "module/b"); t->Branch("channel", &c, "channel/b");
    t->Branch("energy", &e, "energy/s"); t->Branch("energy_short", &es, "energy_short/s");
    t->Branch("timestamp_ns", &ts, "timestamp_ns/D");
    for (std::size_t i = 0; i < hits.size(); ++i) {
      m = hits[i].module; c = hits[i].channel; e = hits[i].energy; es = hits[i].energy_short; ts = hits[i].timestamp_ns;
      t->Fill();
      if ((i + 1) % every == 0) t->AutoSave("SaveSelf");
    }
    std::printf("write-ttree: filled %zu, last AutoSave at %zu, crashing now\n", hits.size(), hits.size() / every * every);
    std::fflush(stdout);
    _exit(1);
  }
  if (mode == "write-rntuple" || mode == "write-rntuple-nocommit") {
    auto model = ROOT::RNTupleModel::Create();
    auto pm = model->MakeField<std::uint8_t>("module");
    auto pc = model->MakeField<std::uint8_t>("channel");
    auto pe = model->MakeField<std::uint16_t>("energy");
    auto pes = model->MakeField<std::uint16_t>("energy_short");
    auto pt = model->MakeField<double>("timestamp_ns");
    ROOT::RNTupleWriteOptions opts;
    opts.SetCompression(505);
    auto w = ROOT::RNTupleWriter::Recreate(std::move(model), "delila", file, opts);
    for (std::size_t i = 0; i < hits.size(); ++i) {
      *pm = hits[i].module; *pc = hits[i].channel; *pe = hits[i].energy; *pes = hits[i].energy_short;
      *pt = hits[i].timestamp_ns;
      w->Fill();
      if (mode == "write-rntuple" && (i + 1) % every == 0) w->CommitCluster(true);
    }
    std::printf("%s: filled %zu, crashing now\n", mode.c_str(), hits.size());
    std::fflush(stdout);
    _exit(1);
  }
  std::fprintf(stderr, "unknown mode\n");
  return 2;
}
